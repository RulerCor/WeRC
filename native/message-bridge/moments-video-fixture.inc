    {
        core_profile_id=1;strcpy_s(binding.account,"owned-self");
        WercPacket begin{};begin.operation=50;begin.receipt=1;begin.present=1;begin.dropped=2;
        strcpy_s(begin.account,"owned-self");strcpy_s(begin.target,"owned-self");strcpy_s(begin.attempt,"owned-video-moment");
        std::memset(begin.digest,'a',64);strcpy_s(begin.text,"e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
        begin.deadline=uint64_t(std::time(nullptr))+60;
        auto bad=begin;bad.present=2;check(moments_dispatch(bad)==ERROR_INVALID_PARAMETER,"video has exactly one cover");
        check(moments_dispatch(begin)==0&&!live_tasks,"video moment begins without native task");
        bad=begin;bad.receipt=0;check(moments_dispatch(bad)==ERROR_INVALID_DATA,"video cannot change to image under same attempt");
        auto video=begin;video.operation=60;video.present=0;video.message_id=sizeof(owned_image);video.sequence=2|(2<<16);video.dropped=1000;
        strcpy_s(video.text,owned_image_sha256);
        check(moments_dispatch(video)==0&&!live_tasks,"video bytes stage without native IO");
        bad=video;bad.dropped=2000;check(moments_dispatch(bad)==ERROR_INVALID_DATA,"staged video duration immutable");
        auto commit=video;commit.operation=62;check(moments_dispatch(commit)==ERROR_INVALID_DATA,"incomplete MP4 cannot commit");
        auto chunk=video;chunk.operation=61;chunk.sequence=0;chunk.present=sizeof(owned_image);std::memcpy(chunk.text,owned_image,sizeof(owned_image));
        check(moments_dispatch(chunk)==0,"bounded video chunk accepted");
        check(moments_dispatch(chunk)==ERROR_INVALID_PARAMETER,"duplicate video bytes rejected");
        check(moments_dispatch(commit)==0&&moment_stage->video_complete,"exact video digest commits once");
        check(moments_dispatch(chunk)==ERROR_INVALID_PARAMETER,"committed bytes immutable");
        auto finish=begin;finish.operation=52;finish.present=0;
        check(moments_dispatch(finish)==ERROR_INVALID_DATA&&!live_tasks,"video without cover cannot call native");
        moment_stage.reset();
    }
