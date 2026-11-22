// Text and canonical PNG images, with explicit public/private visibility. The request digest, UTF-8 digest, size and visibility are
// immutable throughout staging. No stage/chunk operation schedules a publication.
struct MomentStage {
    WercPacket packet{};
    std::vector<unsigned char> bytes;
    std::vector<std::unique_ptr<ImageStage>> images;
    std::unique_ptr<ImageStage> incoming;
    std::unique_ptr<FileStage> video;bool video_complete=false;
    ~MomentStage(){if(!bytes.empty())SecureZeroMemory(bytes.data(),bytes.size());SecureZeroMemory(&packet,sizeof(packet));}
};
static std::unique_ptr<MomentStage> moment_stage;
static bool moment_input(const WercPacket& p) {
    const auto now=uint64_t(std::time(nullptr));
    return (core_profile_id==1||(core_profile_id==5||core_profile_id==14))&&binding.account[0]&&!std::strcmp(p.target,binding.account)&&
        ascii_id(p.attempt,64)&&digest_valid(p.digest)&&p.deadline>now&&p.deadline<=now+300&&
        (p.dropped==1||p.dropped==2)&&p.message_id<=10000;
}
static bool moment_binding(const WercPacket& a,const WercPacket& b) {
    return mention_binding_matches(a,b)&&a.message_id==b.message_id&&a.dropped==b.dropped&&a.receipt==b.receipt;
}
static void moment_receipt(WercPacket& p,size_t index) {
    AcquireSRWLockShared(&receipt_lock);
    const auto& r=receipts[index];p.receipt=r.evidence;p.dropped=r.native_duration;p.message_id=r.message_id;p.present=r.kind;
    ReleaseSRWLockShared(&receipt_lock);
}
#include "moments-image-stage.inc"
#include "moments-video-stage.inc"
static DWORD moments_dispatch(WercPacket& p) {
    if(p.operation>=60&&p.operation<=62)return moment_video_stage_dispatch(p);
    if(p.operation>=54&&p.operation<=56)return moment_image_stage_dispatch(p);
    if(!moment_input(p))return ERROR_INVALID_PARAMETER;
    if(p.operation==50){
        if(p.receipt>1||(p.receipt==1&&p.present!=1)||!digest_valid(p.text)||p.present>9||(!p.message_id&&!p.present)||p.sequence)return ERROR_INVALID_PARAMETER;
        if(live_tasks||live_completions)return ERROR_BUSY;
        if(moment_stage&&!std::strcmp(moment_stage->packet.attempt,p.attempt))
            return moment_binding(moment_stage->packet,p)&&moment_stage->packet.present==p.present&&!std::memcmp(moment_stage->packet.text,p.text,65)?0:ERROR_INVALID_DATA;
        moment_stage=std::make_unique<MomentStage>();moment_stage->packet=p;moment_stage->bytes.reserve(size_t(p.message_id));return 0;
    }
    if(p.operation==51){
        if(!moment_stage||!moment_binding(moment_stage->packet,p))return ERROR_INVALID_STATE;
        auto& bytes=moment_stage->bytes;
        if(p.sequence!=bytes.size()||!p.present||p.present>1024||p.present>p.message_id-bytes.size())return ERROR_INVALID_PARAMETER;
        bytes.insert(bytes.end(),p.text,p.text+p.present);return 0;
    }
    const bool execute=p.operation==53;
    if((!execute&&p.operation!=52)||p.present||p.sequence)return ERROR_INVALID_PARAMETER;
    const uint32_t kind=execute?21:20;
    for(size_t i=0;i<receipt_count;++i)if(!std::strcmp(receipts[i].attempt,p.attempt)){
        if(receipts[i].kind!=kind||std::strcmp(receipts[i].target,p.target)||std::strcmp(receipts[i].digest,p.digest))return ERROR_INVALID_DATA;
        moment_receipt(p,i);return 0;
    }
    if(!moment_stage||!moment_binding(moment_stage->packet,p))return ERROR_INVALID_STATE;
    const auto& bytes=moment_stage->bytes;
    if((moment_stage->packet.receipt==1 && (!moment_stage->video || !moment_stage->video_complete)) ||
       bytes.size()!=p.message_id || moment_stage->incoming || moment_stage->images.size()!=moment_stage->packet.present ||
       (bytes.empty()&&moment_stage->images.empty()) ||
       (!bytes.empty()&&(std::memchr(bytes.data(),0,bytes.size()) ||
        !MultiByteToWideChar(CP_UTF8,MB_ERR_INVALID_CHARS,reinterpret_cast<const char*>(bytes.data()),int(bytes.size()),nullptr,0))) ||
       (bytes.empty()?std::strcmp(moment_stage->packet.text,"e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"):!voice_hash(bytes,moment_stage->packet.text)))return ERROR_INVALID_DATA;
    if(live_tasks||live_completions||!reserve_receipt())return ERROR_BUSY;
    if(!wxbot_moments_candidate::anchors_match())return ERROR_REVISION_MISMATCH;
    MomentsConfig config{};config.base.pid=GetCurrentProcessId();config.base.size=sizeof(config);
    strcpy_s(config.self_wxid,p.account);if(!bytes.empty())std::memcpy(config.text,bytes.data(),bytes.size());config.image_count=uint32_t(moment_stage->images.size());config.visibility=p.dropped;config.timeout_ms=30000;
    auto lease=std::make_shared<wxbot_moments_candidate::Lease>(config,execute);
    SecureZeroMemory(&config,sizeof(config));
    const auto index=receipt_count++;auto& slot=receipts[index];slot={};
    strcpy_s(slot.attempt,p.attempt);strcpy_s(slot.target,p.target);strcpy_s(slot.digest,p.digest);slot.kind=kind;receipt_set(index,1);
    lease->receipt_index=index;lease->expires=p.deadline;lease->images=std::move(moment_stage->images);lease->video=std::move(moment_stage->video);moment_stage.reset();
    std::function<void()> task=[lease](){wxbot_moments_candidate::on_task(lease);};
    wxbot_moments_candidate::retain_pending(lease);
    const auto status=schedule_task(lease->config.base,&task);
    if(status&&_InterlockedCompareExchange(&lease->started,-1,0)==0){
        // The scheduler might own a copied callable; keep the lease/code resident.
        _InterlockedExchange(&lease->quarantine,1);receipt_set(index,0);
    }
    moment_receipt(p,index);return status;
}
