// ABI23: one MP4 and one separately staged PNG cover per video moment.
static DWORD moment_video_stage_dispatch(WercPacket& p) {
    const auto now=uint64_t(std::time(nullptr));
    if((core_profile_id!=1&&(core_profile_id!=5&&core_profile_id!=14)) || !binding.account[0] || std::strcmp(p.target,binding.account) ||
       !ascii_id(p.attempt,64) || !digest_valid(p.digest) || p.deadline<=now || p.deadline>now+300)
        return ERROR_INVALID_PARAMETER;
    if(!moment_stage || !mention_binding_matches(moment_stage->packet,p) ||
       moment_stage->packet.receipt!=1 || moment_stage->packet.present!=1 ||
       moment_stage->bytes.size()!=moment_stage->packet.message_id || p.receipt!=1)
        return ERROR_INVALID_STATE;
    auto& stage=*moment_stage;
    if(p.operation==60) {
        const auto width=uint32_t(p.sequence)&65535,height=uint32_t(p.sequence)>>16;
        if(p.present || p.sequence>UINT32_MAX || !width || !height || width>4096 || height>4096 ||
           p.message_id<32 || p.message_id>file_limit || p.dropped<1000 || p.dropped>60000 ||
           !digest_valid(p.text))return ERROR_INVALID_PARAMETER;
        if(stage.video) {
            const auto& old=stage.video->packet;
            return old.sequence==p.sequence && old.message_id==p.message_id && old.dropped==p.dropped &&
                !std::memcmp(old.text,p.text,65)?0:ERROR_INVALID_DATA;
        }
        stage.video=std::make_unique<FileStage>();stage.video->packet=p;
        stage.video->bytes.reserve(size_t(p.message_id));return 0;
    }
    if(!stage.video || stage.video->packet.message_id!=p.message_id ||
       stage.video->packet.dropped!=p.dropped)return ERROR_INVALID_STATE;
    auto& video=*stage.video;
    if(p.operation==61) {
        if(stage.video_complete || p.sequence!=video.bytes.size() || !p.present || p.present>1024 ||
           p.present>p.message_id-video.bytes.size())return ERROR_INVALID_PARAMETER;
        video.bytes.insert(video.bytes.end(),p.text,p.text+p.present);return 0;
    }
    if(p.operation!=62 || p.present || p.sequence!=video.packet.sequence)return ERROR_INVALID_PARAMETER;
    if(video.bytes.size()!=p.message_id || !voice_hash(video.bytes,video.packet.text))return ERROR_INVALID_DATA;
    stage.video_complete=true;return 0;
}
