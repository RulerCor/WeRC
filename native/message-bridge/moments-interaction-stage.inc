// One authenticated operation per outbox attempt. Tick only polls or cancels;
// it never creates another operation. The native bool is NOT server evidence.
static bool interaction_tick(bool cancel) {
    using namespace wxbot_sns_interaction;
    const auto lease=get_pending();
    if(!lease)return false;
    PollResult state{};state.size=sizeof(state);state.operation_id=lease->config.operation_id;
    if(cancel)WxMomentsInteractionCancel(&state);
    const auto status=WxMomentsInteractionPoll(&state);
    if(status==ERROR_BUSY)return true;
    const size_t index=size_t(state.operation_id-1);
    if(index<receipt_count){
        AcquireSRWLockExclusive(&receipt_lock);
        auto& slot=receipts[index];
        slot.native_duration=state.state;
        slot.evidence=state.attempted?(state.native_known&&state.native_value?1u:0u):
            (state.state==FOUND?22u:
             state.state==NOT_FOUND||state.state==FAILED||state.state==CANCELED?4u:
             state.state==QUARANTINED?0u:1u);
        ReleaseSRWLockExclusive(&receipt_lock);
    }
    if(state.state==QUARANTINED)return false; // retain uncertain ownership; no unload
    if(WxMomentsInteractionClose(&state)==ERROR_SUCCESS)return false;
    return true;
}
static DWORD interaction_dispatch(WercPacket& p) {
    const auto now=uint64_t(std::time(nullptr));
    const bool probe=p.operation==59;
    const bool reply=!probe && p.dropped==5;
    if((core_profile_id!=1&&(core_profile_id!=5&&core_profile_id!=14))||std::strcmp(p.target,binding.account)||
       !ascii_id(p.attempt,64)||!digest_valid(p.digest)||!p.message_id||
       p.deadline<=now||p.deadline>now+300||(!reply&&(p.sequence||p.present))||
       (probe?p.dropped!=0:(p.dropped<1||p.dropped>5)))return ERROR_INVALID_PARAMETER;
    const auto length=strnlen_s(p.text,sizeof(p.text));
    if((p.dropped==2||reply)?(!length||length>=(reply?896:sizeof(p.text))||
       !MultiByteToWideChar(CP_UTF8,MB_ERR_INVALID_CHARS,p.text,int(length),nullptr,0)):length)
        return ERROR_INVALID_PARAMETER;
    if(reply&&((!p.sequence&&!p.present)||strnlen_s(p.text+896,128)>=128||
        !target_id_valid(p.text+896)||std::strchr(p.text+896,'@')))return ERROR_INVALID_PARAMETER;
    const uint32_t kind=probe?22:23;
    for(size_t i=0;i<receipt_count;++i)if(!std::strcmp(receipts[i].attempt,p.attempt)){
        if(receipts[i].kind!=kind||std::strcmp(receipts[i].target,p.target)||
           std::strcmp(receipts[i].digest,p.digest))return ERROR_INVALID_DATA;
        moment_receipt(p,i);return 0;
    }
    if(live_tasks||live_completions||!reserve_receipt())return ERROR_BUSY;
    if(!wxbot_sns_interaction::anchors_match()||
       (!probe&&!sns_interaction_call::fingerprint_anchors(p.dropped)))
        return ERROR_REVISION_MISMATCH;
    wxbot_sns_interaction::Config cfg{};
    cfg.base.size=sizeof(cfg);cfg.base.pid=GetCurrentProcessId();
    strcpy_s(cfg.account,p.account);cfg.sns_id=p.message_id;
    cfg.timeout_ms=uint32_t(std::min<uint64_t>(30000,(p.deadline-now)*1000));
    cfg.action=p.dropped;strcpy_s(cfg.content,p.text);
    if(reply){cfg.reply_id=p.present;cfg.reply_id64=p.sequence;strcpy_s(cfg.reply_user,p.text+896);}
    const auto index=receipt_count++;auto& slot=receipts[index];slot={};
    strcpy_s(slot.attempt,p.attempt);strcpy_s(slot.target,p.target);strcpy_s(slot.digest,p.digest);
    slot.kind=kind;slot.evidence=1;cfg.operation_id=index+1;
    const auto status=wxbot_sns_interaction::WxMomentsInteractionStart(&cfg);
    SecureZeroMemory(&cfg,sizeof(cfg));
    if(status){
        // If a lease exists the scheduler may already own the call. Never retry.
        slot.evidence=wxbot_sns_interaction::get_pending()?0:4;
    }
    interaction_tick(false);moment_receipt(p,index);
    return 0; // the receipt carries rejection/uncertainty after attempt reservation
}

