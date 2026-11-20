// Helper-derived ordinary-file constructor, exact profile 1 only.
// File bytes/name come from the authenticated upload, never a caller path.
static constexpr size_t file_limit=8*1024*1024;
static bool file_name_valid(const char* name){
    const size_t n=strnlen_s(name,121);
    if(!n || n>120 || name[0]==' ' || name[n-1]==' ' || name[n-1]=='.' ||
       !MultiByteToWideChar(CP_UTF8,MB_ERR_INVALID_CHARS,name,static_cast<int>(n),nullptr,0))return false;
    for(size_t i=0;i<n;++i)if(static_cast<unsigned char>(name[i])<32 || name[i]==127 || std::strchr("/\\:*?\"<>|",name[i]))return false;
    char stem[121]{};size_t length=0;while(length<n && name[length]!='.'){stem[length]=name[length];++length;}
    if(!_stricmp(stem,"CON") || !_stricmp(stem,"PRN") || !_stricmp(stem,"AUX") || !_stricmp(stem,"NUL"))return false;
    return !(length==4 && (!_strnicmp(stem,"COM",3)||!_strnicmp(stem,"LPT",3)) && stem[3]>='1' && stem[3]<='9');
}
struct FileStage {
    WercPacket packet{};std::vector<unsigned char> bytes;
    ~FileStage(){if(!bytes.empty())SecureZeroMemory(bytes.data(),bytes.size());SecureZeroMemory(&packet,sizeof(packet));}
};
static std::unique_ptr<FileStage> file_stage;
struct FileLease {
    std::unique_ptr<FileStage> stage;size_t index;bool probe;
    FileLease(std::unique_ptr<FileStage> value,size_t slot,bool inspect):stage(std::move(value)),index(slot),probe(inspect){_InterlockedIncrement(&live_tasks);}
    ~FileLease(){stage.reset();_InterlockedDecrement(&live_tasks);}
};
static const wchar_t* file_wide_data(const ImageWideString& value){
    if(!value.length || value.length>=1024 || value.capacity<value.length || value.capacity>1024*1024)return nullptr;
    const wchar_t* p=value.capacity>7?*reinterpret_cast<const wchar_t*const*>(value.data):value.data;
    return p && !p[value.length]?p:nullptr;
}
static void file_release_wide(ImageWideString* value,TestResult* report){
    __try {using Destroy=void(__fastcall*)(void*);reinterpret_cast<Destroy>(core_address(core_base,0x7a60))(value);}
    __except(record_exception(GetExceptionInformation(),report)){}
}
static bool file_live(const WercPacket& p){return !closing && uint64_t(std::time(nullptr))<p.deadline && WaitForSingleObject(controller,0)==WAIT_TIMEOUT && same_account(p.account) && target_allowed(p);}
static void file_on_task(const FileLease* lease){
    const auto& p=lease->stage->packet;
    if(!lease->probe && ensure_text_task_result()){receipt_set(lease->index,4);return;}
    if(!file_live(p)){receipt_set(lease->index,4);return;}
    SharedMessage app{},provider{},manager{},message{};ImageWideString directory{},prepared{};directory.capacity=prepared.capacity=7;
    SmallString token{};token.capacity=15;alignas(16) unsigned char result[128]{};TestResult report{};
    wchar_t destination[1024]{};HANDLE copy=INVALID_HANDLE_VALUE;bool invoked=false;uint32_t evidence=4;
    __try {
        using GetApp=SharedMessage*(__fastcall*)(SharedMessage*);
        using GetShared=SharedMessage*(__fastcall*)(void*,SharedMessage*);
        reinterpret_cast<GetApp>(core_address(core_base,0x47220))(&app);
        if(!app.object || !app.control || reinterpret_cast<uintptr_t>(app.object)!=bound_context || !file_live(p))__leave;
        using GetDirectory=ImageWideString*(__fastcall*)(ImageWideString*);
        using EnsureDirectory=uint32_t(__fastcall*)(const ImageWideString*);
        using MakeToken=SmallString*(__fastcall*)(SmallString*);
        using Join=ImageWideString*(__fastcall*)(const ImageWideString*,ImageWideString*,const void*);
        reinterpret_cast<GetDirectory>(core_address(core_base,0x22d8390))(&directory);
        const wchar_t* root=file_wide_data(directory);
        if(!root || !image_local_path(root,true) || !reinterpret_cast<EnsureDirectory>(core_address(core_base,0x389830))(&directory))__leave;
        reinterpret_cast<MakeToken>(core_address(core_base,0x57fe90))(&token);
        if(!token.length || token.length>=128 || token.capacity<token.length)__leave;
        const char* chars=token.capacity>15?*reinterpret_cast<const char*const*>(token.data):token.data;
        if(!chars)__leave;
        struct View {const char* data;uint64_t length;} view{chars,token.length};
        reinterpret_cast<Join>(core_address(core_base,0x9f4b0))(&directory,&prepared,&view);
        const wchar_t* path=file_wide_data(prepared);
        if(!path || prepared.length<=directory.length || wcsncmp(path,root,static_cast<size_t>(directory.length)) || path[directory.length]!=L'\\' || !image_local_path(path,true))__leave;
        wcscpy_s(destination,path);
        using Factory=SharedMessage*(__fastcall*)(SharedMessage*);
        reinterpret_cast<Factory>(core_address(core_base,0x2bab360))(&message);
        if(!message.object || !message.control || *static_cast<const uintptr_t*>(message.object)!=core_address(core_base,0x8e53718))__leave;
        auto object=static_cast<unsigned char*>(message.object);
        *reinterpret_cast<uint64_t*>(object+0x118)=0x600000031ull;
        using CopyWide=void*(__fastcall*)(void*,const ImageWideString*);
        reinterpret_cast<CopyWide>(core_address(core_base,0x9b5d0))(object+0x120,&prepared);
        *reinterpret_cast<uint64_t*>(object+0x1c8)=lease->stage->bytes.size();
        if(!image_wide_equals(reinterpret_cast<ImageWideString*>(object+0x120),destination) ||
           !set_native_string(reinterpret_cast<SmallString*>(object+0x1a8),p.text+65) ||
           !set_native_string(reinterpret_cast<SmallString*>(object+0xb0),p.target))__leave;
        if(!file_live(p))__leave;
        if(lease->probe){evidence=10;__leave;}
        reinterpret_cast<GetShared>(core_address(core_base,0x36c740))(app.object,&provider);
        if(!provider.object || !provider.control)__leave;
        reinterpret_cast<GetShared>(core_address(core_base,0x6f4d90))(provider.object,&manager);
        if(!manager.object || !manager.control)__leave;
        copy=CreateFileW(destination,GENERIC_WRITE|DELETE,FILE_SHARE_READ,nullptr,CREATE_NEW,FILE_ATTRIBUTE_NORMAL,nullptr);
        if(copy==INVALID_HANDLE_VALUE)__leave;
        DWORD written=0;
        if(!WriteFile(copy,lease->stage->bytes.data(),static_cast<DWORD>(lease->stage->bytes.size()),&written,nullptr) || written!=lease->stage->bytes.size() || !FlushFileBuffers(copy) || !file_live(p))__leave;
        SharedMessage transferred=message;
        _InterlockedIncrement(reinterpret_cast<volatile LONG*>(static_cast<unsigned char*>(message.control)+8));
        CloseHandle(copy);copy=INVALID_HANDLE_VALUE;invoked=true;evidence=0;
        send_media_with_identity(manager.object,result,&transferred,lease->index,p.target,0x600000031ull);
        evidence=!*reinterpret_cast<uint32_t*>(result) && !*reinterpret_cast<uint32_t*>(result+4)?9:0;
    } __except(record_exception(GetExceptionInformation(),&report)){evidence=invoked?0:4;}
    if(copy!=INVALID_HANDLE_VALUE){FILE_DISPOSITION_INFO remove{TRUE};SetFileInformationByHandle(copy,FileDispositionInfo,&remove,sizeof(remove));CloseHandle(copy);}
    guarded_release_shared(reinterpret_cast<SharedMessage*>(result+40),&report);image_free_string(reinterpret_cast<SmallString*>(result+8),&report);
    image_free_string(&token,&report);file_release_wide(&prepared,&report);file_release_wide(&directory,&report);
    guarded_release_shared(&message,&report);guarded_release_shared(&manager,&report);guarded_release_shared(&provider,&report);guarded_release_shared(&app,&report);
    receipt_set(lease->index,evidence);
}
static DWORD file_dispatch(WercPacket& p){
    const auto op=static_cast<WercOp>(p.operation);const auto now=uint64_t(std::time(nullptr));
    if(!target_allowed(p) || !ascii_id(p.attempt,64) || !digest_valid(p.digest) || p.deadline<=now || p.deadline>now+300 || p.dropped)return ERROR_INVALID_PARAMETER;
    if(op==WercOp::FileBegin){
        if(live_tasks || live_completions)return ERROR_BUSY;
        if(!p.message_id || p.message_id>file_limit || !digest_valid(p.text) || !file_name_valid(p.text+65))return ERROR_INVALID_PARAMETER;
        file_stage=std::make_unique<FileStage>();file_stage->packet=p;file_stage->bytes.reserve(static_cast<size_t>(p.message_id));return 0;
    }
    if(!file_stage || std::memcmp(file_stage->packet.target,p.target,128) || std::memcmp(file_stage->packet.attempt,p.attempt,64) || std::memcmp(file_stage->packet.digest,p.digest,65) || file_stage->packet.deadline!=p.deadline)return ERROR_INVALID_STATE;
    if(op==WercOp::FileChunk){
        if(!p.present || p.present>sizeof(p.text) || p.sequence!=file_stage->bytes.size() || p.present>file_stage->packet.message_id-file_stage->bytes.size())return ERROR_INVALID_PARAMETER;
        file_stage->bytes.insert(file_stage->bytes.end(),p.text,p.text+p.present);return 0;
    }
    if(op!=WercOp::FileProbe && op!=WercOp::FileSend)return ERROR_INVALID_FUNCTION;
    const bool probe=op==WercOp::FileProbe;
    if(file_stage->bytes.size()!=file_stage->packet.message_id || !voice_hash(file_stage->bytes,file_stage->packet.text))return ERROR_INVALID_DATA;
    for(size_t i=0;i<receipt_count;++i)if(!std::strcmp(receipts[i].attempt,p.attempt)){
        if(receipts[i].kind!=(probe?6u:5u) || std::strcmp(receipts[i].digest,p.digest) || std::strcmp(receipts[i].target,p.target))return ERROR_INVALID_DATA;
        AcquireSRWLockShared(&receipt_lock);p.receipt=receipts[i].evidence;ReleaseSRWLockShared(&receipt_lock);file_stage.reset();return 0;
    }
    if(live_tasks || live_completions || !reserve_receipt())return ERROR_BUSY;
    const size_t index=receipt_count++;strcpy_s(receipts[index].target,p.target);strcpy_s(receipts[index].attempt,p.attempt);strcpy_s(receipts[index].digest,p.digest);receipts[index].kind=probe?6:5;receipt_set(index,1);
    auto lease=std::make_shared<FileLease>(std::move(file_stage),index,probe);
    std::function<void()> task=[lease](){_InterlockedIncrement(&active_callbacks);__try{file_on_task(lease.get());}__finally{_InterlockedDecrement(&active_callbacks);}};
    TestConfig config{};config.size=sizeof(config);config.pid=GetCurrentProcessId();
    const auto status=schedule_task(config,&task);if(status)receipt_set(index,0);
    AcquireSRWLockShared(&receipt_lock);p.receipt=receipts[index].evidence;ReleaseSRWLockShared(&receipt_lock);return status;
}
