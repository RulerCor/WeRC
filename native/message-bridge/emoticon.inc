// Exact profile 1 GIF/type47 branch, verified against the supported module.
// Emoticon staging keeps .gif; no file/text fallback. Probe deletes its owned copy.
static constexpr size_t emoticon_limit=1024*1024;
struct GifNativeProbe { uint8_t valid,padding[7]; unsigned char image[16]; uint32_t width,height; };
static_assert(sizeof(GifNativeProbe)==32 && offsetof(GifNativeProbe,width)==0x18,"GIF probe layout");
static bool emoticon_shape(const std::vector<unsigned char>& bytes,uint32_t packed){
    if(bytes.size()<14 || bytes.size()>emoticon_limit ||
       (std::memcmp(bytes.data(),"GIF87a",6) && std::memcmp(bytes.data(),"GIF89a",6)) || bytes.back()!=0x3b)return false;
    const uint32_t w=bytes[6]|(uint32_t(bytes[7])<<8),h=bytes[8]|(uint32_t(bytes[9])<<8);
    return w && h && w<=1024 && h<=1024 && packed==(w|(h<<16));
}
static void emoticon_probe_release(GifNativeProbe* probe,TestResult* report){
    __try {using Release=void(__fastcall*)(void*);reinterpret_cast<Release>(core_address(core_base,0x4e6360))(probe->image);}
    __except(record_exception(GetExceptionInformation(),report)){}
}
static std::unique_ptr<FileStage> emoticon_stage;
static bool emoticon_name_valid(const char* name){const size_t n=strnlen_s(name,121);return n>4 && n<=120 && !_stricmp(name+n-4,".gif");}
static void emoticon_on_task(const FileLease* lease){
    const auto& p=lease->stage->packet;
    if(!lease->probe && ensure_text_task_result()){receipt_set(lease->index,4);return;}
    if(!file_live(p) || !emoticon_shape(lease->stage->bytes,p.dropped)){receipt_set(lease->index,4);return;}
    SharedMessage app{},provider{},manager{},message{};ImageWideString directory{},prepared{};directory.capacity=prepared.capacity=7;
    SmallString token{};token.capacity=15;alignas(16) unsigned char result[128]{};TestResult report{};
    wchar_t destination[1024]{};HANDLE copy=INVALID_HANDLE_VALUE;bool created=false,invoked=false;uint32_t evidence=4;GifNativeProbe info{};bool probe_initialized=false;
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
        if(!unsuffixed || prepared.length+4>=1024 || wcscpy_s(suffixed,unsuffixed) || wcscat_s(suffixed,L".gif"))__leave;
        ImageWideString view_path{};*reinterpret_cast<const wchar_t**>(view_path.data)=suffixed;
        view_path.length=wcslen(suffixed);view_path.capacity=1023;
        using CopyPath=void*(__fastcall*)(void*,const ImageWideString*);
        reinterpret_cast<CopyPath>(core_address(core_base,0x9b5d0))(&prepared,&view_path);
        const wchar_t* path=file_wide_data(prepared);
        if(!path || prepared.length<=directory.length || wcsncmp(path,root,static_cast<size_t>(directory.length)) || path[directory.length]!=L'\\' || !image_local_path(path,true))__leave;
        wcscpy_s(destination,path);
        using Factory=SharedMessage*(__fastcall*)(SharedMessage*);
        reinterpret_cast<Factory>(core_address(core_base,0x1bfb1c0))(&message);
        if(!message.object || !message.control || *static_cast<const uintptr_t*>(message.object)!=core_address(core_base,0x90daff8))__leave;
        auto object=static_cast<unsigned char*>(message.object);
        *reinterpret_cast<uint64_t*>(object+0x118)=47ull;
        using CopyWide=void*(__fastcall*)(void*,const ImageWideString*);
        reinterpret_cast<CopyWide>(core_address(core_base,0x9b5d0))(object+0x120,&prepared);
        if(!image_wide_equals(reinterpret_cast<ImageWideString*>(object+0x120),destination) ||
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
        // The client's GIF branch validates type=2 and uses its image decoder.
        using Validate=int(__fastcall*)(const ImageWideString*);
        if(reinterpret_cast<Validate>(core_address(core_base,0xe7a8e0))(&prepared)!=2)__leave;
        using Probe=GifNativeProbe*(__fastcall*)(GifNativeProbe*,const ImageWideString*);
        probe_initialized=true;reinterpret_cast<Probe>(core_address(core_base,0x5309c0))(&info,&prepared);
        if(!info.valid || info.width!=(p.dropped&0xffff) || info.height!=(p.dropped>>16) || !file_live(p))__leave;
        *reinterpret_cast<uint64_t*>(object+0x778)=uint64_t(info.width)|(uint64_t(info.height)<<32);
        if(lease->probe){evidence=14;__leave;}
        SharedMessage transferred=message;
        _InterlockedIncrement(reinterpret_cast<volatile LONG*>(static_cast<unsigned char*>(message.control)+8));
        CloseHandle(copy);copy=INVALID_HANDLE_VALUE;invoked=true;evidence=0;
        send_media_with_identity(manager.object,result,&transferred,lease->index,p.target,47);
        evidence=!*reinterpret_cast<uint32_t*>(result) && !*reinterpret_cast<uint32_t*>(result+4)?13:0;
    } __except(record_exception(GetExceptionInformation(),&report)){evidence=invoked?0:4;}
    if(copy!=INVALID_HANDLE_VALUE)CloseHandle(copy);
    if(created && !invoked && !DeleteFileW(destination))evidence=4;
    if(probe_initialized)emoticon_probe_release(&info,&report);
    guarded_release_shared(reinterpret_cast<SharedMessage*>(result+40),&report);image_free_string(reinterpret_cast<SmallString*>(result+8),&report);
    image_free_string(&token,&report);file_release_wide(&prepared,&report);file_release_wide(&directory,&report);
    guarded_release_shared(&message,&report);guarded_release_shared(&manager,&report);guarded_release_shared(&provider,&report);guarded_release_shared(&app,&report);
    receipt_set(lease->index,evidence);
}
static DWORD emoticon_dispatch(WercPacket& p){
    const auto op=static_cast<WercOp>(p.operation);const auto now=uint64_t(std::time(nullptr));
    if(!target_allowed(p) || !ascii_id(p.attempt,64) || !digest_valid(p.digest) || p.deadline<=now || p.deadline>now+300 || !(p.dropped&0xffff) || !(p.dropped>>16) || (p.dropped&0xffff)>1024 || (p.dropped>>16)>1024)return ERROR_INVALID_PARAMETER;
    if(op==WercOp::EmoticonBegin){
        if(live_tasks || live_completions)return ERROR_BUSY;
        if(!p.message_id || p.message_id>emoticon_limit || !digest_valid(p.text) || !file_name_valid(p.text+65) || !emoticon_name_valid(p.text+65))return ERROR_INVALID_PARAMETER;
        emoticon_stage=std::make_unique<FileStage>();emoticon_stage->packet=p;emoticon_stage->bytes.reserve(static_cast<size_t>(p.message_id));return 0;
    }
    if(!emoticon_stage || std::memcmp(emoticon_stage->packet.target,p.target,128) || std::memcmp(emoticon_stage->packet.attempt,p.attempt,64) || std::memcmp(emoticon_stage->packet.digest,p.digest,65) || emoticon_stage->packet.deadline!=p.deadline || emoticon_stage->packet.dropped!=p.dropped)return ERROR_INVALID_STATE;
    if(op==WercOp::EmoticonChunk){
        if(!p.present || p.present>sizeof(p.text) || p.sequence!=emoticon_stage->bytes.size() || p.present>emoticon_stage->packet.message_id-emoticon_stage->bytes.size())return ERROR_INVALID_PARAMETER;
        emoticon_stage->bytes.insert(emoticon_stage->bytes.end(),p.text,p.text+p.present);return 0;
    }
    if(op!=WercOp::EmoticonProbe && op!=WercOp::EmoticonSend)return ERROR_INVALID_FUNCTION;
    const bool probe=op==WercOp::EmoticonProbe;
    if(!emoticon_shape(emoticon_stage->bytes,emoticon_stage->packet.dropped) || emoticon_stage->bytes.size()!=emoticon_stage->packet.message_id || !voice_hash(emoticon_stage->bytes,emoticon_stage->packet.text))return ERROR_INVALID_DATA;
    for(size_t i=0;i<receipt_count;++i)if(!std::strcmp(receipts[i].attempt,p.attempt)){
        if(receipts[i].kind!=(probe?10u:9u) || std::strcmp(receipts[i].digest,p.digest) || std::strcmp(receipts[i].target,p.target))return ERROR_INVALID_DATA;
        AcquireSRWLockShared(&receipt_lock);p.receipt=receipts[i].evidence;ReleaseSRWLockShared(&receipt_lock);emoticon_stage.reset();return 0;
    }
    if(live_tasks || live_completions || !reserve_receipt())return ERROR_BUSY;
    const size_t index=receipt_count++;strcpy_s(receipts[index].target,p.target);strcpy_s(receipts[index].attempt,p.attempt);strcpy_s(receipts[index].digest,p.digest);receipts[index].kind=probe?10:9;receipt_set(index,1);
    auto lease=std::make_shared<FileLease>(std::move(emoticon_stage),index,probe);
    std::function<void()> task=[lease](){_InterlockedIncrement(&active_callbacks);__try{emoticon_on_task(lease.get());}__finally{_InterlockedDecrement(&active_callbacks);}};
    TestConfig config{};config.size=sizeof(config);config.pid=GetCurrentProcessId();
    const auto status=schedule_task(config,&task);if(status)receipt_set(index,0);
    AcquireSRWLockShared(&receipt_lock);p.receipt=receipts[index].evidence;ReleaseSRWLockShared(&receipt_lock);return status;
}
