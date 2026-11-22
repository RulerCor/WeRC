// Each ordered PNG is committed before the next begins. No upload operation
// calls Weixin, creates a native cache file, or publishes a moment.
static DWORD moment_image_stage_dispatch(WercPacket& p) {
    const auto now=uint64_t(std::time(nullptr));
    if((core_profile_id!=1&&(core_profile_id!=5&&core_profile_id!=14)) || !binding.account[0] || std::strcmp(p.target,binding.account) ||
       !ascii_id(p.attempt,64) || !digest_valid(p.digest) || p.deadline<=now || p.deadline>now+300)
        return ERROR_INVALID_PARAMETER;
    if(!moment_stage || !mention_binding_matches(moment_stage->packet,p) ||
       moment_stage->bytes.size()!=moment_stage->packet.message_id)return ERROR_INVALID_STATE;
    auto& stage=*moment_stage;
    if(p.operation==54){
        const auto width=p.dropped&65535,height=p.dropped>>16;
        if(p.present || p.sequence!=stage.images.size() || p.sequence>=stage.packet.present ||
           p.message_id<57 || p.message_id>image_limit || !width || !height || width>4096 || height>4096 ||
           uint64_t(width)*height>4194304 || !digest_valid(p.text))return ERROR_INVALID_PARAMETER;
        if(stage.incoming){
            const auto& old=stage.incoming->packet;
            return old.sequence==p.sequence && old.message_id==p.message_id && old.dropped==p.dropped &&
                !std::memcmp(old.text,p.text,65)?0:ERROR_INVALID_DATA;
        }
        stage.incoming=std::make_unique<ImageStage>();stage.incoming->packet=p;
        stage.incoming->bytes.reserve(size_t(p.message_id));return 0;
    }
    if(!stage.incoming || stage.incoming->packet.message_id!=p.message_id ||
       stage.incoming->packet.dropped!=p.dropped)return ERROR_INVALID_STATE;
    auto& input=*stage.incoming;
    if(p.operation==55){
        if(p.sequence!=input.bytes.size() || !p.present || p.present>1024 ||
           p.present>p.message_id-input.bytes.size())return ERROR_INVALID_PARAMETER;
        input.bytes.insert(input.bytes.end(),p.text,p.text+p.present);return 0;
    }
    if(p.operation!=56 || p.present || p.sequence!=input.packet.sequence)return ERROR_INVALID_PARAMETER;
    if(input.bytes.size()!=p.message_id || !image_shape(input.bytes,p.dropped) ||
       !voice_hash(input.bytes,input.packet.text))return ERROR_INVALID_DATA;
    stage.images.push_back(std::move(stage.incoming));return 0;
}
