// ABI15: stage only member IDs; the final packet carries the approved text.
struct MentionStage {
    WercPacket packet;
    explicit MentionStage(const WercPacket& p):packet(p){}
    ~MentionStage(){SecureZeroMemory(&packet,sizeof(packet));}
};
static std::unique_ptr<MentionStage> mention_stage;
static bool mention_binding_matches(const WercPacket& a,const WercPacket& b) {
    return !std::memcmp(a.target,b.target,128)&&!std::memcmp(a.attempt,b.attempt,64)&&
        !std::memcmp(a.digest,b.digest,65)&&a.deadline==b.deadline;
}
static void mention_probe_on_task(const SendLease* lease) {
    const auto& p=lease->packet;
    if(closing||uint64_t(std::time(nullptr))>=p.deadline||WaitForSingleObject(controller,0)!=WAIT_TIMEOUT||!same_account(p.account)||!target_allowed(p)) {receipt_set(lease->index,4);return;}
    SharedMessage message{};TestResult report{};uint32_t evidence=4;
    __try {
        // No builder/service/manager/send invocation. Validate the real factory's
        // text layout and allocator; this is not a recipient delivery check.
        using Factory=SharedMessage*(__fastcall*)(SharedMessage*);
        reinterpret_cast<Factory>(core_address(core_base,0x6f4c70))(&message);
        MentionScope scope{lease->members,false};apply_mention(&message,&scope);
        auto object=static_cast<unsigned char*>(message.object);
        auto target=reinterpret_cast<SmallString*>(object+0xb0);
        auto body=reinterpret_cast<SmallString*>(object+0x758);
        if(target->length || target->capacity!=15 || body->length || body->capacity!=15)__leave;
        if(!set_native_string(target,p.target)||!set_native_string(body,p.text)||!native_text_equals(target,p.target)||!native_text_equals(body,p.text))__leave;
        if(!closing&&uint64_t(std::time(nullptr))<p.deadline&&same_account(p.account)&&target_allowed(p))evidence=15;
    } __except(record_exception(GetExceptionInformation(),&report)) {}
    guarded_release_shared(&message,&report);receipt_set(lease->index,evidence);
}
static DWORD mention_dispatch(WercPacket& p) {
    const auto now=uint64_t(std::time(nullptr));const auto op=static_cast<WercOp>(p.operation);
    if(!target_allowed(p)||!mention_target_valid(p.target)||!ascii_id(p.attempt,64)||!digest_valid(p.digest)||p.deadline<=now||p.deadline>now+300)return ERROR_INVALID_PARAMETER;
    if(op==WercOp::MentionStage) {
        if(live_tasks||live_completions)return ERROR_BUSY;
        if(!mention_members_valid(p.text))return ERROR_INVALID_PARAMETER;
        // Same-attempt restaging cannot change the selection even before send.
        if(mention_stage&&!std::strcmp(mention_stage->packet.attempt,p.attempt)&&
           (!mention_binding_matches(mention_stage->packet,p)||std::strcmp(mention_stage->packet.text,p.text)))return ERROR_INVALID_DATA;
        mention_stage=std::make_unique<MentionStage>(p);return 0;
    }
    if(op!=WercOp::MentionSend&&op!=WercOp::MentionProbe)return ERROR_INVALID_FUNCTION;
    const auto n=strnlen_s(p.text,1024);
    if(!n||n==1024||!MultiByteToWideChar(CP_UTF8,MB_ERR_INVALID_CHARS,p.text,int(n),nullptr,0))return ERROR_INVALID_PARAMETER;
    const bool probe=op==WercOp::MentionProbe;const uint32_t kind=probe?12:11;
    for(size_t i=0;i<receipt_count;++i)if(!std::strcmp(receipts[i].attempt,p.attempt)) {
        if(receipts[i].kind!=kind||std::strcmp(receipts[i].digest,p.digest)||std::strcmp(receipts[i].target,p.target))return ERROR_INVALID_DATA;
        AcquireSRWLockShared(&receipt_lock);p.receipt=receipts[i].evidence;ReleaseSRWLockShared(&receipt_lock);
        if(mention_stage&&!std::strcmp(mention_stage->packet.attempt,p.attempt))mention_stage.reset();return 0;
    }
    if(!mention_stage||!mention_binding_matches(mention_stage->packet,p))return ERROR_INVALID_STATE;
    if(live_tasks||live_completions||!reserve_receipt())return ERROR_BUSY;
    if(!probe){const auto status=ensure_mention_factory();if(status)return status;}
    const size_t index=receipt_count++;
    strcpy_s(receipts[index].attempt,p.attempt);strcpy_s(receipts[index].digest,p.digest);strcpy_s(receipts[index].target,p.target);receipts[index].kind=kind;receipt_set(index,1);
    auto lease=std::make_shared<SendLease>(p,index);strcpy_s(lease->members,mention_stage->packet.text);mention_stage.reset();
    std::function<void()> task=[lease,probe](){
        _InterlockedIncrement(&active_callbacks);
        __try {if(probe)mention_probe_on_task(lease.get());else guarded_native_send(lease.get());}
        __finally {_InterlockedDecrement(&active_callbacks);}
    };
    TestConfig config{};config.size=sizeof(config);config.pid=GetCurrentProcessId();
    const auto status=schedule_task(config,&task);if(status)receipt_set(index,0);
    AcquireSRWLockShared(&receipt_lock);p.receipt=receipts[index].evidence;ReleaseSRWLockShared(&receipt_lock);return status;
}
