// Private inherited-pipe controller. No TCP server, named-pipe listener or
// arbitrary address/function RPC. Only the pinned bridge's two exports run.
#define WIN32_LEAN_AND_MEAN
#include <windows.h>
#include <tlhelp32.h>
#include <sddl.h>
#include <bcrypt.h>
#include <cstdio>
#include <cstring>
#include <string>
#include <vector>
#include <algorithm>
#include "protocol.h"
#include "compatibility.h"
#ifndef WERC_BRIDGE_SHA256
#error WERC_BRIDGE_SHA256 must be set by the verified source build
#endif
struct Handle {
    HANDLE value=nullptr;
    ~Handle(){ if(value && value!=INVALID_HANDLE_VALUE) CloseHandle(value); }
    operator HANDLE() const {return value;}
};
static uint64_t start_time(HANDLE process) {
    FILETIME a{},b{},c{},d{};
    if(!GetProcessTimes(process,&a,&b,&c,&d)) return 0;
    return uint64_t(a.dwHighDateTime)<<32 | a.dwLowDateTime;
}
static bool transfer(HANDLE h,void* bytes,DWORD size,bool write) {
    DWORD total=0;
    while(total<size) {
        DWORD done=0;
        const BOOL ok=write ? WriteFile(h,static_cast<char*>(bytes)+total,size-total,&done,nullptr)
                            : ReadFile(h,static_cast<char*>(bytes)+total,size-total,&done,nullptr);
        if(!ok || !done) return false;
        total+=done;
    }
    return true;
}
static std::vector<unsigned char> user_token(HANDLE process) {
    Handle token;
    if(!OpenProcessToken(process,TOKEN_QUERY,&token.value)) return {};
    DWORD bytes=0;GetTokenInformation(token,TokenUser,nullptr,0,&bytes);
    if(!bytes || bytes>65536) return {};
    std::vector<unsigned char> result(bytes);
    if(!GetTokenInformation(token,TokenUser,result.data(),bytes,&bytes)) return {};
    return result;
}
static bool hash_file(HANDLE file,const char* expected, CoreVariant* variant=nullptr) {
    LARGE_INTEGER size{};
    if(!GetFileSizeEx(file,&size) || size.QuadPart<=0 || size.QuadPart>256*1024*1024) return false;
    LARGE_INTEGER zero{};if(!SetFilePointerEx(file,zero,nullptr,FILE_BEGIN)) return false;
    BCRYPT_ALG_HANDLE alg=nullptr;BCRYPT_HASH_HANDLE hash=nullptr;
    if(BCryptOpenAlgorithmProvider(&alg,BCRYPT_SHA256_ALGORITHM,nullptr,0)<0) return false;
    bool good=BCryptCreateHash(alg,&hash,nullptr,0,nullptr,0,0)>=0;
    unsigned char buffer[65536],digest[32]{};
    DWORD count=0;
    while(good) {
        if(!ReadFile(file,buffer,sizeof(buffer),&count,nullptr)) {good=false;break;}
        if(!count) break;
        good=BCryptHashData(hash,buffer,count,0)>=0;
    }
    if(good) good=BCryptFinishHash(hash,digest,sizeof(digest),0)>=0;
    if(hash) BCryptDestroyHash(hash);BCryptCloseAlgorithmProvider(alg,0);
    char actual[65]{};
    for(size_t i=0;i<32;++i) sprintf_s(actual+i*2,3,"%02x",digest[i]);
    if(variant) *variant=good ? core_variant(actual) : CoreVariant::Unsupported;
    return good && (variant ? *variant!=CoreVariant::Unsupported : !std::strcmp(actual,expected));
}
static bool module(DWORD pid,const wchar_t* name,MODULEENTRY32W& result) {
    Handle snapshot{CreateToolhelp32Snapshot(TH32CS_SNAPMODULE|TH32CS_SNAPMODULE32,pid)};
    if(snapshot.value==INVALID_HANDLE_VALUE) return false;
    MODULEENTRY32W entry{};entry.dwSize=sizeof(entry);
    for(BOOL ok=Module32FirstW(snapshot,&entry);ok;ok=Module32NextW(snapshot,&entry)) {
        if(!_wcsicmp(entry.szModule,name)) {result=entry;return true;}
    }
    return false;
}
static uintptr_t system_function(DWORD pid,const char* name) {
    auto function=GetProcAddress(GetModuleHandleW(L"kernel32.dll"),name);
    HMODULE owner=nullptr;
    if(!function || !GetModuleHandleExW(GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS|GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
        reinterpret_cast<LPCWSTR>(function),&owner)) return 0;
    wchar_t path[32768]{};if(!GetModuleFileNameW(owner,path,32768)) return 0;
    const wchar_t* basename=wcsrchr(path,L'\\');if(!basename) return 0;
    MODULEENTRY32W remote{};
    if(!module(pid,basename+1,remote) || _wcsicmp(remote.szExePath,path)) return 0;
    return reinterpret_cast<uintptr_t>(remote.modBaseAddr)+reinterpret_cast<uintptr_t>(function)-reinterpret_cast<uintptr_t>(owner);
}
static bool remote_call(HANDLE process,uintptr_t function,void* argument,DWORD& code) {
    if(!function) return false;
    Handle thread{CreateRemoteThread(process,nullptr,0,reinterpret_cast<LPTHREAD_START_ROUTINE>(function),argument,0,nullptr)};
    if(!thread.value) return false;
    if(WaitForSingleObject(thread,15000)!=WAIT_OBJECT_0) return false;
    return GetExitCodeThread(thread,&code)!=FALSE;
}
static uintptr_t export_rva(HMODULE image,const char* name) {
    // DONT_RESOLVE_DLL_REFERENCES maps our pinned image without DllMain.
    const auto fn=GetProcAddress(image,name);
    return fn ? reinterpret_cast<uintptr_t>(fn)-reinterpret_cast<uintptr_t>(image) : 0;
}
struct Session {
    Handle process,bridge_file,core_file,mutex;
    DWORD pid=0;uint64_t started=0;
    uintptr_t base=0,call=0,join=0;
    void* memory=nullptr;bool attached=false,uncertain=false,retained=false;
    WercPacket binding{};
    bool invoke(WercPacket& packet) {
        SIZE_T transferred=0;DWORD code=0;
        if(!WriteProcessMemory(process,memory,&packet,sizeof(packet),&transferred) || transferred!=sizeof(packet)) return false;
        if(!remote_call(process,call,memory,code)) {uncertain=true;return false;}
        if(!ReadProcessMemory(process,memory,&packet,sizeof(packet),&transferred) || transferred!=sizeof(packet)) return false;
        return code==packet.status;
    }
    DWORD open(WercPacket& p) {
        if(!p.pid || p.pid==GetCurrentProcessId()) return ERROR_INVALID_PARAMETER;
        pid=p.pid;
        process.value=OpenProcess(PROCESS_CREATE_THREAD|PROCESS_QUERY_LIMITED_INFORMATION|PROCESS_VM_OPERATION|PROCESS_VM_READ|PROCESS_VM_WRITE|SYNCHRONIZE,FALSE,pid);
        if(!process.value) return ERROR_ACCESS_DENIED;
        started=start_time(process);
        if(!started || (p.process_start && p.process_start!=started)) return ERROR_INVALID_PARAMETER;
        auto mine=user_token(GetCurrentProcess()), theirs=user_token(process);
        if(mine.empty() || theirs.empty() || !EqualSid(reinterpret_cast<TOKEN_USER*>(mine.data())->User.Sid,
                                                       reinterpret_cast<TOKEN_USER*>(theirs.data())->User.Sid)) return ERROR_ACCESS_DENIED;
        DWORD session1=0,session2=0;
        if(!ProcessIdToSessionId(pid,&session1) || !ProcessIdToSessionId(GetCurrentProcessId(),&session2) || session1!=session2) return ERROR_ACCESS_DENIED;
        wchar_t exe[32768]{};DWORD exe_size=32768;
        if(!QueryFullProcessImageNameW(process,0,exe,&exe_size)) return ERROR_ACCESS_DENIED;
        const auto filename=wcsrchr(exe,L'\\');if(!filename || _wcsicmp(filename+1,L"Weixin.exe")) return ERROR_INVALID_NAME;
        MODULEENTRY32W core{};
        if(!module(pid,L"Weixin.dll",core)) return ERROR_MOD_NOT_FOUND;
        core_file.value=CreateFileW(core.szExePath,GENERIC_READ,FILE_SHARE_READ,nullptr,OPEN_EXISTING,FILE_ATTRIBUTE_NORMAL,nullptr);
        if(core_file.value==INVALID_HANDLE_VALUE) return ERROR_ACCESS_DENIED;
        CoreVariant variant=CoreVariant::Unsupported;
        if(!hash_file(core_file,nullptr,&variant)) return ERROR_REVISION_MISMATCH;
        if(!core_install_layout(exe,core.szExePath,variant)) return ERROR_INVALID_NAME;
        // Refuse pre-existing hooks at the two entries we depend on. File
        // offsets use this exact image's .text mapping (RVA - 0xc00).
        const std::vector<uint32_t> legacy={0x17b9010u,0x54b02f0u,0x6be0020u,0x47220u,0x36c740u,0x6f4d90u,0x178f100u,0x3981a30u,0xa58dd0u,0x1aee40u,0x2227b80u,0x43a5190u,0x1067800u,0x2ffe3b0u,0x4945b0u,0x4944b0u,0x494920u,0x19cb900u,0x2fd2b0u,0x74ac908u,0x74ac930u,0x2227fa0u,0xee95f0u,0x90b60u,0x22d8390u,0x389830u,0x57fe90u,0x9f4b0u};
        const std::vector<uint32_t> profile52={0x45bc630,0x1e80be0,0x81580,0x89610,0x98070,0x2e5380,0x372f50,0x469940,0x469a40,0x469db0,0x57fa90,0xea5120,0x10137f0,0x1969bd0,0x21f2760,0x21f2db0,0x231f7d0,0x2a28000,0x3047b80,0x306ae60,0x306c360,0x306c830,0x306eee0,0x3071b40,0x43b98e0,0x447f7a0,0x5858bb0,0x776c598,0x776d1b0,0x776d1b8,0x776d1d8,0x7800,0x211a0,0x7e250,0x81580,0x87860,0x94090,0x98070,0x10c850,0x1b1c40,0x1c7fe0,0x33f1b0,0x355620,0x372f50,0x4e5f30,0x5305c0,0x57fa90,0x702c50,0x702d70,0xa92190,0xe16990,0xe34b70,0xe59720,0x1762670,0x179c8f0,0x19ecd60,0x1b5acc0,0x231f7d0,0x2a28000,0x2c29820,0x2c2be20,0x2c2f850,0x2faad30,0x39d6980,0x3e0a690,0x54979c0,0x56fb025,0x6ec0fb0,0x776c55c,0x776c5a0,0x702c50,0x1762670,0x39a6260,0xa92190,0x1b1c40,0x178d380,0x56fab30,0x776c55c,0x776c5a0,0x1fc90,0x20920,0x211a0,0x21290,0x2b3a0,0x358810,0x2e3340,0x9c4700,0x9c4850,0x9c4460,0x9c45b0,0x9c82e0,0x9c81a0};
        const std::vector<uint32_t> profile53={0x45ba040,0x1e7f8b0,0x81580,0x89610,0x98070,0x2e5380,0x372f50,0x468fa0,0x4690a0,0x469410,0x57f0f0,0xea4780,0x1012800,0x19680f0,0x21f1430,0x21f1a80,0x231e4a0,0x2a26b50,0x3046440,0x3069720,0x306ac20,0x306b0f0,0x306d7a0,0x3070400,0x43b72f0,0x447d1b0,0x58565b0,0x7769358,0x7769f70,0x7769f78,0x7769f98,0x7800,0x211a0,0x7e250,0x81580,0x87860,0x94090,0x98070,0x10c850,0x1b1c40,0x1c7fe0,0x33f1b0,0x355620,0x372f50,0x4e5590,0x52fc20,0x57f0f0,0x7022b0,0x7023d0,0xa917f0,0xe15ff0,0xe341d0,0xe58d80,0x1760b80,0x179ae00,0x19eba30,0x1b59990,0x231e4a0,0x2a26b50,0x2c280d0,0x2c2a6d0,0x2c2e100,0x2fa95e0,0x39d5240,0x3e080a0,0x54953e0,0x56f8a45,0x6ebdfb0,0x776931c,0x7769360,0x7022b0,0x1760b80,0x39a4b20,0xa917f0,0x1b1c40,0x178b890,0x56f8550,0x776931c,0x7769360,0x1fc90,0x20920,0x211a0,0x21290,0x2b3a0,0x358810,0x2e3340,0x9c3d60,0x9c3eb0,0x9c3ac0,0x9c3c10,0x9c7940,0x9c7800};
        struct DiskEntry { uint32_t rva,offset; };
        const DiskEntry profile41137[]={{0x417e0,0x40be0},{0x33f850,0x33ec50},{0x40b70,0x3ff70},{0x42060,0x41460},
            {0x42150,0x41550},{0x4c250,0x4b650},{0x2cb8a0,0x2caca0},{0x1796560,0x1795960}};
        if(variant==CoreVariant::WeChat41137 || variant==CoreVariant::WeChat411310) {
            for(size_t di=0;di<sizeof(profile41137)/sizeof(profile41137[0]);++di) {
                const auto& entry=profile41137[di];
                LARGE_INTEGER offset{};offset.QuadPart=entry.offset;
                unsigned char disk[32]{},live[32]{};DWORD n=0;SIZE_T got=0;
                if(!SetFilePointerEx(core_file,offset,nullptr,FILE_BEGIN) || !ReadFile(core_file,disk,32,&n,nullptr) || n!=32 ||
                   !ReadProcessMemory(process,core.modBaseAddr+entry.rva,live,32,&got) || got!=32 || std::memcmp(disk,live,32)) return ERROR_INVALID_DATA;
            }
        } else {
            for(auto rva:variant==CoreVariant::WeChat42053 ? profile53 : variant==CoreVariant::WeChat42052 ? profile52 : legacy) {
                LARGE_INTEGER offset{};offset.QuadPart=rva-0xc00;
                unsigned char disk[32]{},live[32]{};DWORD n=0;SIZE_T got=0;
                if(!SetFilePointerEx(core_file,offset,nullptr,FILE_BEGIN) || !ReadFile(core_file,disk,32,&n,nullptr) || n!=32 ||
                   !ReadProcessMemory(process,core.modBaseAddr+rva,live,32,&got) || got!=32 || std::memcmp(disk,live,32)) return ERROR_INVALID_DATA;
            }
        }
        wchar_t host[32768]{};if(!GetModuleFileNameW(nullptr,host,32768)) return ERROR_PATH_NOT_FOUND;
        wchar_t* slash=wcsrchr(host,L'\\');if(!slash) return ERROR_PATH_NOT_FOUND;*slash=0;
        const std::wstring dll=std::wstring(host)+L"\\werc-message-bridge.dll";
        bridge_file.value=CreateFileW(dll.c_str(),GENERIC_READ,FILE_SHARE_READ,nullptr,OPEN_EXISTING,FILE_ATTRIBUTE_NORMAL,nullptr);
        if(bridge_file.value==INVALID_HANDLE_VALUE || !hash_file(bridge_file,WERC_BRIDGE_SHA256)) return ERROR_INVALID_IMAGE_HASH;
        LPWSTR sid=nullptr;PSECURITY_DESCRIPTOR sd=nullptr;
        if(!ConvertSidToStringSidW(reinterpret_cast<TOKEN_USER*>(mine.data())->User.Sid,&sid)) return ERROR_ACCESS_DENIED;
        const std::wstring sddl=L"D:P(A;;GA;;;"+std::wstring(sid)+L")";LocalFree(sid);
        if(!ConvertStringSecurityDescriptorToSecurityDescriptorW(sddl.c_str(),SDDL_REVISION_1,&sd,nullptr)) return ERROR_ACCESS_DENIED;
        SECURITY_ATTRIBUTES sa{sizeof(sa),sd,FALSE};
        const auto name=L"Local\\WeRC.Native."+std::to_wstring(pid)+L"."+std::to_wstring(started);
        mutex.value=CreateMutexW(&sa,FALSE,name.c_str());const DWORD error=GetLastError();LocalFree(sd);
        if(!mutex.value || error==ERROR_ALREADY_EXISTS) return ERROR_BUSY;
        MODULEENTRY32W prior{};const bool existing=module(pid,L"werc-message-bridge.dll",prior);
        if(existing && _wcsicmp(prior.szExePath,dll.c_str())) return ERROR_ALREADY_EXISTS;
        auto image=LoadLibraryExW(dll.c_str(),nullptr,DONT_RESOLVE_DLL_REFERENCES);
        if(!image) return ERROR_BAD_EXE_FORMAT;
        const auto call_offset=export_rva(image,"WercCall"),join_offset=export_rva(image,"WercJoin");FreeLibrary(image);
        if(!call_offset || !join_offset) return ERROR_PROC_NOT_FOUND;
        if(existing) {
            base=reinterpret_cast<uintptr_t>(prior.modBaseAddr);call=base+call_offset;join=base+join_offset;retained=true;
            memory=VirtualAllocEx(process,nullptr,sizeof(WercPacket),MEM_COMMIT|MEM_RESERVE,PAGE_READWRITE);
            if(!memory) return ERROR_NOT_ENOUGH_MEMORY;
            WercPacket recovery{};recovery.size=sizeof(recovery);recovery.version=WercProtocolVersion;
            recovery.operation=uint32_t(WercOp::Recover);recovery.pid=pid;recovery.process_start=started;
            if(!invoke(recovery)) {uncertain=true;return ERROR_TIMEOUT;}
            if(recovery.status) return recovery.status;
            p.process_start=started;p.controller_pid=GetCurrentProcessId();p.controller_start=start_time(GetCurrentProcess());
            return 0;
        }
        const SIZE_T bytes=(dll.size()+1)*sizeof(wchar_t);
        void* path=VirtualAllocEx(process,nullptr,bytes,MEM_COMMIT|MEM_RESERVE,PAGE_READWRITE);
        SIZE_T written=0;DWORD code=0;
        if(!path) return ERROR_NOT_ENOUGH_MEMORY;
        if(!WriteProcessMemory(process,path,dll.c_str(),bytes,&written) || written!=bytes) {VirtualFreeEx(process,path,0,MEM_RELEASE);return ERROR_WRITE_FAULT;}
        if(!remote_call(process,system_function(pid,"LoadLibraryW"),path,code)) {uncertain=true;return ERROR_TIMEOUT;}
        VirtualFreeEx(process,path,0,MEM_RELEASE);
        MODULEENTRY32W loaded{};
        if(!module(pid,L"werc-message-bridge.dll",loaded) || _wcsicmp(loaded.szExePath,dll.c_str())) return ERROR_DLL_INIT_FAILED;
        base=reinterpret_cast<uintptr_t>(loaded.modBaseAddr);call=base+call_offset;join=base+join_offset;
        memory=VirtualAllocEx(process,nullptr,sizeof(WercPacket),MEM_COMMIT|MEM_RESERVE,PAGE_READWRITE);
        if(!memory) return ERROR_NOT_ENOUGH_MEMORY;
        p.process_start=started;p.controller_pid=GetCurrentProcessId();p.controller_start=start_time(GetCurrentProcess());
        return 0;
    }
    ~Session() {
        if(!base || !process.value || uncertain) return;
        bool safe=true;
        if(attached) {
            WercPacket close=binding;close.operation=uint32_t(WercOp::Close);
            safe=invoke(close) && !close.status;
            DWORD code=1;
            if(safe) safe=remote_call(process,join,nullptr,code) && !code;
        }
        if(safe) {
            if(memory) {WercPacket zero{};SIZE_T n=0;WriteProcessMemory(process,memory,&zero,sizeof(zero),&n);VirtualFreeEx(process,memory,0,MEM_RELEASE);}
            if(!retained) {DWORD code=0;remote_call(process,system_function(pid,"FreeLibrary"),reinterpret_cast<void*>(base),code);}
        }
        SecureZeroMemory(&binding,sizeof(binding));
    }
};
int main() {
    // Only a parent-created anonymous pipe session is supported.
    const auto input=GetStdHandle(STD_INPUT_HANDLE),output=GetStdHandle(STD_OUTPUT_HANDLE);
    if(GetFileType(input)!=FILE_TYPE_PIPE || GetFileType(output)!=FILE_TYPE_PIPE) return 2;
    Session session;
    WercPacket packet{};
    bool opened=false;
    while(transfer(input,&packet,sizeof(packet),false)) {
        DWORD error=0;
        if(packet.size!=sizeof(packet) || packet.version!=WercProtocolVersion || packet.reserved || packet.operation<1 || packet.operation>71 || packet.operation==6 || packet.operation==8)
            error=ERROR_REVISION_MISMATCH;
        else if(!opened) {
            if(packet.operation!=uint32_t(WercOp::Inspect)) error=ERROR_INVALID_FUNCTION;
            else {error=session.open(packet);opened=error==0;}
        }
        if(!error && opened) {
            if(packet.pid!=session.pid || packet.process_start!=session.started) error=ERROR_INVALID_PARAMETER;
            else if(!session.invoke(packet)) {error=ERROR_TIMEOUT;session.uncertain=true;}
            else error=packet.status;
            if(packet.operation==uint32_t(WercOp::Attach) && !error) {session.attached=true;session.binding=packet;}
        }
        packet.status=error;
        if(!transfer(output,&packet,sizeof(packet),true)) break;
        SecureZeroMemory(&packet,sizeof(packet));
        if(error && !opened) break;
        if(session.uncertain) break;
    }
    SecureZeroMemory(&packet,sizeof(packet));
    return session.uncertain ? 3 : 0;
}



