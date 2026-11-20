    {
        receipt_count=0;clear_receipts();core_profile_id=1;strcpy_s(binding.account,"owned-self");
        WercPacket begin{};begin.operation=50;strcpy_s(begin.account,"owned-self");strcpy_s(begin.target,"owned-self");strcpy_s(begin.attempt,"owned-moment-images");
        std::memset(begin.digest,'a',64);strcpy_s(begin.text,"e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");begin.dropped=2;begin.deadline=uint64_t(std::time(nullptr))+60;
        check(moments_dispatch(begin)==ERROR_INVALID_PARAMETER,"empty text without images rejected");
        begin.present=10;check(moments_dispatch(begin)==ERROR_INVALID_PARAMETER,"maximum nine moment images");
        begin.present=9;check(moments_dispatch(begin)==0&&!live_tasks,"empty text with nine images only stages");
        auto changed=begin;changed.present=8;check(moments_dispatch(changed)==ERROR_INVALID_DATA,"image count cannot change under same request");
        auto finish=begin;finish.operation=52;finish.present=0;
        check(moments_dispatch(finish)==ERROR_INVALID_DATA,"missing images cannot construct or publish");
        auto image=begin;image.operation=54;image.message_id=sizeof(owned_image);image.dropped=2|(2<<16);image.present=0;strcpy_s(image.text,owned_image_sha256);
        image.sequence=1;check(moments_dispatch(image)==ERROR_INVALID_PARAMETER,"moment images ordered from zero");image.sequence=0;
        changed=image;changed.digest[0]='b';check(moments_dispatch(changed)==ERROR_INVALID_STATE,"image binds approved request digest");
        changed=image;changed.deadline--;check(moments_dispatch(changed)==ERROR_INVALID_STATE,"image binds original deadline");
        changed=image;strcpy_s(changed.target,"filehelper");check(moments_dispatch(changed)==ERROR_INVALID_PARAMETER,"image cannot redirect to chat");
        changed=image;changed.message_id=image_limit+1;check(moments_dispatch(changed)==ERROR_INVALID_PARAMETER,"bounded canonical image size");
        for(uint64_t index=0;index<9;++index){
            image.sequence=index;
            check(moments_dispatch(image)==0&&!live_tasks,"image begin does not schedule native work");
            changed=image;changed.dropped=3|(2<<16);check(moments_dispatch(changed)==ERROR_INVALID_DATA,"same image cannot change dimensions");
            auto chunk=image;chunk.operation=55;chunk.sequence=0;chunk.present=sizeof(owned_image);std::memcpy(chunk.text,owned_image,sizeof(owned_image));
            check(moments_dispatch(chunk)==0&&!live_tasks,"canonical bytes stage without native writes");
            check(moments_dispatch(chunk)==ERROR_INVALID_PARAMETER,"duplicate image chunk refused");
            check(moments_dispatch(image)==0&&moment_stage->incoming->bytes.size()==sizeof(owned_image),"image begin does not rewind bytes");
            auto commit=image;commit.operation=56;
            moment_stage->incoming->bytes[20]^=1;check(moments_dispatch(commit)==ERROR_INVALID_DATA,"altered PNG shape or digest rejected");moment_stage->incoming->bytes[20]^=1;
            check(moments_dispatch(commit)==0&&!live_tasks&&moment_stage->images.size()==index+1,"ordered image commit retains exact bytes");
            check(moments_dispatch(commit)==ERROR_INVALID_STATE,"duplicate image commit cannot append twice");
        }
        image.sequence=9;check(moments_dispatch(image)==ERROR_INVALID_PARAMETER,"tenth image refused");
        const auto previous_base=core_base;core_base=0;
        check(moments_dispatch(finish)==ERROR_REVISION_MISMATCH&&!live_tasks,"complete staging still requires exact native anchors");core_base=previous_base;
        moment_stage.reset();check(!live_tasks&&!wxbot_moments_candidate::pending,"all staged image bytes released without native work");
    }
