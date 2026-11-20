// Profile1 native media model: 43b54b0 video branch -> type6; 4310b90
// serializes width/height at +88/+8c and videoDuration at +90. The shared
// model constructor initializes scene8; 60def90 sets content type15 at +30.
// Common media ownership comes from the existing native PNG cover factory.
static DWORD prepare_video(Lease* lease) {
    if(!lease->video)return ERROR_SUCCESS;
    ImageWideString directory{},prepared{};directory.capacity=prepared.capacity=7;
    SmallString token{};token.capacity=15;HANDLE file=INVALID_HANDLE_VALUE;
    DWORD status=ERROR_INVALID_DATA;alignas(16) unsigned char info[0x38]{};
    __try {
        if(lease->config.image_count!=1 || !lease->media[0].object || !lease->media[0].control)__leave;
        const auto& input=*lease->video;
        if(input.bytes.empty() || input.bytes.size()>file_limit || !voice_hash(input.bytes,input.packet.text))__leave;
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
        bool valid=true;
        for(const char* c=name;*c;++c)if(!((*c>='a'&&*c<='z')||(*c>='A'&&*c<='Z')||(*c>='0'&&*c<='9')||*c=='-'||*c=='_'))valid=false;
        if(!valid)__leave;
        strcat_s(name,".mp4");
        struct View {const char* data;uint64_t length;} view{name,std::strlen(name)};
        reinterpret_cast<Join>(core_address(core_base,0x9f4b0))(&directory,&prepared,&view);
        const wchar_t* path=file_wide_data(prepared);
        if(!path || prepared.length<=directory.length || std::wcsncmp(path,root,size_t(directory.length)) ||
           path[directory.length]!=L'\\' || !image_local_path(path,true) || wcscpy_s(lease->video_path,path))__leave;
        file=CreateFileW(path,GENERIC_WRITE,FILE_SHARE_READ,nullptr,CREATE_NEW,FILE_ATTRIBUTE_NORMAL,nullptr);
        if(file==INVALID_HANDLE_VALUE)__leave;
        lease->video_created=true;DWORD written=0;
        if(!WriteFile(file,input.bytes.data(),DWORD(input.bytes.size()),&written,nullptr) ||
           written!=input.bytes.size() || !FlushFileBuffers(file))__leave;
        CloseHandle(file);file=INVALID_HANDLE_VALUE;
        file=CreateFileW(path,GENERIC_READ,FILE_SHARE_READ,nullptr,OPEN_EXISTING,FILE_FLAG_OPEN_REPARSE_POINT,nullptr);
        if(file==INVALID_HANDLE_VALUE)__leave;
        BY_HANDLE_FILE_INFORMATION attr{};LARGE_INTEGER size{};
        if(!GetFileInformationByHandle(file,&attr) || (attr.dwFileAttributes&(FILE_ATTRIBUTE_REPARSE_POINT|FILE_ATTRIBUTE_DIRECTORY)) ||
           !GetFileSizeEx(file,&size) || size.QuadPart!=LONGLONG(input.bytes.size()))__leave;
        using ReadInfo=void*(__fastcall*)(void*,const ImageWideString*,bool);
        reinterpret_cast<ReadInfo>(core_address(core_base,0x29eb500))(info,&prepared,true);
        const auto seconds=*reinterpret_cast<const uint32_t*>(info+0x14);
        if(!info[0] || !seconds || seconds>60 || seconds*1000+1000<input.packet.dropped ||
           seconds*1000>input.packet.dropped+1000 || !same_account(lease->config.self_wxid))__leave;
        char video_path[4096]{},cover_path[4096]{};
        if(!WideCharToMultiByte(CP_UTF8,WC_ERR_INVALID_CHARS,path,-1,video_path,sizeof(video_path),nullptr,nullptr) ||
           !WideCharToMultiByte(CP_UTF8,WC_ERR_INVALID_CHARS,lease->image_paths[0],-1,cover_path,sizeof(cover_path),nullptr,nullptr))__leave;
        auto media=static_cast<unsigned char*>(lease->media[0].object);
        using ConstructString=SmallString*(__fastcall*)(SmallString*,const char*);
        for(const auto offset:{0x28,0x48,0x68}) {
            auto value=reinterpret_cast<SmallString*>(media+offset);
            free_native_string(value);
            reinterpret_cast<ConstructString>(core_address(core_base,0x88ad0))(value,offset==0x48?video_path:cover_path);
        }
        *reinterpret_cast<uint32_t*>(media)=6;
        *reinterpret_cast<float*>(media+0x88)=float(uint32_t(input.packet.sequence)&65535);
        *reinterpret_cast<float*>(media+0x8c)=float(uint32_t(input.packet.sequence)>>16);
        *reinterpret_cast<float*>(media+0x90)=float(seconds);
        char check[4096]{};
        if(!read_string(reinterpret_cast<SmallString*>(media+0x48),check,sizeof(check)) || std::strcmp(check,video_path))__leave;
        status=ERROR_SUCCESS;
    } __except(record_exception(GetExceptionInformation(),&lease->report)){status=ERROR_GEN_FAILURE;}
    if(file!=INVALID_HANDLE_VALUE)CloseHandle(file);
    image_free_string(&token,&lease->report);file_release_wide(&prepared,&lease->report);file_release_wide(&directory,&lease->report);
    return lease->report.exception?ERROR_GEN_FAILURE:status;
}
