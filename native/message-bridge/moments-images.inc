// Profile-1 image factory/vector and InputTemp path acceptance were verified
// independently with canonical PNG bytes. Paths are always discovered natively.
static DWORD prepare_image(Lease* lease,uint32_t index) {
    ImageWideString directory{},prepared{};directory.capacity=prepared.capacity=7;
    SmallString token{};token.capacity=15;HANDLE file=INVALID_HANDLE_VALUE;
    DWORD status=ERROR_INVALID_DATA;
    __try {
        if(index>=lease->images.size() || index>=9 || !lease->images[index])__leave;
        const auto& input=*lease->images[index];
        if(!image_shape(input.bytes,input.packet.dropped) || !voice_hash(input.bytes,input.packet.text))__leave;
        using GetDirectory=ImageWideString*(__fastcall*)(ImageWideString*);
        using EnsureDirectory=uint32_t(__fastcall*)(const ImageWideString*);
        using MakeToken=SmallString*(__fastcall*)(SmallString*);
        using Join=ImageWideString*(__fastcall*)(const ImageWideString*,ImageWideString*,const void*);
        reinterpret_cast<GetDirectory>(core_address(core_base,0x22d8390))(&directory);
        const wchar_t* root=file_wide_data(directory);
        if(!root || !image_local_path(root,true) || !reinterpret_cast<EnsureDirectory>(core_address(core_base,0x389830))(&directory))__leave;
        reinterpret_cast<MakeToken>(core_address(core_base,0x57fe90))(&token);
        char name[160]{};
        if(!read_string(&token,name,128))__leave;
        bool valid_name=true;
        for(const char* c=name;*c;++c)if(!((*c>='a'&&*c<='z')||(*c>='A'&&*c<='Z')||(*c>='0'&&*c<='9')||*c=='-'||*c=='_'))valid_name=false;
        if(!valid_name)__leave;
        strcat_s(name,".png");
        struct View {const char* data;uint64_t length;} view{name,std::strlen(name)};
        reinterpret_cast<Join>(core_address(core_base,0x9f4b0))(&directory,&prepared,&view);
        const wchar_t* path=file_wide_data(prepared);
        if(!path || prepared.length<=directory.length || std::wcsncmp(path,root,size_t(directory.length)) ||
           path[directory.length]!=L'\\' || !image_local_path(path,true))__leave;
        if(wcscpy_s(lease->image_paths[index],path))__leave;
        file=CreateFileW(path,GENERIC_WRITE,FILE_SHARE_READ,nullptr,CREATE_NEW,FILE_ATTRIBUTE_NORMAL,nullptr);
        if(file==INVALID_HANDLE_VALUE)__leave;
        lease->image_created[index]=true;
        DWORD written=0;
        if(!WriteFile(file,input.bytes.data(),DWORD(input.bytes.size()),&written,nullptr) ||
           written!=input.bytes.size() || !FlushFileBuffers(file))__leave;
        CloseHandle(file);file=INVALID_HANDLE_VALUE;
        // DELETE access prevents Weixin's reader from sharing this handle.
        file=CreateFileW(path,GENERIC_READ,FILE_SHARE_READ,nullptr,OPEN_EXISTING,FILE_FLAG_OPEN_REPARSE_POINT,nullptr);
        if(file==INVALID_HANDLE_VALUE)__leave;
        BY_HANDLE_FILE_INFORMATION info{};LARGE_INTEGER size{};
        if(!GetFileInformationByHandle(file,&info) || (info.dwFileAttributes&(FILE_ATTRIBUTE_REPARSE_POINT|FILE_ATTRIBUTE_DIRECTORY)) ||
           !GetFileSizeEx(file,&size) || size.QuadPart!=LONGLONG(input.bytes.size()))__leave;
        uint32_t kind=1;
        using Validate=bool(__fastcall*)(const ImageWideString*,uint32_t*);
        if(!reinterpret_cast<Validate>(core_address(core_base,0x90b60))(&prepared,&kind))__leave;
        using Factory=void(__fastcall*)(SharedMessage*,const ImageWideString*);
        reinterpret_cast<Factory>(core_address(core_base,0x2227fa0))(&lease->media[index],&prepared);
        const auto media=static_cast<const unsigned char*>(lease->media[index].object);
        if(!media || !lease->media[index].control || *reinterpret_cast<const uint32_t*>(media)!=2 ||
           *reinterpret_cast<const float*>(media+0x88)!=float(input.packet.dropped&65535) ||
           *reinterpret_cast<const float*>(media+0x8c)!=float(input.packet.dropped>>16))__leave;
        char native_path[4096]{};wchar_t wide[1024]{};
        if(!read_string(reinterpret_cast<const SmallString*>(media+0x48),native_path,sizeof(native_path)) ||
           !MultiByteToWideChar(CP_UTF8,MB_ERR_INVALID_CHARS,native_path,-1,wide,1024) ||
           std::wcscmp(wide,path) || !same_account(lease->config.self_wxid))__leave;
        status=ERROR_SUCCESS;
    } __except(record_exception(GetExceptionInformation(),&lease->report)) {status=ERROR_GEN_FAILURE;}
    if(file!=INVALID_HANDLE_VALUE)CloseHandle(file);
    image_free_string(&token,&lease->report);file_release_wide(&prepared,&lease->report);file_release_wide(&directory,&lease->report);
    return lease->report.exception?ERROR_GEN_FAILURE:status;
}
static DWORD prepare_images(Lease* lease) {
    if(lease->config.image_count>9 || lease->images.size()!=lease->config.image_count)return ERROR_INVALID_DATA;
    for(uint32_t index=0;index<lease->config.image_count;++index){
        if(closing || uint64_t(std::time(nullptr))>=lease->expires)return ERROR_TIMEOUT;
        const auto status=prepare_image(lease,index);if(status)return status;
    }
    const auto video_status=prepare_video(lease);if(video_status)return video_status;
    if(!lease->config.image_count)return ERROR_SUCCESS;
    __try {
        using Assign=void(__fastcall*)(void*,const SharedMessage*,size_t);
        reinterpret_cast<Assign>(core_address(core_base,0xee95f0))(static_cast<unsigned char*>(lease->model.object)+model_field(0x118),lease->media,lease->config.image_count);
        return ERROR_SUCCESS;
    } __except(record_exception(GetExceptionInformation(),&lease->report)) {return ERROR_GEN_FAILURE;}
}
