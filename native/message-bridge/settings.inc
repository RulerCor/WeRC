// Fixed native preferences only. No arbitrary key, address, input or window API.
namespace native_settings {
struct Entry { uintptr_t read, write; };
static constexpr Entry old_entries[]={{0x99a1c0,0x99a310},{0x999f20,0x99a070},{0x99ddd0,0x99dc90}};
static constexpr Entry new_entries[]={{0x9c4700,0x9c4850},{0x9c4460,0x9c45b0},{0x9c82e0,0x9c81a0}};
static constexpr Entry new53_entries[]={{0x9c3d60,0x9c3eb0},{0x9c3ac0,0x9c3c10},{0x9c7940,0x9c7800}};
static const Entry (&selected_entries())[3] { if(core_profile_id==14)return new53_entries; return core_profile_id==5 ? new_entries : old_entries; }
static DWORD validate() {
    const auto& entries=selected_entries();
    if((core_profile_id!=1&&(core_profile_id!=5&&core_profile_id!=14)&&core_profile_id!=6)||core_settings_mask!=7)return ERROR_NOT_SUPPORTED;
    static const unsigned char getter[]={0x55,0x56,0x53,0x48,0x83,0xec,0x60,0x48,0x8d,0x6c,0x24,0x60,0x48,0xc7,0x45,0xf8,0xfe,0xff,0xff,0xff,0x48,0x8d,0x4d,0xc8};
    static const unsigned char setter[]={0x55,0x56,0x53,0x48,0x83,0xec,0x60,0x48,0x8d,0x6c,0x24,0x60,0x48,0xc7,0x45,0xf8,0xfe,0xff,0xff,0xff,0x88,0xd3,0x48,0x8d};
    for(const auto& e:entries)if(std::memcmp(reinterpret_cast<void*>(core_base+e.read),getter,sizeof(getter))||std::memcmp(reinterpret_cast<void*>(core_base+e.write),setter,sizeof(setter)))return ERROR_INVALID_DATA;
    return 0;
}
struct Work {
    WercPacket packet{};
    HANDLE done=CreateEventW(nullptr,TRUE,FALSE,nullptr);
    volatile LONG stage=0; // 0 queued, 1 executing, 2 completed, 3 expired before execution
    DWORD status=ERROR_IO_PENDING;
    TaskLease lifetime;
    explicit Work(const WercPacket& p,const TestConfig& c):packet(p),lifetime(c){}
    ~Work(){if(done)CloseHandle(done);SecureZeroMemory(&packet,sizeof(packet));}
};
static DWORD execute(WercPacket& p) {
    const auto& entries=selected_entries();
    if(closing||!same_account(p.account)||uint64_t(std::time(nullptr))>=p.deadline)return ERROR_ACCESS_DENIED;
    const auto check=validate();if(check)return check;
    SharedMessage config{};TestResult release_error{};
    DWORD result=ERROR_NOT_READY;
    __try {
        __try {
            using Config=SharedMessage*(__fastcall*)(SharedMessage*);
            reinterpret_cast<Config>(core_address(core_base,0x51420))(&config);
            // The client getters return defaults when storage is unavailable.
            // Never report that fallback as an observed setting.
            if(!config.object||!config.control||! *reinterpret_cast<void**>(static_cast<unsigned char*>(config.object)+0x40))return ERROR_NOT_READY;
            using Read=bool(__fastcall*)(void*);
            using Write=void(__fastcall*)(void*,bool);
            if(p.operation==70){
                p.present=7;p.dropped=0;
                for(size_t i=0;i<std::size(entries);++i)if(reinterpret_cast<Read>(core_base+entries[i].read)(nullptr))p.dropped|=1u<<i;
                strcpy_s(p.text,core_profile_id==14 ? "4.2.0.53" : core_profile_id==5 ? "4.2.0.52" : "4.1.15.13");

                strcpy_s(p.digest,core_profile_id==14 ? "ac6b174d98ee1b69e95c99a7a39c3197c8c167b7c6d32f5b72c7a7659c104825" : core_profile_id==5 ? "d733489eea0b62af77e1aa61ba645cd941583d2a64fb2d7572da818d285f8d84" : "10f8e995453e2da46d4f2b5080cd6da1f13cc5147746adc119ceae38cb039de5");
            }else{
                const auto& e=entries[p.dropped-1];
                const bool before=reinterpret_cast<Read>(core_base+e.read)(nullptr);
                const bool desired=p.present!=0;
                p.message_id=before;
                if(p.sequence<2&&before!=(p.sequence!=0)){p.receipt=3;p.sequence=before;return 0;}
                if(before==desired){p.receipt=2;p.sequence=before;return 0;}
                if(closing||!same_account(p.account)||uint64_t(std::time(nullptr))>=p.deadline)return ERROR_ACCESS_DENIED;
                reinterpret_cast<Write>(core_base+e.write)(nullptr,desired);
                const bool after=reinterpret_cast<Read>(core_base+e.read)(nullptr);
                p.sequence=after;p.receipt=after==desired?1:4;
            }
            result=same_account(p.account)?0:ERROR_ACCESS_DENIED;
        } __finally { guarded_release_shared(&config,&release_error); }
    } __except(EXCEPTION_EXECUTE_HANDLER){result=ERROR_INVALID_DATA;}
    return release_error.exception?ERROR_INVALID_DATA:result;
}
static void run(Work* w) {
    if(InterlockedCompareExchange(&w->stage,1,0)!=0)return;
    w->status=execute(w->packet);
    InterlockedExchange(&w->stage,2);SetEvent(w->done);
}
static DWORD dispatch(WercPacket& p) {
    const auto now=uint64_t(std::time(nullptr));
    if(p.deadline<=now||p.deadline>now+30)return ERROR_INVALID_PARAMETER;
    if(p.operation==71&&(p.dropped<1||p.dropped>3||p.present>1||p.sequence>2))return ERROR_INVALID_PARAMETER;
    const auto check=validate();if(check)return check;
    TestConfig c{};c.size=sizeof(c);c.pid=GetCurrentProcessId();
    auto work=std::make_shared<Work>(p,c);if(!work->done)return ERROR_NOT_ENOUGH_MEMORY;
    std::function<void()> task=[work](){run(work.get());};
    const auto scheduled=schedule_task(c,&task);if(scheduled){InterlockedCompareExchange(&work->stage,3,0);return scheduled;}
    if(WaitForSingleObject(work->done,8000)!=WAIT_OBJECT_0){InterlockedCompareExchange(&work->stage,3,0);return ERROR_TIMEOUT;}
    if(work->status)return work->status;
    p=work->packet;return 0;
}
}
