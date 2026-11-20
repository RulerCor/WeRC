// Noncommercial helper-derived image constructor/dispatch, exact profile 1 only.
// Caller supplies bounded canonical PNG bytes, never an arbitrary filesystem path.
#include <sddl.h>
#include <string>
static constexpr size_t image_limit=8*1024*1024;
struct ImageStage {
    WercPacket packet{};std::vector<unsigned char> bytes;
    std::wstring directory,path;HANDLE pin=INVALID_HANDLE_VALUE;bool owns_directory=false,owns_file=false;
    ~ImageStage(){
        if(pin!=INVALID_HANDLE_VALUE)CloseHandle(pin);
        if(owns_file)DeleteFileW(path.c_str());
        if(owns_directory)RemoveDirectoryW(directory.c_str());
        if(!bytes.empty())SecureZeroMemory(bytes.data(),bytes.size());SecureZeroMemory(&packet,sizeof(packet));
    }
};
static std::unique_ptr<ImageStage> image_stage;
static uint32_t image_be32(const unsigned char* p){return uint32_t(p[0])<<24|uint32_t(p[1])<<16|uint32_t(p[2])<<8|p[3];}
static bool image_shape(const std::vector<unsigned char>& bytes,uint32_t dimensions){
    static constexpr unsigned char header[]={137,80,78,71,13,10,26,10,0,0,0,13,'I','H','D','R'};
    static constexpr unsigned char end[]={0,0,0,0,'I','E','N','D',174,66,96,130};
    if(bytes.size()<57 || bytes.size()>image_limit || std::memcmp(bytes.data(),header,sizeof(header)) ||
       std::memcmp(bytes.data()+bytes.size()-12,end,12))return false;
    const uint32_t w=image_be32(bytes.data()+16),h=image_be32(bytes.data()+20);
    return w && h && w<=4096 && h<=4096 && uint64_t(w)*h<=4194304 &&
        dimensions==(w|(h<<16)) && bytes[24]==8 && bytes[25]==6 && !bytes[26] && !bytes[27] && !bytes[28];
}
// Only local absolute paths, no streams, relative traversal, or reparse ancestors.
static bool image_local_path(const wchar_t* path,bool input_temp){
    const size_t n=wcsnlen_s(path,1024);
    if(n<4 || n>=1024 || !((path[0]>=L'A'&&path[0]<=L'Z')||(path[0]>=L'a'&&path[0]<=L'z')) || path[1]!=L':' || path[2]!=L'\\')return false;
    bool saw_temp=false;
    for(size_t i=3,start=3;i<=n;++i){
        if(i<n && (path[i]==L':' || path[i]==L'/' || path[i]==L'*' || path[i]==L'?' || path[i]<32))return false;
        if(i==n || path[i]==L'\\'){
            if(i==start || (i-start==1&&path[start]==L'.') || (i-start==2&&path[start]==L'.'&&path[start+1]==L'.') || path[i-1]==L'.' || path[i-1]==L' ')return false;
            if(i-start==9 && !_wcsnicmp(path+start,L"InputTemp",9))saw_temp=true;
            if(i<n){wchar_t part[1024]{};std::memcpy(part,path,i*sizeof(wchar_t));
                const DWORD attributes=GetFileAttributesW(part);
                if(attributes==INVALID_FILE_ATTRIBUTES || !(attributes&FILE_ATTRIBUTE_DIRECTORY) || (attributes&FILE_ATTRIBUTE_REPARSE_POINT))return false;
            }
            start=i+1;
        }
    }
    return !input_temp || saw_temp;
}
static bool image_stage_file(ImageStage& stage){
    wchar_t temp[768]{};const DWORD count=GetTempPathW(768,temp);
    if(!count || count>=768)return false;
    unsigned char random[16]{};if(BCryptGenRandom(nullptr,random,sizeof(random),BCRYPT_USE_SYSTEM_PREFERRED_RNG)<0)return false;
    static const wchar_t alphabet[]=L"0123456789abcdef";wchar_t id[33]{};
    for(size_t i=0;i<16;++i){id[i*2]=alphabet[random[i]>>4];id[i*2+1]=alphabet[random[i]&15];}
    stage.directory=std::wstring(temp)+L"werc-image-stage-"+id;stage.path=stage.directory+L"\\image.png";
    if(!image_local_path(stage.directory.c_str(),false))return false;
    HANDLE token=nullptr;unsigned char storage[1024]{};DWORD needed=0;LPWSTR sid=nullptr;PSECURITY_DESCRIPTOR descriptor=nullptr;
    bool ok=OpenProcessToken(GetCurrentProcess(),TOKEN_QUERY,&token)!=FALSE;
    if(ok)ok=GetTokenInformation(token,TokenUser,storage,sizeof(storage),&needed)!=FALSE;
    if(ok)ok=ConvertSidToStringSidW(reinterpret_cast<TOKEN_USER*>(storage)->User.Sid,&sid)!=FALSE;
    if(token)CloseHandle(token);
    if(ok){const auto acl=L"D:P(A;OICI;FA;;;SY)(A;OICI;FA;;;"+std::wstring(sid)+L")";
        ok=ConvertStringSecurityDescriptorToSecurityDescriptorW(acl.c_str(),SDDL_REVISION_1,&descriptor,nullptr)!=FALSE;}
    if(sid)LocalFree(sid);
    SECURITY_ATTRIBUTES sa{sizeof(sa),descriptor,FALSE};
    if(ok)ok=CreateDirectoryW(stage.directory.c_str(),&sa)!=FALSE;
    if(descriptor)LocalFree(descriptor);
    if(!ok)return false;stage.owns_directory=true;
    HANDLE file=CreateFileW(stage.path.c_str(),GENERIC_WRITE,0,nullptr,CREATE_NEW,FILE_ATTRIBUTE_TEMPORARY,nullptr);
    if(file==INVALID_HANDLE_VALUE)return false;stage.owns_file=true;
    DWORD written=0;ok=WriteFile(file,stage.bytes.data(),static_cast<DWORD>(stage.bytes.size()),&written,nullptr)!=FALSE && written==stage.bytes.size() && FlushFileBuffers(file);
    CloseHandle(file);if(!ok)return false;
    stage.pin=CreateFileW(stage.path.c_str(),GENERIC_READ,FILE_SHARE_READ,nullptr,OPEN_EXISTING,FILE_FLAG_OPEN_REPARSE_POINT,nullptr);
    if(stage.pin==INVALID_HANDLE_VALUE)return false;
    BY_HANDLE_FILE_INFORMATION info{};LARGE_INTEGER length{};
    if(!GetFileInformationByHandle(stage.pin,&info) || (info.dwFileAttributes&(FILE_ATTRIBUTE_REPARSE_POINT|FILE_ATTRIBUTE_DIRECTORY)) ||
       !GetFileSizeEx(stage.pin,&length) || length.QuadPart!=static_cast<LONGLONG>(stage.bytes.size()))return false;
    std::vector<unsigned char> actual(stage.bytes.size());DWORD read=0;
    ok=ReadFile(stage.pin,actual.data(),static_cast<DWORD>(actual.size()),&read,nullptr)!=FALSE && read==actual.size() && actual==stage.bytes;
    SecureZeroMemory(actual.data(),actual.size());return ok;
}
struct ImageLease {
    std::unique_ptr<ImageStage> stage;size_t index;bool probe;
    ImageLease(std::unique_ptr<ImageStage> value,size_t slot,bool inspect):stage(std::move(value)),index(slot),probe(inspect){_InterlockedIncrement(&live_tasks);}
    ~ImageLease(){stage.reset();_InterlockedDecrement(&live_tasks);}
};
struct ImageQtString {int32_t references,length;uint32_t allocation,padding;int64_t offset;wchar_t text[1024];};
static_assert(offsetof(ImageQtString,text)==24,"Qt static string header");
struct ImageWideString {wchar_t data[8];uint64_t length,capacity;};
static bool image_wide_equals(const ImageWideString* value,const wchar_t* expected){
    const size_t length=wcsnlen_s(expected,1024);
    if(!length || length>=1024 || value->length!=length || value->capacity<value->length)return false;
    const wchar_t* chars=value->capacity>7?*reinterpret_cast<const wchar_t*const*>(value->data):value->data;
    return chars && !std::memcmp(chars,expected,(length+1)*sizeof(wchar_t));
}
static void image_free_string(SmallString* s,TestResult* report){
    __try {if(s->capacity>15 && s->capacity<1024*1024){
        void* memory=*reinterpret_cast<void**>(s->data);size_t size=s->capacity+1;
        if(size>=4096){memory=*(reinterpret_cast<void**>(memory)-1);size+=39;}
        using Free=void(__fastcall*)(void*,size_t);reinterpret_cast<Free>(core_address(core_base,0x74ab8e0))(memory,size);
    }} __except(record_exception(GetExceptionInformation(),report)){}
}
static void image_on_task(const ImageLease* lease){
    const auto& p=lease->stage->packet;
    if(!lease->probe && ensure_text_task_result()){receipt_set(lease->index,4);return;}
    if(closing || uint64_t(std::time(nullptr))>=p.deadline || WaitForSingleObject(controller,0)!=WAIT_TIMEOUT || (!same_account(p.account) || !target_allowed(p))){receipt_set(lease->index,4);return;}
    SharedMessage app{},context{},manager{},message{};SmallString roundtrip{};roundtrip.capacity=15;
    alignas(16) unsigned char result[128]{};TestResult report{};
    ImageQtString path{};path.references=-1;path.length=static_cast<int32_t>(lease->stage->path.size());path.offset=24;
    if(path.length>=1024){receipt_set(lease->index,4);return;}
    std::memcpy(path.text,lease->stage->path.c_str(),(path.length+1)*sizeof(wchar_t));
    wchar_t destination[1024]{};HANDLE copy=INVALID_HANDLE_VALUE;bool invoked=false;uint32_t evidence=4;
    __try {
        using GetApp=SharedMessage*(__fastcall*)(SharedMessage*);
        using GetShared=SharedMessage*(__fastcall*)(void*,SharedMessage*);
        using Factory=SharedMessage*(__fastcall*)(void*,SharedMessage*,void*);
        using ToUtf8=SmallString*(__fastcall*)(void*,SmallString*);
        ImageQtString* qstring=&path;
        reinterpret_cast<ToUtf8>(core_address(core_base,0x857a0))(&qstring,&roundtrip);
        char expected[4096]{};const int expected_size=WideCharToMultiByte(CP_UTF8,WC_ERR_INVALID_CHARS,path.text,path.length,expected,sizeof(expected),nullptr,nullptr);
        const char* actual=roundtrip.capacity>15?*reinterpret_cast<char**>(roundtrip.data):roundtrip.data;
        if(!expected_size || roundtrip.length!=static_cast<uint64_t>(expected_size) || !actual || std::memcmp(actual,expected,expected_size))__leave;
        reinterpret_cast<GetApp>(core_address(core_base,0x47220))(&app);
        if(!app.object || !app.control || reinterpret_cast<uintptr_t>(app.object)!=bound_context || (!same_account(p.account) || !target_allowed(p)))__leave;
        reinterpret_cast<GetShared>(core_address(core_base,0x36c740))(app.object,&context);
        if(!context.object || !context.control)__leave;
        reinterpret_cast<GetShared>(core_address(core_base,0x6f4d90))(context.object,&manager);
        if(!manager.object || !manager.control)__leave;
        reinterpret_cast<Factory>(core_address(core_base,0x3d9d990))(nullptr,&message,&qstring);
        if(!message.object || !message.control)__leave;
        auto object=static_cast<unsigned char*>(message.object);
        if(*reinterpret_cast<uint32_t*>(object+0x118)!=3)__leave;
        const auto prepared=reinterpret_cast<const ImageWideString*>(object+0x120);
        if(!prepared->length || prepared->length>=1024 || prepared->capacity<prepared->length)__leave;
        const wchar_t* chars=prepared->capacity>7?*reinterpret_cast<const wchar_t*const*>(prepared->data):prepared->data;
        if(!chars)__leave;std::memcpy(destination,chars,prepared->length*sizeof(wchar_t));
        if(wcslen(destination)!=prepared->length || !image_local_path(destination,true))__leave;
        const auto original=reinterpret_cast<ImageWideString*>(object+0x168);
        if(!image_wide_equals(original,path.text))__leave;
        // Both paths must outlive this task and bridge unload. Let Weixin own
        // its InputTemp copy; never leave the async uploader our deleted source.
        using CopyWide=void*(__fastcall*)(void*,const ImageWideString*);
        reinterpret_cast<CopyWide>(core_address(core_base,0x9b5d0))(original,prepared);
        if(!image_wide_equals(original,destination) || !image_wide_equals(prepared,destination))__leave;
        if(!set_native_string(reinterpret_cast<SmallString*>(object+0xb0),p.target))__leave;
        if((!same_account(p.account) || !target_allowed(p)) || closing || uint64_t(std::time(nullptr))>=p.deadline || WaitForSingleObject(controller,0)!=WAIT_TIMEOUT)__leave;
        if(lease->probe){evidence=8;__leave;}
        copy=CreateFileW(destination,GENERIC_WRITE|DELETE,FILE_SHARE_READ,nullptr,CREATE_NEW,FILE_ATTRIBUTE_NORMAL,nullptr);
        if(copy==INVALID_HANDLE_VALUE)__leave;
        DWORD written=0;
        if(!WriteFile(copy,lease->stage->bytes.data(),static_cast<DWORD>(lease->stage->bytes.size()),&written,nullptr) || written!=lease->stage->bytes.size() || !FlushFileBuffers(copy))__leave;
        if((!same_account(p.account) || !target_allowed(p)) || closing || uint64_t(std::time(nullptr))>=p.deadline || WaitForSingleObject(controller,0)!=WAIT_TIMEOUT)__leave;
        SharedMessage transferred=message;
        _InterlockedIncrement(reinterpret_cast<volatile LONG*>(static_cast<unsigned char*>(message.control)+8));
        CloseHandle(copy);copy=INVALID_HANDLE_VALUE;invoked=true;evidence=0;
        send_media_with_identity(manager.object,result,&transferred,lease->index,p.target,3);
        evidence=!*reinterpret_cast<uint32_t*>(result) && !*reinterpret_cast<uint32_t*>(result+4)?7:0;
    } __except(record_exception(GetExceptionInformation(),&report)){evidence=invoked?0:4;}
    if(copy!=INVALID_HANDLE_VALUE){FILE_DISPOSITION_INFO remove{TRUE};SetFileInformationByHandle(copy,FileDispositionInfo,&remove,sizeof(remove));CloseHandle(copy);}
    guarded_release_shared(reinterpret_cast<SharedMessage*>(result+40),&report);
    image_free_string(reinterpret_cast<SmallString*>(result+8),&report);image_free_string(&roundtrip,&report);
    guarded_release_shared(&message,&report);guarded_release_shared(&manager,&report);guarded_release_shared(&context,&report);guarded_release_shared(&app,&report);
    receipt_set(lease->index,evidence);
}
static DWORD image_dispatch(WercPacket& p){
    const auto op=static_cast<WercOp>(p.operation);const auto now=uint64_t(std::time(nullptr));
    if(!target_allowed(p) || !ascii_id(p.attempt,64) || !digest_valid(p.digest) || p.deadline<=now || p.deadline>now+300)return ERROR_INVALID_PARAMETER;
    if(op==WercOp::ImageBegin){
        if(live_tasks || live_completions)return ERROR_BUSY;
        const uint32_t w=p.dropped&65535,h=p.dropped>>16;
        if(p.message_id<57 || p.message_id>image_limit || !w || !h || w>4096 || h>4096 || uint64_t(w)*h>4194304 || !digest_valid(p.text))return ERROR_INVALID_PARAMETER;
        image_stage=std::make_unique<ImageStage>();image_stage->packet=p;image_stage->bytes.reserve(static_cast<size_t>(p.message_id));return 0;
    }
    if(!image_stage || std::memcmp(image_stage->packet.target,p.target,128) || std::memcmp(image_stage->packet.attempt,p.attempt,64) || std::memcmp(image_stage->packet.digest,p.digest,65) || image_stage->packet.deadline!=p.deadline || image_stage->packet.dropped!=p.dropped)return ERROR_INVALID_STATE;
    if(op==WercOp::ImageChunk){
        if(!p.present || p.present>sizeof(p.text) || p.sequence!=image_stage->bytes.size() || p.present>image_stage->packet.message_id-image_stage->bytes.size())return ERROR_INVALID_PARAMETER;
        image_stage->bytes.insert(image_stage->bytes.end(),p.text,p.text+p.present);return 0;
    }
    if(op!=WercOp::ImageProbe && op!=WercOp::ImageSend)return ERROR_INVALID_FUNCTION;
    const bool probe=op==WercOp::ImageProbe;
    if(image_stage->bytes.size()!=image_stage->packet.message_id || !image_shape(image_stage->bytes,p.dropped) || !voice_hash(image_stage->bytes,image_stage->packet.text))return ERROR_INVALID_DATA;
    for(size_t i=0;i<receipt_count;++i)if(!std::strcmp(receipts[i].attempt,p.attempt)){
        if(receipts[i].kind!=(probe?4u:3u) || std::strcmp(receipts[i].digest,p.digest) || std::strcmp(receipts[i].target,p.target))return ERROR_INVALID_DATA;
        AcquireSRWLockShared(&receipt_lock);p.receipt=receipts[i].evidence;ReleaseSRWLockShared(&receipt_lock);image_stage.reset();return 0;
    }
    if(live_tasks || live_completions || !reserve_receipt())return ERROR_BUSY;
    if(!image_stage_file(*image_stage)){image_stage.reset();return ERROR_INVALID_DATA;}
    const size_t index=receipt_count++;strcpy_s(receipts[index].target,p.target);strcpy_s(receipts[index].attempt,p.attempt);strcpy_s(receipts[index].digest,p.digest);receipts[index].kind=probe?4:3;receipt_set(index,1);
    auto lease=std::make_shared<ImageLease>(std::move(image_stage),index,probe);
    std::function<void()> task=[lease](){_InterlockedIncrement(&active_callbacks);__try{image_on_task(lease.get());}__finally{_InterlockedDecrement(&active_callbacks);}};
    TestConfig config{};config.size=sizeof(config);config.pid=GetCurrentProcessId();
    const auto status=schedule_task(config,&task);if(status)receipt_set(index,0);
    AcquireSRWLockShared(&receipt_lock);p.receipt=receipts[index].evidence;ReleaseSRWLockShared(&receipt_lock);return status;
}
