    {
        MomentsConfig cfg{};strcpy_s(cfg.self_wxid,"owned-self");
        auto owned=std::make_shared<wxbot_moments_candidate::Lease>(cfg,false);strcpy_s(owned->client_id,"owned-client");
        alignas(16) unsigned char event_data[0x40]{},model_data[0x2a8]{};uint64_t sns=12345;
        auto cid=reinterpret_cast<SmallString*>(event_data+8);strcpy_s(cid->data,"owned-client");cid->length=12;cid->capacity=15;
        *reinterpret_cast<SmallString*>(model_data+0x40)=*cid;
        *reinterpret_cast<SharedMessage*>(event_data+0x28)={model_data,model_data};
        *reinterpret_cast<SharedMessage*>(model_data+0x280)={&sns,&sns};
        SharedMessage native_event{event_data,event_data};wxbot_moments_candidate::Snapshot snap{};
        check(wxbot_moments_candidate::snapshot_native(owned.get(),&native_event,&snap)==0&&snap.matching&&snap.ret_known&&snap.sns_id==sns&&!snap.account_matches,"completed event layout and identity gate");
        cid->data[0]='x';snap={};check(wxbot_moments_candidate::snapshot_native(owned.get(),&native_event,&snap)==0&&!snap.matching,"unrelated completed event ignored");cid->data[0]='o';
        model_data[0x40]='x';snap={};check(wxbot_moments_candidate::snapshot_native(owned.get(),&native_event,&snap)==ERROR_INVALID_DATA,"returned model client ID must match");model_data[0x40]='o';
        owned->receipt_index=0;owned->accepted=1;owned->report.stage=166;
        wxbot_moments_candidate::write_result(*owned,0,"completed",false,true,0,12345,true);check(receipts[0].evidence==21&&receipts[0].message_id==12345,"completion requires server ID and current account");
        wxbot_moments_candidate::write_result(*owned,0,"completed",false,true,0,0,true);check(receipts[0].evidence!=21,"no server ID remains unconfirmed");
        wxbot_moments_candidate::write_result(*owned,0,"completed",false,true,0,12345,false);check(receipts[0].evidence!=21,"changed account remains unconfirmed");
        owned->submitted=1;wxbot_moments_candidate::write_result(*owned,ERROR_GEN_FAILURE,"submit_unconfirmed",false);check(receipts[0].evidence==0,"attempted publication failure stays unknown");
        owned->submitted=0;wxbot_moments_candidate::write_result(*owned,ERROR_NOT_READY,"prepare_failed",false);check(receipts[0].evidence==4,"pre-submit rejection is distinct");
        owned->deadline_observed=1;wxbot_moments_candidate::write_result(*owned,0,"completed",false,true,0,12345,true);check(receipts[0].evidence!=21,"late completion remains unconfirmed");
    }
    check(!live_tasks,"completed event fixture lease released");

    receipt_count=0;clear_receipts();core_profile_id=1;strcpy_s(binding.account,"owned-self");
    WercPacket mp{};mp.operation=50;strcpy_s(mp.account,"owned-self");strcpy_s(mp.target,"owned-self");strcpy_s(mp.attempt,"owned-moment");std::memset(mp.digest,'a',64);std::memset(mp.text,'b',64);mp.deadline=uint64_t(std::time(nullptr))+60;mp.message_id=10000;mp.dropped=2;
    check(moments_dispatch(mp)==0&&!live_tasks,"moment stage no side effect");
    auto mc=mp;mc.operation=51;mc.present=1024;std::memset(mc.text,'x',sizeof(mc.text));
    mc.sequence=1;check(moments_dispatch(mc)==ERROR_INVALID_PARAMETER,"moment contiguous chunks");mc.sequence=0;
    mc.dropped=1;check(moments_dispatch(mc)==ERROR_INVALID_STATE,"staged visibility immutable");mc.dropped=2;
    mc.digest[0]='c';check(moments_dispatch(mc)==ERROR_INVALID_STATE,"approved request digest bound");mc.digest[0]='a';
    mc.deadline--;check(moments_dispatch(mc)==ERROR_INVALID_STATE,"moment deadline immutable");mc.deadline++;
    strcpy_s(mc.target,"filehelper");check(moments_dispatch(mc)==ERROR_INVALID_PARAMETER,"moment cannot target chat");strcpy_s(mc.target,"owned-self");
    for(size_t offset=0;offset<10000;){mc.sequence=offset;mc.present=uint32_t(std::min<size_t>(1024,10000-offset));check(moments_dispatch(mc)==0&&!live_tasks,"moment bounded chunk without publication");offset+=mc.present;}
    check(moments_dispatch(mc)==ERROR_INVALID_PARAMETER,"duplicate moment chunk refused");
    check(moments_dispatch(mp)==0&&moment_stage->bytes.size()==10000,"begin does not rewind moment");
    auto mf=mp;mf.operation=53;check(moments_dispatch(mf)==ERROR_INVALID_DATA&&!live_tasks,"changed moment body hash never submits");
    moment_stage->bytes[0]=0;check(moments_dispatch(mf)==ERROR_INVALID_DATA,"moment embedded NUL refused");moment_stage->bytes[0]=0xff;check(moments_dispatch(mf)==ERROR_INVALID_DATA,"moment invalid UTF8 refused");
    mf.message_id=10001;check(moments_dispatch(mf)==ERROR_INVALID_PARAMETER,"moment maximum byte bound");mf.message_id=10000;
    receipt_count=1;receipts[0]={};strcpy_s(receipts[0].attempt,mp.attempt);strcpy_s(receipts[0].digest,mp.digest);strcpy_s(receipts[0].target,mp.target);receipts[0].kind=21;receipts[0].evidence=21;receipts[0].message_id=9007199254740993ull;
    check(moments_dispatch(mf)==0&&mf.receipt==21&&mf.present==21&&mf.message_id==9007199254740993ull&&!live_tasks,"duplicate moment returns original receipt without scheduling");
    mf=mp;mf.operation=52;check(moments_dispatch(mf)==ERROR_INVALID_DATA,"probe cannot consume publication attempt");
    core_profile_id=2;check(moments_dispatch(mp)==ERROR_INVALID_PARAMETER,"unverified build refused");core_profile_id=1;
    moment_stage.reset();receipt_count=0;clear_receipts();
    mp.dropped=1;check(moments_dispatch(mp)==0&&!live_tasks,"public staging never publishes");
    mc=mp;mc.operation=51;mc.present=1;mc.text[0]='x';mc.dropped=2;
    check(moments_dispatch(mc)==ERROR_INVALID_STATE,"public stage cannot switch private");
    mc.dropped=3;check(moments_dispatch(mc)==ERROR_INVALID_PARAMETER,"unknown visibility refused");
    moment_stage.reset();check(!live_tasks&&!wxbot_moments_candidate::pending,"moment owned fixtures clean");
