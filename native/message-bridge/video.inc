// Exact profile 1 video constructor, statically verified against the supported module.
// Video staging keeps .mp4; no file/text fallback. Probe deletes its owned copy.
static std::unique_ptr<FileStage> video_stage;
static bool video_name_valid(const char* name){const size_t n=strnlen_s(name,121);return n>4 && n<=120 && !_stricmp(name+n-4,".mp4");}
static void video_on_task(const FileLease* lease){
    const auto& p=lease->stage->packet;
    if(!lease->probe && ensure_text_task_result()){receipt_set(lease->index,4);return;}
    if(!file_live(p)){receipt_set(lease->index,4);return;}
    SharedMessage app{},provider{},manager{},message{};ImageWideString directory{},prepared{};directory.capacity=prepared.capacity=7;
    SmallString token{};token.capacity=15;alignas(16) unsigned char result[128]{};TestResult report{};
    wchar_t destination[1024]{};HANDLE copy=INVALID_HANDLE_VALUE;bool created=false,invoked=false;uint32_t evidence=4;alignas(16) unsigned char info[0x38]{};
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
        const wchar_t* unsuffixed=file_wide_data(prepared);
        wchar_t suffixed[1024]{};
        if(!unsuffixed || prepared.length+4>=1024 || wcscpy_s(suffixed,unsuffixed) || wcscat_s(suffixed,L".mp4"))__leave;
        ImageWideString view_path{};*reinterpret_cast<const wchar_t**>(view_path.data)=suffixed;
        view_path.length=wcslen(suffixed);view_path.capacity=1023;
        using CopyPath=void*(__fastcall*)(void*,const ImageWideString*);
        reinterpret_cast<CopyPath>(core_address(core_base,0x9b5d0))(&prepared,&view_path);
        const wchar_t* path=file_wide_data(prepared);
        if(!path || prepared.length<=directory.length || wcsncmp(path,root,static_cast<size_t>(directory.length)) || path[directory.length]!=L'\\' || !image_local_path(path,true))__leave;
        wcscpy_s(destination,path);
        using Factory=SharedMessage*(__fastcall*)(SharedMessage*);
        reinterpret_cast<Factory>(core_address(core_base,0x2f267d0))(&message);
        if(!message.object || !message.control || *static_cast<const uintptr_t*>(message.object)!=core_address(core_base,0x90daeb8))__leave;
        auto object=static_cast<unsigned char*>(message.object);
        *reinterpret_cast<uint64_t*>(object+0x118)=43ull;
        using CopyWide=void*(__fastcall*)(void*,const ImageWideString*);
        reinterpret_cast<CopyWide>(core_address(core_base,0x9b5d0))(object+0x120,&prepared);
        *reinterpret_cast<uint64_t*>(object+0x1c8)=lease->stage->bytes.size();
        if(!image_wide_equals(reinterpret_cast<ImageWideString*>(object+0x120),destination) ||
           !set_native_string(reinterpret_cast<SmallString*>(object+0x1a8),p.text+65) ||
           !set_native_string(reinterpret_cast<SmallString*>(object+0xb0),p.target))__leave;
        if(!file_live(p))__leave;
        
        reinterpret_cast<GetShared>(core_address(core_base,0x36c740))(app.object,&provider);
        if(!provider.object || !provider.control)__leave;
        reinterpret_cast<GetShared>(core_address(core_base,0x6f4d90))(provider.object,&manager);
        if(!manager.object || !manager.control)__leave;
        copy=CreateFileW(destination,GENERIC_WRITE|DELETE,FILE_SHARE_READ,nullptr,CREATE_NEW,FILE_ATTRIBUTE_NORMAL,nullptr);
        if(copy==INVALID_HANDLE_VALUE)__leave;
        created=true;
        DWORD written=0;
        if(!WriteFile(copy,lease->stage->bytes.data(),static_cast<DWORD>(lease->stage->bytes.size()),&written,nullptr) || written!=lease->stage->bytes.size() || !FlushFileBuffers(copy) || !file_live(p))__leave;
        CloseHandle(copy);copy=INVALID_HANDLE_VALUE;
        copy=CreateFileW(destination,GENERIC_READ,FILE_SHARE_READ,nullptr,OPEN_EXISTING,FILE_ATTRIBUTE_NORMAL,nullptr);
        if(copy==INVALID_HANDLE_VALUE)__leave;
        // Query the client's own media reader. Profile 1 POD layout is 0x38 bytes.
        using ReadInfo=void*(__fastcall*)(void*,const ImageWideString*,bool);
        reinterpret_cast<ReadInfo>(core_address(core_base,0x29eb500))(info,&prepared,true);
        const uint32_t seconds=*reinterpret_cast<const uint32_t*>(info+0x14);
        AcquireSRWLockExclusive(&receipt_lock);receipts[lease->index].native_duration=seconds;ReleaseSRWLockExclusive(&receipt_lock);
        if(!info[0] || !seconds || seconds>60 ||
           seconds*1000+1000<p.dropped || seconds*1000>p.dropped+1000 || !file_live(p))__leave;
        *reinterpret_cast<uint32_t*>(object+0x76c)=seconds;
        if(lease->probe){evidence=12;__leave;}
        SharedMessage transferred=message;
        _InterlockedIncrement(reinterpret_cast<volatile LONG*>(static_cast<unsigned char*>(message.control)+8));
        CloseHandle(copy);copy=INVALID_HANDLE_VALUE;invoked=true;evidence=0;
        send_media_with_identity(manager.object,result,&transferred,lease->index,p.target,43);
        evidence=!*reinterpret_cast<uint32_t*>(result) && !*reinterpret_cast<uint32_t*>(result+4)?11:0;
    } __except(record_exception(GetExceptionInformation(),&report)){evidence=invoked?0:4;}
    if(copy!=INVALID_HANDLE_VALUE)CloseHandle(copy);
    if(created && !invoked && !DeleteFileW(destination))evidence=4;
    guarded_release_shared(reinterpret_cast<SharedMessage*>(result+40),&report);image_free_string(reinterpret_cast<SmallString*>(result+8),&report);
    image_free_string(&token,&report);file_release_wide(&prepared,&report);file_release_wide(&directory,&report);
    guarded_release_shared(&message,&report);guarded_release_shared(&manager,&report);guarded_release_shared(&provider,&report);guarded_release_shared(&app,&report);
    receipt_set(lease->index,evidence);
}
static DWORD video_dispatch(WercPacket& p){
    const auto op=static_cast<WercOp>(p.operation);const auto now=uint64_t(std::time(nullptr));
    if(!target_allowed(p) || !ascii_id(p.attempt,64) || !digest_valid(p.digest) || p.deadline<=now || p.deadline>now+300 || p.dropped<1000 || p.dropped>60000)return ERROR_INVALID_PARAMETER;
    if(op==WercOp::VideoBegin){
        if(live_tasks || live_completions)return ERROR_BUSY;
        if(!p.message_id || p.message_id>file_limit || !digest_valid(p.text) || !file_name_valid(p.text+65) || !video_name_valid(p.text+65))return ERROR_INVALID_PARAMETER;
        video_stage=std::make_unique<FileStage>();video_stage->packet=p;video_stage->bytes.reserve(static_cast<size_t>(p.message_id));return 0;
    }
    if(!video_stage || std::memcmp(video_stage->packet.target,p.target,128) || std::memcmp(video_stage->packet.attempt,p.attempt,64) || std::memcmp(video_stage->packet.digest,p.digest,65) || video_stage->packet.deadline!=p.deadline || video_stage->packet.dropped!=p.dropped)return ERROR_INVALID_STATE;
    if(op==WercOp::VideoChunk){
        if(!p.present || p.present>sizeof(p.text) || p.sequence!=video_stage->bytes.size() || p.present>video_stage->packet.message_id-video_stage->bytes.size())return ERROR_INVALID_PARAMETER;
        video_stage->bytes.insert(video_stage->bytes.end(),p.text,p.text+p.present);return 0;
    }
    if(op!=WercOp::VideoProbe && op!=WercOp::VideoSend)return ERROR_INVALID_FUNCTION;
    const bool probe=op==WercOp::VideoProbe;
    if(video_stage->bytes.size()!=video_stage->packet.message_id || !voice_hash(video_stage->bytes,video_stage->packet.text))return ERROR_INVALID_DATA;
    for(size_t i=0;i<receipt_count;++i)if(!std::strcmp(receipts[i].attempt,p.attempt)){
        if(receipts[i].kind!=(probe?8u:7u) || std::strcmp(receipts[i].digest,p.digest) || std::strcmp(receipts[i].target,p.target))return ERROR_INVALID_DATA;
        AcquireSRWLockShared(&receipt_lock);p.receipt=receipts[i].evidence;ReleaseSRWLockShared(&receipt_lock);video_stage.reset();return 0;
    }
    if(live_tasks || live_completions || !reserve_receipt())return ERROR_BUSY;
    const size_t index=receipt_count++;strcpy_s(receipts[index].target,p.target);strcpy_s(receipts[index].attempt,p.attempt);strcpy_s(receipts[index].digest,p.digest);receipts[index].kind=probe?8:7;receipt_set(index,1);
    auto lease=std::make_shared<FileLease>(std::move(video_stage),index,probe);
    std::function<void()> task=[lease](){_InterlockedIncrement(&active_callbacks);__try{video_on_task(lease.get());}__finally{_InterlockedDecrement(&active_callbacks);}};
    TestConfig config{};config.size=sizeof(config);config.pid=GetCurrentProcessId();
    const auto status=schedule_task(config,&task);if(status)receipt_set(index,0);
    AcquireSRWLockShared(&receipt_lock);p.receipt=receipts[index].evidence;ReleaseSRWLockShared(&receipt_lock);return status;
}
