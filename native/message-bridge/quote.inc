// Profile 1 quote construction. Adapted from helper/native/quote_send.h under
// its noncommercial license. Source identity is checked before construction.
struct QuoteStage { WercPacket packet{}; std::vector<unsigned char> bytes;
    ~QuoteStage(){if(!bytes.empty())SecureZeroMemory(bytes.data(),bytes.size());SecureZeroMemory(&packet,sizeof(packet));} };
static std::unique_ptr<QuoteStage> quote_stage;
static bool quote_binding(const WercPacket& a,const WercPacket& b) {
    return mention_binding_matches(a,b)&&a.message_id==b.message_id&&a.dropped==b.dropped;
}
struct QuoteLease {
    WercPacket packet; size_t index; char source_hash[65]{},body[10001]{};
    QuoteLease(const WercPacket& p,size_t i,const char* hash):packet(p),index(i){
        strcpy_s(source_hash,hash);_InterlockedIncrement(&live_tasks);
    }
    ~QuoteLease(){SecureZeroMemory(&packet,sizeof(packet));SecureZeroMemory(source_hash,sizeof(source_hash));SecureZeroMemory(body,sizeof(body));_InterlockedDecrement(&live_tasks);}
};
static bool quote_body_hash(const SmallString* text,const char* expected) {
    if(!text->length||text->length>10000||text->capacity<text->length||text->capacity>1048576)return false;
    const auto bytes=text->capacity<16?text->data:*reinterpret_cast<const char*const*>(text->data);
    if(!bytes)return false;
    // Use CNG directly: no C++ allocation/destruction across SEH.
    BCRYPT_ALG_HANDLE algorithm=nullptr;BCRYPT_HASH_HANDLE hash=nullptr;unsigned char actual[32]{};
    if(BCryptOpenAlgorithmProvider(&algorithm,BCRYPT_SHA256_ALGORITHM,nullptr,0)<0)return false;
    bool ok=BCryptCreateHash(algorithm,&hash,nullptr,0,nullptr,0,0)>=0;
    if(ok)ok=BCryptHashData(hash,reinterpret_cast<PUCHAR>(const_cast<char*>(bytes)),static_cast<ULONG>(text->length),0)>=0&&BCryptFinishHash(hash,actual,32,0)>=0;
    if(hash)BCryptDestroyHash(hash);BCryptCloseAlgorithmProvider(algorithm,0);
    const char hex[]="0123456789abcdef";unsigned char diff=0;
    for(size_t i=0;i<32;++i){diff|=expected[i*2]^hex[actual[i]>>4];diff|=expected[i*2+1]^hex[actual[i]&15];}
    SecureZeroMemory(actual,sizeof(actual));return ok&&!diff;
}
using QuoteService=SharedMessage*(__fastcall*)(void*,SharedMessage*);
static QuoteService original_quote_service=nullptr;
static SharedMessage* __fastcall quote_service(void* provider,SharedMessage* output) {
    _InterlockedIncrement(&active_callbacks);
    __try {
        const auto scope=factory_tls==TLS_OUT_OF_INDEXES?nullptr:static_cast<MentionScope*>(TlsGetValue(factory_tls));
        if(scope&&scope->quote&&reinterpret_cast<uintptr_t>(_ReturnAddress())==core_address(core_base,0x54b07e5)){
            auto& quote=*scope->quote;
            if(quote.prepared.object||!quote.object||quote.applied||
               *reinterpret_cast<uint64_t*>(quote.object+0x118)!=((57ull<<32)|1))
                RaiseException(ERROR_INVALID_DATA,0,0,nullptr);
            *reinterpret_cast<uint64_t*>(quote.object+0x118)=(57ull<<32)|49;
            quote.applied=true;
            AcquireSRWLockExclusive(&receipt_lock);receipts[quote.receipt_index].native_duration=1;ReleaseSRWLockExclusive(&receipt_lock);
        }
        return original_quote_service(provider,output);
    } __finally {_InterlockedDecrement(&active_callbacks);}
}
static DWORD ensure_quote_factory(){
    auto status=ensure_mention_factory();if(status||quote_manager_hook_created)return status;
    auto target=reinterpret_cast<void*>(core_address(core_base,0x6f4d90));
    auto hook=MH_CreateHook(target,reinterpret_cast<void*>(&quote_service),reinterpret_cast<void**>(&original_quote_service));
    if(hook==MH_OK){hook=MH_EnableHook(target);if(hook!=MH_OK)MH_RemoveHook(target);}
    if(hook!=MH_OK)return 600+hook;quote_manager_hook_created=true;return 0;
}
static void quote_on_task(const QuoteLease* lease,SendLease* send) {
    const auto& p=lease->packet;
    if(closing||uint64_t(std::time(nullptr))>=p.deadline||WaitForSingleObject(controller,0)!=WAIT_TIMEOUT||!same_account(p.account)||!target_allowed(p)){receipt_set(lease->index,4);return;}
    alignas(16) unsigned char content[0x2f0]{},source[0x440]{},extra[0x68]{};
    SharedMessage app{},provider{},manager{};QuoteScope quote{};quote.receipt_index=lease->index;
    SmallString target{},reply{};target.capacity=reply.capacity=15;
    bool content_ready=false,source_ready=false,extra_ready=false;
    uint32_t stage=80,evidence=4;TestResult report{};bool submitted=false;
    __try {
        using GetContext=SharedMessage*(__fastcall*)(SharedMessage*);
        using GetService=SharedMessage*(__fastcall*)(void*,SharedMessage*);
        using Query=void*(__fastcall*)(void*,void*,const SmallString*,uint64_t,uint32_t);
        using Describe=void*(__fastcall*)(const void*,void*);
        using Extra=void*(__fastcall*)(void*);
        using Construct=SharedMessage*(__fastcall*)(SharedMessage*,void*,const SmallString*,void*);
        reinterpret_cast<GetContext>(core_address(core_base,0x47220))(&app);
        if(!app.object||!same_account(p.account))__leave;
        reinterpret_cast<GetService>(core_address(core_base,0x36c740))(app.object,&provider);
        if(!provider.object)__leave;
        reinterpret_cast<GetService>(core_address(core_base,0x6f4d90))(provider.object,&manager);
        if(!manager.object||!set_native_string(&target,p.target))__leave;
        stage=81;
        reinterpret_cast<Query>(core_address(core_base,0x17c5260))(manager.object,content,&target,p.message_id,0);
        content_ready=content[0x2e8]==1;
        if(!content_ready)__leave;
        stage=82;
        if(*reinterpret_cast<uintptr_t*>(content)!=core_address(core_base,0x8d9f958))__leave;
        reinterpret_cast<Describe>(core_address(core_base,0xa58dd0))(content,source);source_ready=true;
        stage=83;
        if(*reinterpret_cast<uintptr_t*>(source)!=core_address(core_base,0x8d9e9a8)||
           *reinterpret_cast<uint32_t*>(source+8)!=1||
           *reinterpret_cast<uint64_t*>(source+0xb8)!=p.message_id||
           !native_text_equals(reinterpret_cast<SmallString*>(source+0x70),p.target))__leave;
        stage=84;
        if(!quote_body_hash(reinterpret_cast<SmallString*>(source+0xe0),lease->source_hash))__leave;
        stage=85;
        if(!set_native_body(&reply,lease->body))__leave;
        reinterpret_cast<Extra>(core_address(core_base,0x1a5fc50))(extra);extra_ready=true;
        reinterpret_cast<Construct>(core_address(core_base,0x3995970))(&quote.prepared,source,&reply,extra);
        stage=86;
        auto object=static_cast<unsigned char*>(quote.prepared.object);quote.object=object;
        if(!object||!quote.prepared.control||*reinterpret_cast<uintptr_t*>(object)!=core_address(core_base,0x8e53588)||
           *reinterpret_cast<uint64_t*>(object+0x118)!=((57ull<<32)|49)||
           *reinterpret_cast<uint64_t*>(object+0x300)!=p.message_id||
           !native_text_equals(reinterpret_cast<SmallString*>(object+0x758),lease->body)||
           !set_native_string(reinterpret_cast<SmallString*>(object+0xb0),p.target))__leave;
        stage=87;
        if(!closing&&uint64_t(std::time(nullptr))<p.deadline&&same_account(p.account)&&target_allowed(p)){
            if(send){submitted=true;native_send(send,&quote);}else evidence=17;
        }
    } __except(record_exception(GetExceptionInformation(),&report)) {}
    using Destroy=void(__fastcall*)(void*);
    guarded_release_shared(&quote.prepared,&report);
    __try {if(extra_ready)reinterpret_cast<Destroy>(core_address(core_base,0xe9e920))(extra);} __except(record_exception(GetExceptionInformation(),&report)) {}
    __try {if(source_ready)reinterpret_cast<Destroy>(core_address(core_base,0x1aee40))(source);} __except(record_exception(GetExceptionInformation(),&report)) {}
    __try {if(content_ready)reinterpret_cast<Destroy>(core_address(core_base,0x1c50f0))(content);} __except(record_exception(GetExceptionInformation(),&report)) {}
    __try {free_native_string(&reply);free_native_string(&target);} __except(record_exception(GetExceptionInformation(),&report)) {}
    guarded_release_shared(&manager,&report);guarded_release_shared(&provider,&report);guarded_release_shared(&app,&report);
    if(!send){AcquireSRWLockExclusive(&receipt_lock);receipts[lease->index].native_duration=stage;ReleaseSRWLockExclusive(&receipt_lock);}
    if(report.exception)receipt_set(lease->index,submitted?0:4);
    else if(!submitted)receipt_set(lease->index,evidence);
}
static DWORD quote_dispatch(WercPacket& p) {
    const auto now=uint64_t(std::time(nullptr));
    if(!target_allowed(p)||!ascii_id(p.attempt,64)||!digest_valid(p.digest)||!p.message_id||
       p.deadline<=now||p.deadline>now+300||!p.dropped||p.dropped>10000)return ERROR_INVALID_PARAMETER;
    if(p.operation==uint32_t(WercOp::QuoteStage)){
        if(!digest_valid(p.text)||!digest_valid(p.text+65))return ERROR_INVALID_PARAMETER;
        if(live_tasks||live_completions)return ERROR_BUSY;
        if(quote_stage&&!std::strcmp(quote_stage->packet.attempt,p.attempt)){
            return quote_binding(quote_stage->packet,p)&&!std::memcmp(quote_stage->packet.text,p.text,130)?0:ERROR_INVALID_DATA;
        }
        quote_stage=std::make_unique<QuoteStage>();quote_stage->packet=p;quote_stage->bytes.reserve(p.dropped);return 0;
    }
    if(p.operation==uint32_t(WercOp::QuoteChunk)){
        if(!quote_stage||!quote_binding(quote_stage->packet,p))return ERROR_INVALID_STATE;
        auto& bytes=quote_stage->bytes;
        if(p.sequence!=bytes.size()||!p.present||p.present>1024||p.present>p.dropped-bytes.size())return ERROR_INVALID_PARAMETER;
        bytes.insert(bytes.end(),p.text,p.text+p.present);return 0;
    }
    const bool probe=p.operation==uint32_t(WercOp::QuoteProbe);
    if(!probe&&p.operation!=uint32_t(WercOp::QuoteSend))return ERROR_INVALID_FUNCTION;
    const uint32_t kind=probe?17:18;
    for(size_t i=0;i<receipt_count;++i)if(!std::strcmp(receipts[i].attempt,p.attempt)){
        if(receipts[i].kind!=kind||receipts[i].message_id!=p.message_id||std::strcmp(receipts[i].target,p.target)||std::strcmp(receipts[i].digest,p.digest))return ERROR_INVALID_DATA;
        AcquireSRWLockShared(&receipt_lock);p.receipt=receipts[i].evidence;p.dropped=receipts[i].native_duration;ReleaseSRWLockShared(&receipt_lock);return 0;
    }
    if(!quote_stage||!quote_binding(quote_stage->packet,p))return ERROR_INVALID_STATE;
    const auto& bytes=quote_stage->bytes;
    if(bytes.size()!=p.dropped||std::memchr(bytes.data(),0,bytes.size())||
       !MultiByteToWideChar(CP_UTF8,MB_ERR_INVALID_CHARS,reinterpret_cast<const char*>(bytes.data()),static_cast<int>(bytes.size()),nullptr,0)||
       !voice_hash(bytes,quote_stage->packet.text+65))return ERROR_INVALID_DATA;
    if(live_tasks||live_completions||!reserve_receipt())return ERROR_BUSY;
    if(!probe){const auto status=ensure_quote_factory();if(status)return status;}
    const auto index=receipt_count++;
    auto& slot=receipts[index];strcpy_s(slot.attempt,p.attempt);strcpy_s(slot.digest,p.digest);strcpy_s(slot.target,p.target);slot.kind=kind;slot.message_id=p.message_id;receipt_set(index,1);
    auto lease=std::make_shared<QuoteLease>(p,index,quote_stage->packet.text);
    std::memcpy(lease->body,bytes.data(),bytes.size());quote_stage.reset();
    std::shared_ptr<SendLease> send;
    if(!probe){send=std::make_shared<SendLease>(p,index);std::memcpy(send->body,lease->body,sizeof(lease->body));}
    std::function<void()> task=[lease,send](){_InterlockedIncrement(&active_callbacks);
        __try {quote_on_task(lease.get(),send.get());} __finally {_InterlockedDecrement(&active_callbacks);}
    };
    TestConfig config{};config.size=sizeof(config);config.pid=GetCurrentProcessId();
    const auto result=schedule_task(config,&task);if(result)receipt_set(index,4);
    AcquireSRWLockShared(&receipt_lock);p.receipt=receipts[index].evidence;p.dropped=receipts[index].native_duration;ReleaseSRWLockShared(&receipt_lock);return result;
}
