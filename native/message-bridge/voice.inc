// Voice constructor/submit path derived from helper/native/voice_send.h.
// Bytes arrive over bounded inherited IPC; no caller-supplied file paths.
struct VoiceStage {
    WercPacket packet{};
    std::vector<unsigned char> bytes;
    ~VoiceStage(){if(!bytes.empty())SecureZeroMemory(bytes.data(),bytes.size());SecureZeroMemory(&packet,sizeof(packet));}
};
static std::unique_ptr<VoiceStage> voice_stage;
static bool voice_hash(const std::vector<unsigned char>& bytes,const char* expected) {
    BCRYPT_ALG_HANDLE alg=nullptr;BCRYPT_HASH_HANDLE hash=nullptr;unsigned char digest[32]{};
    bool good=BCryptOpenAlgorithmProvider(&alg,BCRYPT_SHA256_ALGORITHM,nullptr,0)>=0;
    if(good)good=BCryptCreateHash(alg,&hash,nullptr,0,nullptr,0,0)>=0;
    if(good)good=BCryptHashData(hash,const_cast<unsigned char*>(bytes.data()),static_cast<ULONG>(bytes.size()),0)>=0 && BCryptFinishHash(hash,digest,32,0)>=0;
    if(hash)BCryptDestroyHash(hash);if(alg)BCryptCloseAlgorithmProvider(alg,0);
    char hex[65]{};static const char alphabet[]="0123456789abcdef";
    for(size_t i=0;i<32;++i){hex[i*2]=alphabet[digest[i]>>4];hex[i*2+1]=alphabet[digest[i]&15];}
    return good && !std::memcmp(hex,expected,65);
}
static bool voice_frames(const std::vector<unsigned char>& bytes,uint32_t duration) {
    if(bytes.size()<=12 || bytes.size()>512*1024 || std::memcmp(bytes.data(),"\x02#!SILK_V3",10))return false;
    size_t at=10;uint32_t packets=0;
    while(at<bytes.size()) {
        if(bytes.size()-at<2)return false;size_t n=bytes[at]|(size_t(bytes[at+1])<<8);at+=2;
        if(!n || n>1024 || n>bytes.size()-at || packets==3000)return false;
        at+=n;++packets;
    }
    return duration && packets*20==duration;
}
static SmallString voice_view(const char* bytes,size_t length) {
    SmallString s{};s.length=length;s.capacity=length<=15?15:length;
    if(length<=15)std::memcpy(s.data,bytes,length+1);else std::memcpy(s.data,&bytes,sizeof(bytes));return s;
}
struct VoiceLease {
    std::unique_ptr<VoiceStage> stage;size_t index;bool probe;
    VoiceLease(std::unique_ptr<VoiceStage> value,size_t slot,bool inspect):stage(std::move(value)),index(slot),probe(inspect){_InterlockedIncrement(&live_tasks);}
    ~VoiceLease(){_InterlockedDecrement(&live_tasks);}
};
static void voice_on_task(const VoiceLease* lease) {
    const auto& p=lease->stage->packet;
    if(!lease->probe && ensure_text_task_result()){receipt_set(lease->index,4);return;}
    if(closing || uint64_t(std::time(nullptr))>=p.deadline || WaitForSingleObject(controller,0)!=WAIT_TIMEOUT || (!same_account(p.account) || !target_allowed(p))) {receipt_set(lease->index,4);return;}
    SharedMessage context{},service{},manager{},message{};alignas(16) unsigned char result[128]{};TestResult report{};
    SmallString target=voice_view(p.target,std::strlen(p.target));
    SmallString payload=voice_view(reinterpret_cast<const char*>(lease->stage->bytes.data()),lease->stage->bytes.size()-1);
    alignas(8) unsigned char options[56]{};
    *reinterpret_cast<uint32_t*>(options)=4;*reinterpret_cast<uint32_t*>(options+4)=3;
    *reinterpret_cast<uint64_t*>(options+8)=30000;*reinterpret_cast<uint64_t*>(options+48)=15;
    uint32_t duration=p.dropped;bool invoked=false;uint32_t evidence=4;
    __try {
        using Context=SharedMessage* (__fastcall*)(SharedMessage*);
        using Service=SharedMessage* (__fastcall*)(void*,SharedMessage*);
        using Factory=SharedMessage* (__fastcall*)(SharedMessage*,const SmallString*,const SmallString*,const uint32_t*,const void*);
        reinterpret_cast<Context>(core_address(core_base,0x47220))(&context);
        if(!context.object || !context.control || reinterpret_cast<uintptr_t>(context.object)!=bound_context || (!same_account(p.account) || !target_allowed(p)))__leave;
        reinterpret_cast<Factory>(core_address(core_base,0x6be0020))(&message,&target,&payload,&duration,options);
        if(!message.object || !message.control || *static_cast<uintptr_t*>(message.object)!=core_address(core_base,0x9af9078))__leave;
        auto object=static_cast<unsigned char*>(message.object);*reinterpret_cast<uint32_t*>(object+0x118)=34;
        const auto receiver=reinterpret_cast<const SmallString*>(object+0xb0);
        const auto receiver_bytes=receiver->capacity<=15?receiver->data:*reinterpret_cast<const char*const*>(receiver->data);
        if(receiver->length!=target.length || receiver->capacity<receiver->length || !receiver_bytes || std::memcmp(receiver_bytes,p.target,target.length))__leave;
        const auto copied=reinterpret_cast<SmallString*>(object+0x758);
        const auto copied_bytes=copied->capacity<=15?copied->data:*reinterpret_cast<char**>(copied->data);
        if(copied->length!=payload.length || copied->capacity<copied->length || !copied_bytes ||
           std::memcmp(copied_bytes,lease->stage->bytes.data(),payload.length) || *reinterpret_cast<uint32_t*>(object+0x778)!=duration)__leave;
        if((!same_account(p.account) || !target_allowed(p)) || closing || uint64_t(std::time(nullptr))>=p.deadline || WaitForSingleObject(controller,0)!=WAIT_TIMEOUT)__leave;
        if(lease->probe) {evidence=6;__leave;}
        reinterpret_cast<Service>(core_address(core_base,0x36c740))(context.object,&service);
        if(!service.object || !service.control)__leave;
        reinterpret_cast<Service>(core_address(core_base,0x6f4d90))(service.object,&manager);
        if(!manager.object || !manager.control || (!same_account(p.account) || !target_allowed(p)) || closing ||
           uint64_t(std::time(nullptr))>=p.deadline || WaitForSingleObject(controller,0)!=WAIT_TIMEOUT)__leave;
        SharedMessage transferred=message;
        _InterlockedIncrement(reinterpret_cast<volatile LONG*>(static_cast<unsigned char*>(message.control)+8));
        invoked=true;evidence=0;
        send_media_with_identity(manager.object,result,&transferred,lease->index,p.target,34);
        evidence=!*reinterpret_cast<uint32_t*>(result) && !*reinterpret_cast<uint32_t*>(result+4)?5:0;
    } __except(record_exception(GetExceptionInformation(),&report)) {evidence=invoked?0:4;}
    guarded_release_shared(reinterpret_cast<SharedMessage*>(result+40),&report);
    // Same native allocator contract as helper's image/voice result destructor.
    __try {
        auto s=reinterpret_cast<SmallString*>(result+8);
        if(s->capacity>15 && s->capacity<1024*1024){
            void* memory=*reinterpret_cast<void**>(s->data);size_t size=s->capacity+1;
            if(size>=4096){memory=*(reinterpret_cast<void**>(memory)-1);size+=39;}
            using Free=void(__fastcall*)(void*,size_t);reinterpret_cast<Free>(core_address(core_base,0x74ab8e0))(memory,size);
        }
    } __except(record_exception(GetExceptionInformation(),&report)) {}
    guarded_release_shared(&message,&report);guarded_release_shared(&manager,&report);
    guarded_release_shared(&service,&report);guarded_release_shared(&context,&report);
    receipt_set(lease->index,evidence);
}
static DWORD voice_dispatch(WercPacket& p) {
    const auto op=static_cast<WercOp>(p.operation);const auto now=uint64_t(std::time(nullptr));
    if(!target_allowed(p) || !ascii_id(p.attempt,64) || !digest_valid(p.digest) || p.deadline<=now || p.deadline>now+300)return ERROR_INVALID_PARAMETER;
    if(op==WercOp::VoiceBegin) {
        if(live_tasks || live_completions)return ERROR_BUSY;
        if(p.message_id<=12 || p.message_id>512*1024 || !p.dropped || p.dropped>60000 || p.dropped%20 || !digest_valid(p.text))return ERROR_INVALID_PARAMETER;
        voice_stage=std::make_unique<VoiceStage>();voice_stage->packet=p;voice_stage->bytes.reserve(static_cast<size_t>(p.message_id)+1);return 0;
    }
    if(!voice_stage || std::memcmp(voice_stage->packet.target,p.target,128) || std::memcmp(voice_stage->packet.attempt,p.attempt,64) || std::memcmp(voice_stage->packet.digest,p.digest,65) ||
       voice_stage->packet.deadline!=p.deadline)return ERROR_INVALID_STATE;
    if(op==WercOp::VoiceChunk) {
        if(!p.present || p.present>sizeof(p.text) || p.sequence!=voice_stage->bytes.size() || p.present>voice_stage->packet.message_id-voice_stage->bytes.size())return ERROR_INVALID_PARAMETER;
        voice_stage->bytes.insert(voice_stage->bytes.end(),p.text,p.text+p.present);return 0;
    }
    if(op!=WercOp::VoiceProbe && op!=WercOp::VoiceSend)return ERROR_INVALID_FUNCTION;
    const bool probe=op==WercOp::VoiceProbe;
    if(voice_stage->bytes.size()!=voice_stage->packet.message_id || !voice_frames(voice_stage->bytes,voice_stage->packet.dropped) || !voice_hash(voice_stage->bytes,voice_stage->packet.text))return ERROR_INVALID_DATA;
    for(size_t i=0;i<receipt_count;++i)if(!std::strcmp(receipts[i].attempt,p.attempt)) {
        if(receipts[i].kind!=(probe?2u:1u) || std::strcmp(receipts[i].digest,p.digest) || std::strcmp(receipts[i].target,p.target))return ERROR_INVALID_DATA;
        AcquireSRWLockShared(&receipt_lock);p.receipt=receipts[i].evidence;ReleaseSRWLockShared(&receipt_lock);voice_stage.reset();return 0;
    }
    if(live_tasks || live_completions || !reserve_receipt())return ERROR_BUSY;
    const size_t index=receipt_count++;strcpy_s(receipts[index].target,p.target);strcpy_s(receipts[index].attempt,p.attempt);strcpy_s(receipts[index].digest,p.digest);receipts[index].kind=probe?2:1;receipt_set(index,1);
    voice_stage->bytes.push_back(0);auto lease=std::make_shared<VoiceLease>(std::move(voice_stage),index,probe);
    std::function<void()> task=[lease](){_InterlockedIncrement(&active_callbacks);__try {voice_on_task(lease.get());}__finally {_InterlockedDecrement(&active_callbacks);}};
    TestConfig config{};config.size=sizeof(config);config.pid=GetCurrentProcessId();
    const auto status=schedule_task(config,&task);if(status)receipt_set(index,0);
    AcquireSRWLockShared(&receipt_lock);p.receipt=receipts[index].evidence;ReleaseSRWLockShared(&receipt_lock);return status;
}
