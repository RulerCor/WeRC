// Derived from helper/native/image_download.h (noncommercial license).
// Exact profile 1, bounded image/file/video download. No caller-selected path or send operation.
struct DownloadRequest {
    SmallString task_id,filekey,aeskey;
    uint32_t image_type,padding;
    ImageWideString destination;
    uint64_t reserved;
    unsigned char flags[2],tail[6];
};
static_assert(sizeof(DownloadRequest)==0x98 && offsetof(DownloadRequest,destination)==0x68,"download ABI");
struct DownloadStatus {uintptr_t vtable;int32_t code;uint32_t padding;SharedMessage details;};
static_assert(sizeof(DownloadStatus)==32,"download status ABI");
struct DownloadResult {
    WercPacket packet{};
    std::vector<unsigned char> bytes;
    volatile LONG state=1,cancelled=0;
    DWORD error=0,native_status=0xffffffffu,stage=0;
    uint64_t actual_size=0;
    ~DownloadResult(){SecureZeroMemory(&packet,sizeof(packet));if(!bytes.empty())SecureZeroMemory(bytes.data(),bytes.size());}
};
static std::shared_ptr<DownloadResult> download_result;
static volatile LONG64 download_sequence=0;
static constexpr uint64_t video_download_limit=16ull*1024*1024;
static uint64_t download_limit(uint32_t type){return type==4?video_download_limit:image_limit;}
using DownloadFactory=void*(__fastcall*)(void*,void*);
static DownloadFactory download_original_factory=nullptr;
static void* volatile download_active_task=nullptr;
static volatile LONG download_favorite_scene=0;
static bool download_hex(const char* text,size_t capacity,size_t exact=0){
    const size_t n=strnlen_s(text,capacity);
    if(!n || n>=capacity || n%2 || (exact && n!=exact))return false;
    for(size_t i=0;i<n;++i)if(!((text[i]>='0'&&text[i]<='9')||(text[i]>='a'&&text[i]<='f')||(text[i]>='A'&&text[i]<='F')))return false;
    return true;
}
static bool download_descriptor_valid(const WercPacket& p){
    return p.message_id>0 && p.message_id<=download_limit(p.dropped) && (p.dropped==1 || p.dropped==2 || p.dropped==5 || p.dropped==4) &&
        download_hex(p.text,769) && download_hex(p.text+769,33,32);
}
static void* __fastcall download_factory(void* task,void* output){
    _InterlockedIncrement(&active_callbacks);
    void* result=nullptr;
    __try {
        result=download_original_factory(task,output);
        // Restrict the adjustment to this bridge's own unpredictable task IDs.
        // All other client tasks keep their own CDN parameters.
        const auto object=static_cast<const unsigned char*>(task);
        const auto id=reinterpret_cast<const SmallString*>(object+0x238);
        static const char prefix[]="werc_receive_img_";
        if(result && task==InterlockedCompareExchangePointer(&download_active_task,nullptr,nullptr) && *reinterpret_cast<const uintptr_t*>(object)==core_address(core_base,0x97ca198) &&
            id->length>=sizeof(prefix)-1 && id->length<128 && id->capacity>=id->length){
            const char* bytes=id->capacity>15?*reinterpret_cast<const char*const*>(id->data):id->data;
            if(bytes && !std::memcmp(bytes,prefix,sizeof(prefix)-1)){
                if(InterlockedCompareExchange(&download_favorite_scene,0,0)) {
                    // Favorites use the client's generic CDN scene. Type 5's
                    // default scene 8 is for chat attachments, not favorites.
                    *reinterpret_cast<int32_t*>(static_cast<unsigned char*>(result)+0xd8)=2;
                } else {
                    *reinterpret_cast<int32_t*>(static_cast<unsigned char*>(result)+0xd4)=1;
                    *reinterpret_cast<int32_t*>(static_cast<unsigned char*>(result)+0xd8)=1;
                }
            }
        }
    } __finally {_InterlockedDecrement(&active_callbacks);}
    return result;
}
static DWORD download_factory_install(){
    if(image_download_factory_hook_created)return 0;
    void* address=reinterpret_cast<void*>(core_address(core_base,0x5240720));
    MH_STATUS status=MH_CreateHook(address,reinterpret_cast<void*>(&download_factory),reinterpret_cast<void**>(&download_original_factory));
    if(status!=MH_OK)return 600+status;
    status=MH_EnableHook(address);
    if(status!=MH_OK){MH_RemoveHook(address);return 600+status;}
    image_download_factory_hook_created=true;return 0;
}
static bool download_alive(const DownloadResult* value){
    return !closing && !value->cancelled && uint64_t(std::time(nullptr))<value->packet.deadline &&
        WaitForSingleObject(controller,0)==WAIT_TIMEOUT && same_account(value->packet.account);
}
static void download_destroy_request(DownloadRequest* request,TestResult* report){
    __try {using Destroy=void(__fastcall*)(DownloadRequest*);reinterpret_cast<Destroy>(core_address(core_base,0x2ba7940))(request);}
    __except(record_exception(GetExceptionInformation(),report)){}
}
static void download_destroy_wide(ImageWideString* value,TestResult* report){
    __try {using Destroy=void(__fastcall*)(ImageWideString*);reinterpret_cast<Destroy>(core_address(core_base,0x7a60))(value);}
    __except(record_exception(GetExceptionInformation(),report)){}
}
static bool download_set_wide(ImageWideString* value,const wchar_t* path){
    const size_t n=wcsnlen_s(path,1024);
    if(!n || n>=1024 || value->length || value->capacity!=7)return false;
    if(n<=7){std::memcpy(value->data,path,(n+1)*2);value->length=n;return true;}
    using Alloc=void*(__fastcall*)(size_t);
    void* memory=reinterpret_cast<Alloc>(core_address(core_base,0x74ab89c))((n+1)*2);
    if(!memory)return false;
    std::memcpy(memory,path,(n+1)*2);std::memcpy(value->data,&memory,sizeof(memory));value->length=value->capacity=n;return true;
}
// Keep native calls/SEH separate from vector ownership and C++ exceptions.
static DWORD download_native(DownloadResult* result,wchar_t (&path)[1024],TestResult* report){
    if(!download_alive(result))return ERROR_CANCELLED;
    ImageWideString root{};root.capacity=7;
    DownloadRequest request{};request.task_id.capacity=request.filekey.capacity=request.aeskey.capacity=15;request.destination.capacity=7;request.image_type=result->packet.dropped;
    SharedMessage app{},context{},service{},task{};DownloadStatus status{};
    DWORD answer=ERROR_INVALID_DATA;HANDLE parent=INVALID_HANDLE_VALUE;
    __try {
        result->stage=1;
        using GetRoot=ImageWideString*(__fastcall*)(ImageWideString*,uint32_t);
        reinterpret_cast<GetRoot>(core_address(core_base,0x8edb0))(&root,2);
        result->stage=2;if(!root.length || root.length>900 || root.capacity<root.length)__leave;
        const wchar_t* chars=root.capacity>7?*reinterpret_cast<const wchar_t*const*>(root.data):root.data;
        if(!chars)__leave;std::memcpy(path,chars,root.length*2);path[root.length]=0;
        result->stage=3;if(!image_local_path(path,false))__leave;
        parent=CreateFileW(path,FILE_READ_ATTRIBUTES,FILE_SHARE_READ|FILE_SHARE_WRITE,nullptr,OPEN_EXISTING,FILE_FLAG_BACKUP_SEMANTICS|FILE_FLAG_OPEN_REPARSE_POINT,nullptr);
        result->stage=4;if(parent==INVALID_HANDLE_VALUE)__leave;
        BY_HANDLE_FILE_INFORMATION info{};
        if(!GetFileInformationByHandle(parent,&info) || !(info.dwFileAttributes&FILE_ATTRIBUTE_DIRECTORY) || (info.dwFileAttributes&FILE_ATTRIBUTE_REPARSE_POINT))__leave;
        unsigned char random[16]{};if(BCryptGenRandom(nullptr,random,sizeof(random),BCRYPT_USE_SYSTEM_PREFERRED_RNG)<0)__leave;
        char random_hex[33]{};for(size_t i=0;i<16;++i)sprintf_s(random_hex+i*2,3,"%02x",random[i]);
        wchar_t suffix[80]{};swprintf_s(suffix,L"\\werc-receive-image-%hs.bin",random_hex);
        result->stage=5;if(wcscat_s(path,suffix)!=0 || !image_local_path(path,false) || GetFileAttributesW(path)!=INVALID_FILE_ATTRIBUTES || GetLastError()!=ERROR_FILE_NOT_FOUND)__leave;
        char task_id[96]{};sprintf_s(task_id,"werc_receive_img_%s_%llu",random_hex,static_cast<unsigned long long>(_InterlockedIncrement64(&download_sequence)));
        result->stage=6;if(!set_native_string(&request.task_id,task_id) || !set_native_string(&request.filekey,result->packet.text) ||
            !set_native_string(&request.aeskey,result->packet.text+769) || !download_set_wide(&request.destination,path))__leave;
        using GetApp=SharedMessage*(__fastcall*)(SharedMessage*);
        using GetService=SharedMessage*(__fastcall*)(void*,SharedMessage*);
        using Dispatcher=void*(__fastcall*)(void*);
        reinterpret_cast<GetApp>(core_address(core_base,0x47220))(&app);
        result->stage=7;if(!app.object || !app.control || reinterpret_cast<uintptr_t>(app.object)!=bound_context || !download_alive(result))__leave;
        reinterpret_cast<GetService>(core_address(core_base,0x36c740))(app.object,&context);
        result->stage=8;if(!context.object || !context.control)__leave;
        reinterpret_cast<GetService>(core_address(core_base,0x356dd0))(context.object,&service);
        result->stage=9;if(!service.object || !service.control)__leave;
        void* dispatcher=reinterpret_cast<Dispatcher>(core_address(core_base,0x10e390))(service.object);
        result->stage=10;if(!dispatcher || !download_alive(result))__leave;
        int argument=0,category=14;
        using Make=SharedMessage*(__fastcall*)(SharedMessage*,const DownloadRequest*,const int*);
        reinterpret_cast<Make>(core_address(core_base,0x2ba5330))(&task,&request,&argument);
        result->stage=11;if(!task.object || !task.control || *static_cast<const uintptr_t*>(task.object)!=core_address(core_base,0x97ca198) || !download_alive(result))__leave;
        using Dispatch=DownloadStatus*(__fastcall*)(void*,DownloadStatus*,const int*,const SharedMessage*);
        InterlockedExchange(&download_favorite_scene,result->packet.operation==uint32_t(WercOp::DownloadFavoriteBegin));
        InterlockedExchangePointer(&download_active_task,task.object);
        result->stage=12;reinterpret_cast<Dispatch>(core_address(core_base,0xe5c720))(dispatcher,&status,&category,&task);
        result->native_status=static_cast<DWORD>(status.code);
        result->stage=13;if(status.vtable!=core_address(core_base,0x8dfb4b8) || status.code!=0)__leave;
        answer=download_alive(result)?ERROR_SUCCESS:ERROR_CANCELLED;
    } __except(record_exception(GetExceptionInformation(),report)){answer=ERROR_INVALID_DATA;}
    InterlockedExchangePointer(&download_active_task,nullptr);
    InterlockedExchange(&download_favorite_scene,0);
    guarded_release_shared(&status.details,report);guarded_release_shared(&task,report);guarded_release_shared(&service,report);guarded_release_shared(&context,report);guarded_release_shared(&app,report);
    download_destroy_request(&request,report);download_destroy_wide(&root,report);
    if(parent!=INVALID_HANDLE_VALUE)CloseHandle(parent);
    return report->exception?ERROR_INVALID_DATA:answer;
}
static DWORD download_read_file(const wchar_t* path,uint64_t expected,std::vector<unsigned char>& bytes,uint64_t* actual=nullptr,uint64_t maximum=image_limit){
    HANDLE file=CreateFileW(path,GENERIC_READ,FILE_SHARE_READ,nullptr,OPEN_EXISTING,FILE_FLAG_OPEN_REPARSE_POINT,nullptr);
    if(file==INVALID_HANDLE_VALUE)return GetLastError();
    DWORD answer=ERROR_INVALID_DATA;BY_HANDLE_FILE_INFORMATION info{};LARGE_INTEGER size{};
    if(GetFileSizeEx(file,&size) && actual)*actual=static_cast<uint64_t>(size.QuadPart);
    try {
        if(GetFileInformationByHandle(file,&info) && !(info.dwFileAttributes&(FILE_ATTRIBUTE_DIRECTORY|FILE_ATTRIBUTE_REPARSE_POINT)) &&
            GetFileSizeEx(file,&size) && size.QuadPart>0 && static_cast<uint64_t>(size.QuadPart)<=maximum && (!expected || static_cast<uint64_t>(size.QuadPart)==expected)){
            bytes.resize(static_cast<size_t>(size.QuadPart));DWORD n=0;
            if(ReadFile(file,bytes.data(),static_cast<DWORD>(bytes.size()),&n,nullptr) && n==bytes.size())answer=0;
        }
    } catch(...){answer=ERROR_NOT_ENOUGH_MEMORY;}
    CloseHandle(file);return answer;
}
static void download_execute(DownloadResult* value){
    wchar_t path[1024]{};TestResult report{};
    DWORD status=download_native(value,path,&report);
    // The client can transform the CDN object. XML length is an admission hint,
    // not the completed image length. Actual disk/memory bytes have their own cap.
    if(!status){value->stage=14;status=download_read_file(path,(value->packet.dropped==1 || value->packet.dropped==5)?value->packet.message_id:0,value->bytes,&value->actual_size,download_limit(value->packet.dropped));}
    // Only a fully generated unique leaf may be removed, never the native root.
    const wchar_t* leaf=wcsrchr(path,L'\\');
    if(leaf && !wcsncmp(leaf+1,L"werc-receive-image-",19)){
        if(!DeleteFileW(path) && GetLastError()!=ERROR_FILE_NOT_FOUND)status=ERROR_ACCESS_DENIED;
        wchar_t temporary[1030]{};
        if(swprintf_s(temporary,L"%s.tmp",path)>0 && !DeleteFileW(temporary) && GetLastError()!=ERROR_FILE_NOT_FOUND)status=ERROR_ACCESS_DENIED;
    }
    if(!download_alive(value))status=ERROR_CANCELLED;
    if(status && !value->bytes.empty()){SecureZeroMemory(value->bytes.data(),value->bytes.size());value->bytes.clear();}
    SecureZeroMemory(value->packet.text,sizeof(value->packet.text));value->error=status;
    InterlockedExchange(&value->state,status?3:2);
}
struct DownloadTask {
    std::shared_ptr<DownloadResult> value;
    explicit DownloadTask(std::shared_ptr<DownloadResult> v):value(std::move(v)){_InterlockedIncrement(&live_tasks);}
    ~DownloadTask(){_InterlockedDecrement(&live_tasks);}
};
static DWORD download_dispatch(WercPacket& p){
    const auto op=static_cast<WercOp>(p.operation);
    if(std::memcmp(p.target,"filehelper",11) || !ascii_id(p.attempt,64) || !digest_valid(p.digest))return ERROR_INVALID_PARAMETER;
    if(op==WercOp::DownloadImageBegin || op==WercOp::DownloadFileBegin || op==WercOp::DownloadVideoBegin || op==WercOp::DownloadOriginalImageBegin || op==WercOp::DownloadFavoriteBegin){
        if(op==WercOp::DownloadFavoriteBegin ? !(p.dropped==2 || p.dropped==4 || p.dropped==5) : p.dropped!=(op==WercOp::DownloadFileBegin?5u:op==WercOp::DownloadVideoBegin?4u:op==WercOp::DownloadOriginalImageBegin?1u:2u))return ERROR_INVALID_PARAMETER;
        const auto now=uint64_t(std::time(nullptr));
        if((core_profile_id!=1&&(core_profile_id!=5&&core_profile_id!=14)) || p.deadline<=now || p.deadline>now+120 || !download_descriptor_valid(p))return ERROR_INVALID_PARAMETER;
        if(download_result || live_tasks || live_completions)return ERROR_BUSY;
        const DWORD status=download_factory_install();if(status)return status;
        auto value=std::make_shared<DownloadResult>();value->packet=p;
        auto lease=std::make_shared<DownloadTask>(value);
        std::function<void()> task=[lease](){download_execute(lease->value.get());};
        download_result=value;TestConfig config{};config.size=sizeof(config);config.pid=GetCurrentProcessId();
        const DWORD scheduled=schedule_task(config,&task);
        SecureZeroMemory(p.text,sizeof(p.text));p.receipt=1;
        if(scheduled){InterlockedExchange(&value->cancelled,1);download_result.reset();}
        return scheduled;
    }
    if(!download_result || std::memcmp(download_result->packet.attempt,p.attempt,64) || std::memcmp(download_result->packet.digest,p.digest,65))return ERROR_INVALID_STATE;
    auto value=download_result;
    SecureZeroMemory(p.text,sizeof(p.text));
    if(op==WercOp::DownloadRelease){InterlockedExchange(&value->cancelled,1);download_result.reset();return 0;}
    p.receipt=static_cast<uint32_t>(InterlockedCompareExchange(&value->state,0,0));
    p.message_id=p.receipt==2?value->bytes.size():value->packet.message_id;
    if(op==WercOp::DownloadPoll){p.dropped=p.receipt==3?value->error:0;p.present=p.receipt==3?value->native_status:0;p.sequence=p.receipt==3?value->stage:0;p.deadline=p.receipt==3?value->actual_size:0;return 0;}
    if(op!=WercOp::DownloadChunk || p.receipt!=2 || !download_alive(value.get()) || p.sequence>=value->bytes.size())return ERROR_INVALID_STATE;
    p.present=static_cast<uint32_t>(std::min<uint64_t>(sizeof(p.text),value->bytes.size()-p.sequence));
    std::memcpy(p.text,value->bytes.data()+p.sequence,p.present);return 0;
}
