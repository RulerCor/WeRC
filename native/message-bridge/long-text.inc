// ABI16: bounded UTF-8 staging. Upload and construction never submit a message.
struct LongTextStage {
    WercPacket packet{};
    char members[1024]{};
    std::vector<unsigned char> bytes;
    ~LongTextStage(){if(!bytes.empty())SecureZeroMemory(bytes.data(),bytes.size());SecureZeroMemory(members,sizeof(members));SecureZeroMemory(&packet,sizeof(packet));}
};
static std::unique_ptr<LongTextStage> long_text_stage;
static bool long_text_binding(const WercPacket& a,const WercPacket& b) {
    return mention_binding_matches(a,b)&&a.message_id==b.message_id&&a.dropped==b.dropped;
}
static bool long_text_complete(const LongTextStage& stage) {
    const auto& bytes=stage.bytes;
    return bytes.size()==stage.packet.message_id && !bytes.empty() &&
        !std::memchr(bytes.data(),0,bytes.size()) &&
        MultiByteToWideChar(CP_UTF8,MB_ERR_INVALID_CHARS,reinterpret_cast<const char*>(bytes.data()),static_cast<int>(bytes.size()),nullptr,0) &&
        voice_hash(bytes,stage.packet.text);
}
static void long_text_probe_on_task(const SendLease* lease) {
    const auto& p=lease->packet;
    if(closing||uint64_t(std::time(nullptr))>=p.deadline||WaitForSingleObject(controller,0)!=WAIT_TIMEOUT||!same_account(p.account)||!target_allowed(p)){receipt_set(lease->index,4);return;}
    SharedMessage message{};TestResult report{};SmallString owned{};owned.capacity=15;uint32_t evidence=4;
    __try {
        // Exercise both request destruction and the native object's destructor.
        if(!set_native_body(&owned,lease->body)||!native_text_equals(&owned,lease->body))__leave;
        free_native_string(&owned);
        using Factory=SharedMessage*(__fastcall*)(SharedMessage*);
        reinterpret_cast<Factory>(core_address(core_base,0x6f4c70))(&message);
        auto object=static_cast<unsigned char*>(message.object);
        if(!object||!message.control||*reinterpret_cast<uintptr_t*>(object)!=core_address(core_base,0x8e53588))__leave;
        if(lease->members[0]){MentionScope scope{lease->members,false};apply_mention(&message,&scope);}
        auto target=reinterpret_cast<SmallString*>(object+0xb0);
        auto body=reinterpret_cast<SmallString*>(object+0x758);
        if(!set_native_string(target,p.target)||!set_native_body(body,lease->body)||!native_text_equals(target,p.target)||!native_text_equals(body,lease->body))__leave;
        if(!closing&&uint64_t(std::time(nullptr))<p.deadline&&same_account(p.account)&&target_allowed(p))evidence=16;
    } __except(record_exception(GetExceptionInformation(),&report)) {}
    __try {free_native_string(&owned);} __except(record_exception(GetExceptionInformation(),&report)) {}
    guarded_release_shared(&message,&report);
    receipt_set(lease->index,report.exception?4:evidence);
}
static DWORD long_text_dispatch(WercPacket& p) {
    const auto now=uint64_t(std::time(nullptr));const auto op=static_cast<WercOp>(p.operation);
    if(!target_allowed(p)||!ascii_id(p.attempt,64)||!digest_valid(p.digest)||p.deadline<=now||p.deadline>now+300||
       p.message_id<1024||p.message_id>10000||p.dropped>1||(p.dropped&&!mention_target_valid(p.target)))return ERROR_INVALID_PARAMETER;
    if(op==WercOp::TextBegin){
        if(live_tasks||live_completions)return ERROR_BUSY;
        if(!digest_valid(p.text))return ERROR_INVALID_PARAMETER;
        if(long_text_stage&&!std::strcmp(long_text_stage->packet.attempt,p.attempt)) {
            if(!long_text_binding(long_text_stage->packet,p)||std::strcmp(long_text_stage->packet.text,p.text))return ERROR_INVALID_DATA;
            return 0; // An identical begin does not rewind uploaded bytes.
        }
        if(p.dropped&&(!mention_stage||!mention_binding_matches(mention_stage->packet,p)))return ERROR_INVALID_STATE;
        auto stage=std::make_unique<LongTextStage>();stage->packet=p;stage->bytes.reserve(static_cast<size_t>(p.message_id));
        if(p.dropped){strcpy_s(stage->members,mention_stage->packet.text);mention_stage.reset();}
        long_text_stage=std::move(stage);return 0;
    }
    if(op==WercOp::TextChunk){
        if(!long_text_stage||!long_text_binding(long_text_stage->packet,p))return ERROR_INVALID_STATE;
        auto& bytes=long_text_stage->bytes;
        if(p.sequence!=bytes.size()||!p.present||p.present>sizeof(p.text)||p.present>p.message_id-bytes.size())return ERROR_INVALID_PARAMETER;
        bytes.insert(bytes.end(),p.text,p.text+p.present);return 0;
    }
    if(op!=WercOp::TextSend&&op!=WercOp::TextProbe)return ERROR_INVALID_FUNCTION;
    const bool probe=op==WercOp::TextProbe;const uint32_t kind=(probe?15:13)+p.dropped;
    for(size_t i=0;i<receipt_count;++i)if(!std::strcmp(receipts[i].attempt,p.attempt)){
        if(receipts[i].kind!=kind||std::strcmp(receipts[i].digest,p.digest)||std::strcmp(receipts[i].target,p.target))return ERROR_INVALID_DATA;
        AcquireSRWLockShared(&receipt_lock);p.receipt=receipts[i].evidence;ReleaseSRWLockShared(&receipt_lock);
        if(long_text_stage&&!std::strcmp(long_text_stage->packet.attempt,p.attempt))long_text_stage.reset();return 0;
    }
    if(!long_text_stage||!long_text_binding(long_text_stage->packet,p)||!long_text_complete(*long_text_stage))return ERROR_INVALID_STATE;
    if(live_tasks||live_completions||!reserve_receipt())return ERROR_BUSY;
    if(!probe&&p.dropped){const auto status=ensure_mention_factory();if(status)return status;}
    const size_t index=receipt_count++;
    strcpy_s(receipts[index].attempt,p.attempt);strcpy_s(receipts[index].digest,p.digest);strcpy_s(receipts[index].target,p.target);receipts[index].kind=kind;receipt_set(index,1);
    auto lease=std::make_shared<SendLease>(p,index);
    std::memcpy(lease->body,long_text_stage->bytes.data(),long_text_stage->bytes.size());
    strcpy_s(lease->members,long_text_stage->members);long_text_stage.reset();
    std::function<void()> task=[lease,probe](){
        _InterlockedIncrement(&active_callbacks);
        __try {if(probe)long_text_probe_on_task(lease.get());else guarded_native_send(lease.get());}
        __finally {_InterlockedDecrement(&active_callbacks);}
    };
    TestConfig config{};config.size=sizeof(config);config.pid=GetCurrentProcessId();
    const auto status=schedule_task(config,&task);if(status)receipt_set(index,0);
    AcquireSRWLockShared(&receipt_lock);p.receipt=receipts[index].evidence;ReleaseSRWLockShared(&receipt_lock);return status;
}
