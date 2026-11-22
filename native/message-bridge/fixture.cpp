// Owned host only. Includes implementation to exercise actual validation,
// attempt reuse, callback evidence and late-task expiry without Weixin.
#include "bridge.cpp"
#include <cstdlib>
#include <string>
#include "fixture-image.h"
#include "compatibility.h"
static unsigned checks=0;
static void check(bool value,const char* name) {
    if(!value) { std::fprintf(stderr,"FAIL %s\n",name);std::exit(1); }
    ++checks;
}
#include "fixture-article.inc"
#include "fixture-profile52.inc"
#include "fixture-profile53.inc"
int main(int argc,char**) {
    if(argc>1) return 0; // Owned short-lived controller for orphan recovery tests.
    check_articles();
    check_profile52();
    check_profile53();
    for(const auto& build:core_builds){
        check(core_select_build(build.bytes,build.sha256,build.timestamp,build.image_size),"settings build exact selection");
        check(core_settings_mask==build.settings_mask,"settings feature selected independently");
        check((build.settings_mask==7)==(build.id==5||build.id==14||(build.sha256[0]==0x10&&build.sha256[1]==0xf8)),"settings research and verified images explicitly selected");
    }
    core_profile_id=0;core_settings_mask=0;
    check(native_settings::validate()==ERROR_NOT_SUPPORTED,"unknown settings build refused");
    {WercPacket s{};s.operation=71;s.deadline=uint64_t(std::time(nullptr))+20;
     s.dropped=0;check(native_settings::dispatch(s)==ERROR_INVALID_PARAMETER,"arbitrary settings id refused");
     s.dropped=1;s.present=2;check(native_settings::dispatch(s)==ERROR_INVALID_PARAMETER,"non boolean setting refused");
     s.present=0;s.sequence=3;check(native_settings::dispatch(s)==ERROR_INVALID_PARAMETER,"invalid expected setting refused");
     s.sequence=2;s.deadline=0;check(native_settings::dispatch(s)==ERROR_INVALID_PARAMETER,"expired setting refused");}

    check(core_variant("10f8e995453e2da46d4f2b5080cd6da1f13cc5147746adc119ceae38cb039de5")==CoreVariant::Original,"original core recognized");
    check(core_variant("ff8fa05d5497c0497765439cc886436868610d10ca3b21131642482311821c4f")==CoreVariant::AntiRecall,"anti recall core recognized");
    check(core_variant("unknown")==CoreVariant::Unsupported,"unknown patch not treated as compatible");
    check(core_variant("88eabf78f961e0abe26b397521b05f4ba6cd752727fb9db74a2b3f660b0400f2")==CoreVariant::WeChat411553,"4.1.15.53 exact core recognized");
    check(core_variant("f4c4118d85ee743a0832c301288edf77b041e02f84b73d7905ffe10090206d01")==CoreVariant::WeChat41137,"4.1.13.7 exact core recognized");
    check(core_variant("afe3d02c58bada3c35009c0cd58ee1f57b2a00fe53237d0dce649107fddb9a20")==CoreVariant::WeChat42013,"4.2.0.13 exact core recognized");
    check(core_install_layout(L"Q:\\中文 空格\\Weixin.exe",L"q:\\中文 空格\\renamed build\\Weixin.dll"),"dynamic renamed build path");
    check(core_install_layout(L"Q:\\portable\\Weixin.exe",L"Q:\\portable\\Weixin.dll"),"colocated core path");
    check(!core_install_layout(L"Q:\\portable\\Weixin.exe",L"Q:\\portable-other\\Weixin.dll"),"sibling prefix rejected");
    check(!core_install_layout(L"Q:\\portable\\Weixin.exe",L"Q:\\portable\\..\\Weixin.dll"),"parent traversal rejected");
    WercPacket p{};p.size=sizeof(p);p.version=WercProtocolVersion;p.pid=GetCurrentProcessId();
    p.process_start=creation_time(GetCurrentProcess());p.operation=uint32_t(WercOp::Inspect);
    check(WercCall(nullptr)==ERROR_INVALID_PARAMETER,"null packet");
    p.size--;check(WercCall(&p)==ERROR_REVISION_MISMATCH,"bad size");p.size++;
    p.version++;check(WercCall(&p)==ERROR_REVISION_MISMATCH,"bad ABI");p.version--;
    p.reserved=1;check(WercCall(&p)==ERROR_REVISION_MISMATCH,"reserved flags");p.reserved=0;
    p.process_start++;check(WercCall(&p)==ERROR_INVALID_PARAMETER,"PID reuse");p.process_start--;
    check(WercCall(&p)==ERROR_REVISION_MISMATCH,"no Weixin loaded");
    p.operation=uint32_t(WercOp::Send);check(WercCall(&p)==ERROR_ACCESS_DENIED,"no binding");
    p.operation=uint32_t(WercOp::Close);check(WercCall(&p)==ERROR_ACCESS_DENIED,"unauthorized close");
    strcpy_s(p.target,"filehelper");strcpy_s(p.text,"owned fixture");strcpy_s(p.attempt,"owned-attempt");
    std::memset(p.digest,'a',64);p.digest[64]=0;p.deadline=uint64_t(std::time(nullptr))+60;
    strcpy_s(p.target,"someone_else");check(submit(p)==ERROR_INVALID_PARAMETER,"target restriction");strcpy_s(p.target,"filehelper");
    p.deadline=0;check(submit(p)==ERROR_INVALID_PARAMETER,"expired before enqueue");p.deadline=uint64_t(std::time(nullptr))+60;
    p.text[0]=-1;check(submit(p)==ERROR_INVALID_PARAMETER,"invalid UTF8");strcpy_s(p.text,"owned fixture");
    p.digest[0]='z';check(submit(p)==ERROR_INVALID_PARAMETER,"invalid digest");p.digest[0]='a';
    std::memset(p.attempt,'x',64);check(submit(p)==ERROR_INVALID_PARAMETER,"unterminated attempt");strcpy_s(p.attempt,"owned-attempt");
    receipt_count=1;strcpy_s(receipts[0].target,p.target);strcpy_s(receipts[0].attempt,p.attempt);strcpy_s(receipts[0].digest,p.digest);receipts[0].evidence=1;
    check(submit(p)==0 && p.receipt==1 && !live_tasks,"duplicate is receipt-only");
    p.digest[0]='b';check(submit(p)==ERROR_INVALID_DATA,"duplicate digest mismatch");p.digest[0]='a';
    // Owned recipient registry exercises actual dispatch guards without native calls.
    WercPacket target=p;strcpy_s(target.target,"owned_long_recipient_identifier_12345@chatroom");
    check(!target_allowed(target),"unprepared group rejected");
    check(prepare_target(target)==0 && target_allowed(target) && !live_tasks,"prepare does not send");
    check(submit(target)==ERROR_INVALID_DATA,"same text attempt cannot change recipient");
    auto long_view=voice_view(target.target,std::strlen(target.target));
    check(long_view.length==std::strlen(target.target)&&long_view.capacity>15&&*reinterpret_cast<const char**>(long_view.data)==target.target,"long voice target points to retained packet");
    auto short_view=voice_view("owned",5);check(short_view.length==5&&short_view.capacity==15&&!std::strcmp(short_view.data,"owned"),"short voice target inline");
    target.deadline=uint64_t(std::time(nullptr))+301;check(prepare_target(target)==ERROR_INVALID_PARAMETER,"target expiry limit");
    target.deadline=uint64_t(std::time(nullptr))+60;
    std::memset(target.target,'x',128);check(prepare_target(target)==ERROR_INVALID_PARAMETER,"unterminated target");
    strcpy_s(target.target,"bad target");check(prepare_target(target)==ERROR_INVALID_PARAMETER,"target excludes spaces");
    clear_targets();
    for(unsigned n=0;n<16;++n){sprintf_s(target.target,"owned-%u@chatroom",n);check(prepare_target(target)==0,"target bounded registry fill");}
    strcpy_s(target.target,"owned-overflow");check(prepare_target(target)==0,"target registry grows beyond sixteen");
    prepared_targets[0].deadline=0;strcpy_s(target.target,"owned-reused");check(prepare_target(target)==0,"expired slot reused");
    auto later=target;later.deadline++;check(!target_allowed(later),"task cannot outlive selected target");
    prepared_targets[0].deadline=0;check(!target_allowed(target),"expired target rejected");
    clear_targets();
    const auto saved_first=receipts[0];
    receipt_count=receipts.size();
    check(reserve_receipt() && receipts.size()>128,"receipt history grows beyond 128");
    check(!std::strcmp(receipts[0].attempt,saved_first.attempt),"growth retains attempt identity");
    receipt_count=1;
    NativeCompletion completion{1,0,completed,destroyed,0};unsigned char success=1;
    completed(&completion,&success);check(receipts[0].evidence==2 && !active_callbacks,"native success stays native evidence");
    check(receipts[0].observed_text_id==0,"callback without owned object cannot invent message identity");
    completed(&completion,nullptr);check(receipts[0].evidence==3 && !active_callbacks,"native failure ambiguous");
    p.deadline=0;
    {SendLease lease(p,0);native_send(&lease);check(receipts[0].evidence==4,"expiry rechecked on native task");}
    check(!live_tasks && !live_completions,"task lease released");
    char account[128]{};check(!account_snapshot(account),"identity unavailable");
    check(WercJoin(nullptr)==ERROR_INVALID_STATE,"cannot unload active unknown state");
    controller=OpenProcess(SYNCHRONIZE|PROCESS_QUERY_LIMITED_INFORMATION,FALSE,GetCurrentProcessId());
    stop_event=CreateEventW(nullptr,TRUE,FALSE,nullptr);binding.nonce[0]=27;
    binding.controller_pid=GetCurrentProcessId();binding.controller_start=creation_time(controller);
    attached=true;watcher=CreateThread(nullptr,0,watch_controller,nullptr,0,nullptr);
    check(watcher && WaitForSingleObject(watcher,2000)==WAIT_OBJECT_0,"identity loss stops watcher");
    check(closing && binding.nonce[0]==27,"identity loss preserves cleanup authentication");
    WercPacket recovery{};recovery.size=sizeof(recovery);recovery.version=WercProtocolVersion;
    recovery.operation=uint32_t(WercOp::Recover);recovery.pid=GetCurrentProcessId();recovery.process_start=creation_time(GetCurrentProcess());
    check(WercCall(&recovery)==ERROR_BUSY,"live controller cannot be replaced");
    WercPacket close=binding;close.size=sizeof(close);close.version=WercProtocolVersion;close.operation=uint32_t(WercOp::Close);
    check(WercCall(&close)==0,"identity loss permits authenticated cleanup");
    check(WercJoin(nullptr)==0 && !attached && !watcher,"join releases owned handles");
    enqueue_received("fixture","filehelper","first");
    auto first_storage=std::make_unique<ReceivedMessage>(),again_storage=std::make_unique<ReceivedMessage>();
    auto& first=*first_storage;auto& again=*again_storage;poll_received(&first);poll_received(&again);
    check(first.sequence==again.sequence && !std::strcmp(again.text,"first"),"poll retains unacknowledged event");
    check(ack_received(first.sequence+1)==ERROR_INVALID_PARAMETER,"reject wrong ack");
    for(unsigned i=0;i<65;++i) enqueue_received("fixture","filehelper","burst");
    poll_received(&again);
    check(again.sequence==first.sequence && again.dropped==1,"overflow preserves inflight and exposes gap");
    check(ack_received(first.sequence)==0 && ack_received(first.sequence)==0,"ack is idempotent");
    poll_received(&again);
    check(again.sequence==3 && !std::strcmp(again.text,"burst"),"advance only after ack");
    // Chunk protocol reads a retained copy and does not acknowledge it.
    std::string long_body(16383,'x');
    receive_head=receive_count=0;SecureZeroMemory(&receive_inflight,sizeof(receive_inflight));
    enqueue_received("fixture","filehelper",long_body.c_str(),nullptr,49,nullptr,0,123);
    poll_received(&first);
    check(first.type==49 && std::strlen(first.text)==16383,"bounded media metadata retained");
    WercPacket chunk{};chunk.sequence=first.sequence;chunk.dropped=0;
    check(receive_chunk(chunk)==0 && chunk.present==1024 && chunk.receipt==49 && chunk.deadline==16383 && chunk.message_id==123,"typed first receive chunk");
    chunk.dropped=16380;check(receive_chunk(chunk)==0 && chunk.present==3,"receive last chunk exact bytes");
    chunk.dropped=16383;check(receive_chunk(chunk)==ERROR_INVALID_STATE,"receive offset bounds");
    chunk.dropped=0;chunk.sequence++;
    check(receive_chunk(chunk)==ERROR_INVALID_STATE,"receive chunk cannot cross sequence");chunk.sequence--;
    check(ack_received(first.sequence)==0 && receive_chunk(chunk)==ERROR_INVALID_STATE,"ack clears all metadata bytes");
    check(observed_type(34) && observed_type(10002) && !observed_type(999),"explicit receive types");
    for (const auto type : {37u,42u,48u,50u}) {
        enqueue_received("fixture","filehelper","owned typed observation",nullptr,type,nullptr,0,456);
        poll_received(&first);
        check(observed_type(type) && first.type==type && first.native_message_id==456 && !std::strcmp(first.text,"owned typed observation"),"verification/contact/location/call observation retained");
        check(ack_received(first.sequence)==0,"additional typed observation ACK");
    }
    const auto drops=receive_dropped;
    long_body.push_back('x');enqueue_received("fixture","filehelper",long_body.c_str());
    check(receive_dropped==drops+1 && !receive_count,"oversized content records a gap");
    // Actual hook extractor against owned AddMsg-shaped memory, no client calls.
    alignas(8) unsigned char owned_message[0x78]{};
    SmallString from{},to{},body{};from.capacity=to.capacity=15;
    strcpy_s(from.data,"fixture");from.length=7;strcpy_s(to.data,"filehelper");to.length=10;
    std::string utf8="<owned>"+std::string(3000,'x')+"自有</owned>";
    *reinterpret_cast<const char**>(body.data)=utf8.c_str();body.length=utf8.size();body.capacity=utf8.size();
    struct OwnedWrapper {void* unused;SmallString* text;} from_w{nullptr,&from},to_w{nullptr,&to},body_w{nullptr,&body};
    *reinterpret_cast<void**>(owned_message+8)=&from_w;*reinterpret_cast<void**>(owned_message+0x18)=&to_w;*reinterpret_cast<void**>(owned_message+0x20)=&body_w;
    *reinterpret_cast<uint32_t*>(owned_message+0x14)=34;*reinterpret_cast<uint64_t*>(owned_message+0x50)=991;
    uintptr_t batch[]{reinterpret_cast<uintptr_t>(owned_message),reinterpret_cast<uintptr_t>(owned_message)+sizeof(owned_message)};
    strcpy_s(receive_login,"fixture");strcpy_s(binding.account,"fixture");
    receive_queue_mode=true;sample_receive(batch);poll_received(&first);
    check(first.type==34 && first.native_message_id==991 && !std::strcmp(first.text,utf8.c_str()),"hook copies owned UTF8 media metadata");
    check(ack_received(first.sequence)==0,"owned extracted metadata acknowledged");
    for (const auto type : {37u,42u,48u,50u}) {
        *reinterpret_cast<uint32_t*>(owned_message+0x14)=type;
        sample_receive(batch);poll_received(&first);
        check(first.type==type && first.native_message_id==991 && !std::strcmp(first.text,utf8.c_str()),"hook retains verification/contact/location/call metadata");
        check(ack_received(first.sequence)==0,"typed hook observation acknowledged");
    }
    *reinterpret_cast<uint32_t*>(owned_message+0x14)=34;
    unsigned char voice_sample[]{2,'#','!','S','I','L','K','_','V','3',2,0,0,1};
    SmallString voice_string{};std::memcpy(voice_string.data,voice_sample,sizeof(voice_sample));voice_string.length=sizeof(voice_sample);voice_string.capacity=15;
    struct OwnedBuffer {void* unused;SmallString* bytes;uint32_t length;} voice_buffer{nullptr,&voice_string,sizeof(voice_sample)};
    *reinterpret_cast<void**>(owned_message+0x30)=&voice_buffer;
    sample_receive(batch);poll_received(&first);
    check(first.voice_length==sizeof(voice_sample) && !std::memcmp(first.voice_bytes,voice_sample,sizeof(voice_sample)),"profile1 inline voice captures embedded NUL");
    WercPacket vc{};vc.sequence=first.sequence;
    check(receive_voice_chunk(vc)==0 && vc.present==sizeof(voice_sample) && vc.deadline==sizeof(voice_sample) && !std::memcmp(vc.text,voice_sample,sizeof(voice_sample)),"binary voice chunk authenticated observation");
    vc.dropped=sizeof(voice_sample);check(receive_voice_chunk(vc)==ERROR_INVALID_STATE,"voice chunk bounds");vc.dropped=0;vc.sequence++;
    check(receive_voice_chunk(vc)==ERROR_INVALID_STATE,"voice chunk sequence binding");vc.sequence--;
    check(ack_received(first.sequence)==0 && receive_voice_chunk(vc)==ERROR_INVALID_STATE && receive_inflight.voice_length==0,"ACK clears inline voice");
    voice_buffer.length=262145;sample_receive(batch);poll_received(&first);
    check(first.voice_length==0 && first.type==34,"oversized voice leaves metadata only");ack_received(first.sequence);
    voice_buffer.length=sizeof(voice_sample)-1;sample_receive(batch);poll_received(&first);
    check(first.voice_length==0 && first.type==34,"declared voice length mismatch preserves metadata");ack_received(first.sequence);
    *reinterpret_cast<void**>(owned_message+0x30)=reinterpret_cast<void*>(1);sample_receive(batch);poll_received(&first);
    check(first.voice_length==0 && first.type==34,"invalid voice buffer pointer contained");ack_received(first.sequence);
    std::vector<unsigned char> long_voice(262144);for(size_t i=0;i<long_voice.size();++i)long_voice[i]=static_cast<unsigned char>(i%251);
    *reinterpret_cast<const unsigned char**>(voice_string.data)=long_voice.data();voice_string.length=voice_string.capacity=long_voice.size();voice_buffer.length=static_cast<uint32_t>(long_voice.size());
    *reinterpret_cast<void**>(owned_message+0x30)=&voice_buffer;sample_receive(batch);poll_received(&first);
    check(first.voice_length==long_voice.size(),"maximum bounded heap voice capture");vc.sequence=first.sequence;
    std::vector<unsigned char> restored;for(uint32_t at=0;at<first.voice_length;){vc.dropped=at;check(receive_voice_chunk(vc)==0 && vc.present>0,"voice multi chunk read");restored.insert(restored.end(),vc.text,vc.text+vc.present);at+=vc.present;}
    check(restored==long_voice,"all binary chunks reassemble including NUL");ack_received(first.sequence);
    *reinterpret_cast<const unsigned char**>(voice_string.data)=reinterpret_cast<const unsigned char*>(1);sample_receive(batch);poll_received(&first);
    check(first.voice_length==0 && first.type==34,"invalid voice data pointer contained");ack_received(first.sequence);
    *reinterpret_cast<void**>(owned_message+0x30)=nullptr;
    const auto invalid_before=receive_dropped;utf8[10]=0;sample_receive(batch);
    check(receive_dropped==invalid_before+1 && !receive_count,"embedded NUL cannot truncate metadata silently");utf8[10]='x';
    *reinterpret_cast<uint32_t*>(owned_message+0x14)=999;sample_receive(batch);
    check(receive_dropped==invalid_before+2 && !receive_count,"unsupported targeted type records gap");
    check(receive_conversation("fixture","fixture","peer") && !std::strcmp(receive_conversation("fixture","fixture","peer"),"peer"),"outgoing conversation route");
    check(receive_conversation("fixture","123@chatroom","fixture") && !std::strcmp(receive_conversation("fixture","123@chatroom","fixture"),"123@chatroom"),"incoming group envelope route");
    check(!receive_conversation("fixture","someone","another") && !receive_conversation("fixture","bad/id","fixture"),"unrelated and malformed routes refused");
    *reinterpret_cast<uint32_t*>(owned_message+0x14)=1;
    strcpy_s(from.data,"123@chatroom");from.length=12;strcpy_s(to.data,"fixture");to.length=7;
    sample_receive(batch);WercPacket routed{};
    check(poll_packet(routed)==0 && routed.present==1 && !std::strcmp(routed.target,"123@chatroom") && !std::strcmp(routed.text,"123@chatroom") && !std::strcmp(routed.text+128,"fixture"),"hook and poll preserve real conversation envelope");
    WercPacket route_retry{};check(poll_packet(route_retry)==0 && route_retry.sequence==routed.sequence && !std::memcmp(route_retry.text,routed.text,256),"poll retry retains envelope until ACK");
    check(ack_received(routed.sequence)==0,"routed event ACK");
    const auto route_drops=receive_dropped;strcpy_s(to.data,"other");to.length=5;sample_receive(batch);
    check(receive_count==0 && receive_dropped==route_drops+1,"unrelated hook envelope records gap without misattribution");
    receive_queue_mode=false;
    wchar_t self[MAX_PATH]{};GetModuleFileNameW(nullptr,self,MAX_PATH);
    std::wstring command=L"\""+std::wstring(self)+L"\" --owned-controller";
    STARTUPINFOW startup{};startup.cb=sizeof(startup);PROCESS_INFORMATION child{};
    check(CreateProcessW(self,command.data(),nullptr,nullptr,FALSE,CREATE_NO_WINDOW,nullptr,nullptr,&startup,&child)!=FALSE,"owned controller starts");
    CloseHandle(child.hThread);controller=child.hProcess;
    check(WaitForSingleObject(controller,5000)==WAIT_OBJECT_0,"owned controller exits");
    stop_event=CreateEventW(nullptr,TRUE,FALSE,nullptr);attached=true;closing=0;
    watcher=CreateThread(nullptr,0,watch_controller,nullptr,0,nullptr);
    check(watcher && WaitForSingleObject(watcher,2000)==WAIT_OBJECT_0 && closing,"controller death closes watcher");
    live_tasks=1;check(WercCall(&recovery)==ERROR_BUSY,"pending task prevents recovery");live_tasks=0;
    check(WercCall(&recovery)==0 && recovered_resident && !attached && !controller && !watcher,"dead owner recovered without unloading code");
    check(WercCall(&recovery)==0,"inert resident can be reused again");
    std::vector<unsigned char> audio={2,'#','!','S','I','L','K','_','V','3',2,0,1,1};
    check(voice_frames(audio,20) && !voice_frames(audio,40),"voice duration bound to packets");
    auto broken=audio;broken.pop_back();check(!voice_frames(broken,20),"truncated voice packet");
    const char* hash="f607764bb6e5ca6d9ca880d859440c48ae5fb540734346262390d800a2cfb421";
    check(voice_hash(audio,hash),"voice SHA256 verification");
    WercPacket v{};v.operation=uint32_t(WercOp::VoiceBegin);v.deadline=uint64_t(std::time(nullptr))+30;
    strcpy_s(v.target,"filehelper");strcpy_s(v.attempt,"voice-fixture");strcpy_s(v.digest,hash);strcpy_s(v.text,hash);v.message_id=audio.size();v.dropped=20;
    v.message_id=512*1024+1;check(voice_dispatch(v)==ERROR_INVALID_PARAMETER,"voice upload quota");v.message_id=audio.size();
    check(voice_dispatch(v)==0 && voice_stage,"voice upload begins without native invocation");
    {auto changed=v;strcpy_s(changed.target,"owned-alternate-target@chatroom");check(prepare_target(changed)==0,"voice alternate target prepared");
      changed.operation=uint32_t(WercOp::VoiceChunk);changed.sequence=0;changed.present=1;
      check(voice_dispatch(changed)==ERROR_INVALID_STATE,"voice staged recipient immutable");}

    v.operation=uint32_t(WercOp::VoiceChunk);v.present=static_cast<uint32_t>(audio.size());v.sequence=1;std::memcpy(v.text,audio.data(),audio.size());
    check(voice_dispatch(v)==ERROR_INVALID_PARAMETER,"out of order voice chunk");v.sequence=0;
    check(voice_dispatch(v)==0 && !live_tasks,"voice chunk copies without native invocation");
    check(voice_dispatch(v)==ERROR_INVALID_PARAMETER,"duplicate chunk rejected");
    receipt_count=1;strcpy_s(receipts[0].target,v.target);strcpy_s(receipts[0].attempt,v.attempt);strcpy_s(receipts[0].digest,v.digest);receipts[0].kind=1;receipts[0].evidence=5;
    v.operation=uint32_t(WercOp::VoiceProbe);check(voice_dispatch(v)==ERROR_INVALID_DATA,"probe cannot reuse send attempt");
    v.operation=uint32_t(WercOp::VoiceSend);check(voice_dispatch(v)==0 && v.receipt==5 && !live_tasks,"duplicate voice attempt is receipt only");
    auto staged=std::make_unique<VoiceStage>();staged->packet=v;staged->packet.deadline=0;
    {VoiceLease lease(std::move(staged),0,false);voice_on_task(&lease);check(receipts[0].evidence==4,"late voice task rejected before constructor");}
    check(!live_tasks,"voice lease released");
    std::vector<unsigned char> png(std::begin(owned_image),std::end(owned_image));
    ImageWideString inline_path{};wcscpy_s(inline_path.data,L"a.png");inline_path.length=5;inline_path.capacity=7;
    check(image_wide_equals(&inline_path,L"a.png") && !image_wide_equals(&inline_path,L"b.png"),"image inline path exact match");
    const wchar_t* heap_path=L"C:\\InputTemp\\owned.png";ImageWideString heap_view{};
    *reinterpret_cast<const wchar_t**>(heap_view.data)=heap_path;heap_view.length=wcslen(heap_path);heap_view.capacity=heap_view.length;
    check(image_wide_equals(&heap_view,heap_path),"image heap path exact match");
    heap_view.capacity=8;check(!image_wide_equals(&heap_view,heap_path),"image invalid path capacity rejected");
    check(image_shape(png,2|(2<<16)) && !image_shape(png,3|(2<<16)),"image dimensions authenticated");
    auto invalid_png=png;invalid_png.pop_back();check(!image_shape(invalid_png,2|(2<<16)),"truncated image rejected");
    check(!image_local_path(L"C:\\temp\\..\\file.png",false) && !image_local_path(L"\\\\server\\file.png",false) && !image_local_path(L"C:\\file.png:stream",false),"image paths reject traversal network and stream");
    ImageStage image_file_stage;image_file_stage.bytes=png;
    check(image_stage_file(image_file_stage),"owned PNG staging file");
    const auto staged_path=image_file_stage.path,staged_directory=image_file_stage.directory;
    check(!DeleteFileW(staged_path.c_str()),"staging pin blocks deletion");
    CloseHandle(image_file_stage.pin);image_file_stage.pin=INVALID_HANDLE_VALUE;
    check(DeleteFileW(staged_path.c_str()) && RemoveDirectoryW(staged_directory.c_str()),"owned staging cleanup");image_file_stage.owns_file=image_file_stage.owns_directory=false;
    WercPacket im{};im.operation=uint32_t(WercOp::ImageBegin);im.deadline=uint64_t(std::time(nullptr))+30;
    strcpy_s(im.target,"filehelper");strcpy_s(im.attempt,"image-fixture");strcpy_s(im.digest,owned_image_sha256);strcpy_s(im.text,owned_image_sha256);im.message_id=png.size();im.dropped=2|(2<<16);
    im.message_id=image_limit+1;check(image_dispatch(im)==ERROR_INVALID_PARAMETER,"image upload quota");im.message_id=png.size();
    check(image_dispatch(im)==0 && image_stage,"image begin never invokes native factory");
    {auto changed=im;strcpy_s(changed.target,"owned-alternate-target@chatroom");check(prepare_target(changed)==0,"image alternate target prepared");
      changed.operation=uint32_t(WercOp::ImageChunk);changed.sequence=0;changed.present=1;
      check(image_dispatch(changed)==ERROR_INVALID_STATE,"image staged recipient immutable");}

    im.operation=uint32_t(WercOp::ImageChunk);im.present=static_cast<uint32_t>(png.size());im.sequence=1;std::memcpy(im.text,png.data(),png.size());
    check(image_dispatch(im)==ERROR_INVALID_PARAMETER,"image chunk order");im.sequence=0;
    check(image_dispatch(im)==0 && !live_tasks,"image chunk has no native effect");
    check(image_dispatch(im)==ERROR_INVALID_PARAMETER,"duplicate image chunk rejected");
    receipt_count=1;strcpy_s(receipts[0].target,im.target);strcpy_s(receipts[0].attempt,im.attempt);strcpy_s(receipts[0].digest,im.digest);receipts[0].kind=3;receipts[0].evidence=7;
    im.operation=uint32_t(WercOp::ImageProbe);check(image_dispatch(im)==ERROR_INVALID_DATA,"image probe cannot reuse submit attempt");
    im.operation=uint32_t(WercOp::ImageSend);check(image_dispatch(im)==0 && im.receipt==7 && !live_tasks,"image duplicate receipt only");
    auto image_expired=std::make_unique<ImageStage>();image_expired->packet=im;image_expired->packet.deadline=0;
    {ImageLease lease(std::move(image_expired),0,false);image_on_task(&lease);check(receipts[0].evidence==4,"image deadline checked before native constructor");}
    check(!live_tasks,"image lease cleanup completed");
    check(file_name_valid("owned.txt") && file_name_valid("自动化测试.txt"),"file UTF8 basenames accepted");
    check(!file_name_valid("../x") && !file_name_valid("CON.txt") && !file_name_valid("a:stream") && !file_name_valid("x."),"file invalid names rejected");
    WercPacket f{};f.operation=uint32_t(WercOp::FileBegin);f.deadline=uint64_t(std::time(nullptr))+30;
    strcpy_s(f.target,"filehelper");strcpy_s(f.attempt,"file-fixture");strcpy_s(f.digest,owned_image_sha256);strcpy_s(f.text,owned_image_sha256);strcpy_s(f.text+65,sizeof(f.text)-65,"owned.bin");f.message_id=png.size();
    f.message_id=file_limit+1;check(file_dispatch(f)==ERROR_INVALID_PARAMETER,"file upload quota");f.message_id=png.size();
    check(file_dispatch(f)==0 && file_stage,"file begin does not invoke native code");
    {auto changed=f;strcpy_s(changed.target,"owned-alternate-target@chatroom");check(prepare_target(changed)==0,"file alternate target prepared");
      changed.operation=uint32_t(WercOp::FileChunk);changed.sequence=0;changed.present=1;
      check(file_dispatch(changed)==ERROR_INVALID_STATE,"file staged recipient immutable");}

    f.operation=uint32_t(WercOp::FileChunk);f.present=static_cast<uint32_t>(png.size());f.sequence=1;std::memcpy(f.text,png.data(),png.size());
    check(file_dispatch(f)==ERROR_INVALID_PARAMETER,"file chunk ordering");f.sequence=0;
    check(file_dispatch(f)==0 && !live_tasks,"file bytes staged without send");
    check(file_dispatch(f)==ERROR_INVALID_PARAMETER,"file duplicate chunk rejected");
    receipt_count=1;strcpy_s(receipts[0].target,f.target);strcpy_s(receipts[0].attempt,f.attempt);strcpy_s(receipts[0].digest,f.digest);receipts[0].kind=5;receipts[0].evidence=9;
    f.operation=uint32_t(WercOp::FileProbe);check(file_dispatch(f)==ERROR_INVALID_DATA,"file probe and send attempts separated");
    f.operation=uint32_t(WercOp::FileSend);check(file_dispatch(f)==0 && f.receipt==9 && !live_tasks,"file duplicate send returns receipt only");
    auto expired_file=std::make_unique<FileStage>();expired_file->packet=f;expired_file->packet.deadline=0;
    {FileLease lease(std::move(expired_file),0,false);file_on_task(&lease);check(receipts[0].evidence==4,"expired file task never calls native constructor");}
    check(!live_tasks,"file task lifetime closed");
    check(video_name_valid("自动化测试.MP4") && !video_name_valid("owned.txt"),"video extension gate");
    WercPacket vs{};vs.operation=uint32_t(WercOp::VideoBegin);vs.deadline=uint64_t(std::time(nullptr))+30;
    strcpy_s(vs.target,"filehelper");strcpy_s(vs.attempt,"video-fixture");strcpy_s(vs.digest,owned_image_sha256);strcpy_s(vs.text,owned_image_sha256);strcpy_s(vs.text+65,sizeof(vs.text)-65,"owned.mp4");vs.message_id=png.size();
    check(video_dispatch(vs)==ERROR_INVALID_PARAMETER,"video rejects zero duration");vs.dropped=60001;
    check(video_dispatch(vs)==ERROR_INVALID_PARAMETER,"video rejects duration over limit");vs.dropped=4000;vs.message_id=file_limit+1;
    check(video_dispatch(vs)==ERROR_INVALID_PARAMETER,"video size limit");vs.message_id=png.size();
    check(video_dispatch(vs)==0 && video_stage,"video begin has no native effect");
    {auto changed=vs;strcpy_s(changed.target,"owned-alternate-target@chatroom");check(prepare_target(changed)==0,"video alternate target prepared");
      changed.operation=uint32_t(WercOp::VideoChunk);changed.sequence=0;changed.present=1;
      check(video_dispatch(changed)==ERROR_INVALID_STATE,"video staged recipient immutable");}

    vs.operation=uint32_t(WercOp::VideoChunk);vs.present=static_cast<uint32_t>(png.size());vs.sequence=0;std::memcpy(vs.text,png.data(),png.size());vs.dropped=5000;
    check(video_dispatch(vs)==ERROR_INVALID_STATE,"video duration bound through chunks");vs.dropped=4000;
    check(video_dispatch(vs)==0 && !live_tasks,"video staging has no native effect");
    check(video_dispatch(vs)==ERROR_INVALID_PARAMETER,"video duplicate chunk rejected");
    receipt_count=1;strcpy_s(receipts[0].target,vs.target);strcpy_s(receipts[0].attempt,vs.attempt);strcpy_s(receipts[0].digest,vs.digest);receipts[0].kind=7;receipts[0].evidence=11;
    vs.operation=uint32_t(WercOp::VideoProbe);check(video_dispatch(vs)==ERROR_INVALID_DATA,"video probe cannot reuse send attempt");
    vs.operation=uint32_t(WercOp::VideoSend);check(video_dispatch(vs)==0 && vs.receipt==11 && !live_tasks,"video repeated attempt returns receipt without resending");
    auto expired_video=std::make_unique<FileStage>();expired_video->packet=vs;expired_video->packet.deadline=0;
    {FileLease lease(std::move(expired_video),0,true);video_on_task(&lease);check(receipts[0].evidence==4,"expired video never calls constructor");}
    check(!live_tasks,"video task cleanup");
    std::vector<unsigned char> gif_bytes={'G','I','F','8','9','a',2,0,2,0,0,0,0,0x3b};
    check(emoticon_shape(gif_bytes,2|(2<<16)) && !emoticon_shape(gif_bytes,3|(2<<16)),"GIF dimensions bound");
    auto gif_bad=gif_bytes;gif_bad.back()=0;check(!emoticon_shape(gif_bad,2|(2<<16)),"GIF missing trailer rejected");
    gif_bad.resize(emoticon_limit+1,0x3b);check(!emoticon_shape(gif_bad,2|(2<<16)),"GIF byte budget");
    check(emoticon_name_valid("自动化测试.GIF") && !emoticon_name_valid("owned.mp4"),"GIF filename gate");
    const char* gif_hash="8b3eabe0e07a897c8de26b110e8d5a3875828e75fd6033aac6afc5307305c71c";check(voice_hash(gif_bytes,gif_hash),"GIF owned header digest");
    WercPacket es{};es.operation=uint32_t(WercOp::EmoticonBegin);es.deadline=uint64_t(std::time(nullptr))+30;
    strcpy_s(es.target,"filehelper");strcpy_s(es.attempt,"gif-fixture");strcpy_s(es.digest,gif_hash);strcpy_s(es.text,gif_hash);strcpy_s(es.text+65,sizeof(es.text)-65,"owned.gif");es.message_id=gif_bytes.size();
    check(emoticon_dispatch(es)==ERROR_INVALID_PARAMETER,"GIF zero dimensions rejected");es.dropped=2|(2<<16);
    es.message_id=emoticon_limit+1;check(emoticon_dispatch(es)==ERROR_INVALID_PARAMETER,"GIF begin quota");es.message_id=gif_bytes.size();
    check(emoticon_dispatch(es)==0 && emoticon_stage && !live_tasks,"GIF begin never constructs");
    {auto changed=es;strcpy_s(changed.target,"owned-alternate-target@chatroom");check(prepare_target(changed)==0,"emoticon alternate target prepared");
      changed.operation=uint32_t(WercOp::EmoticonChunk);changed.sequence=0;changed.present=1;
      check(emoticon_dispatch(changed)==ERROR_INVALID_STATE,"emoticon staged recipient immutable");}

    es.operation=uint32_t(WercOp::EmoticonChunk);es.present=static_cast<uint32_t>(gif_bytes.size());std::memcpy(es.text,gif_bytes.data(),gif_bytes.size());es.dropped=3|(2<<16);
    check(emoticon_dispatch(es)==ERROR_INVALID_STATE,"GIF dimensions immutable during chunks");es.dropped=2|(2<<16);
    es.sequence=1;check(emoticon_dispatch(es)==ERROR_INVALID_PARAMETER,"GIF chunk ordering");es.sequence=0;
    check(emoticon_dispatch(es)==0 && !live_tasks,"GIF chunks never send");
    check(emoticon_dispatch(es)==ERROR_INVALID_PARAMETER,"GIF duplicate chunk refused");
    receipt_count=1;strcpy_s(receipts[0].target,es.target);strcpy_s(receipts[0].attempt,es.attempt);strcpy_s(receipts[0].digest,es.digest);receipts[0].kind=9;receipts[0].evidence=13;
    es.operation=uint32_t(WercOp::EmoticonProbe);check(emoticon_dispatch(es)==ERROR_INVALID_DATA,"GIF probe cannot reuse send attempt");
    es.operation=uint32_t(WercOp::EmoticonSend);check(emoticon_dispatch(es)==0 && es.receipt==13 && !live_tasks,"GIF duplicate attempt receipt only");
    auto expired_gif=std::make_unique<FileStage>();expired_gif->packet=es;expired_gif->packet.deadline=0;
    {FileLease lease(std::move(expired_gif),0,true);emoticon_on_task(&lease);check(receipts[0].evidence==4,"GIF expired task rejected before native calls");}
    check(!live_tasks,"GIF task release");
    WercPacket dl{};dl.message_id=100;dl.dropped=2;strcpy_s(dl.text,"aabbccdd");strcpy_s(dl.text+769,33,"0123456789abcdef0123456789abcdef");
    check(download_descriptor_valid(dl),"bounded image download descriptor");
    dl.message_id=image_limit+1;check(!download_descriptor_valid(dl),"download declared size quota");dl.message_id=100;
    dl.dropped=5;check(download_descriptor_valid(dl),"ordinary file descriptor bounded");
    dl.dropped=4;check(download_descriptor_valid(dl),"bounded ordinary video descriptor");
    dl.message_id=video_download_limit;check(download_descriptor_valid(dl),"video maximum accepted");
    dl.message_id=video_download_limit+1;check(!download_descriptor_valid(dl),"video over maximum rejected");dl.message_id=100;
    dl.dropped=1;check(download_descriptor_valid(dl),"original image descriptor accepted");
    dl.message_id=image_limit+1;check(!download_descriptor_valid(dl),"original image size quota");dl.message_id=100;
    dl.dropped=3;check(!download_descriptor_valid(dl),"unrequested thumbnail rejected");dl.dropped=2;
    dl.text[0]='h';check(!download_descriptor_valid(dl),"download no arbitrary URL");dl.text[0]='a';
    dl.text[769]='x';check(!download_descriptor_valid(dl),"download AES hex required");dl.text[769]='0';
    ImageStage download_owned;download_owned.bytes=png;check(image_stage_file(download_owned),"owned download byte fixture");
    std::vector<unsigned char> download_bytes;
    check(download_read_file(download_owned.path.c_str(),png.size(),download_bytes)==0 && download_bytes==png,"download bytes read only after exact length");
    download_bytes.clear();check(download_read_file(download_owned.path.c_str(),png.size()+1,download_bytes)!=0 && download_bytes.empty(),"download actual size mismatch rejected before allocation");
    strcpy_s(dl.target,"filehelper");strcpy_s(dl.attempt,"download-fixture");strcpy_s(dl.digest,hash);
    dl.operation=uint32_t(WercOp::DownloadOriginalImageBegin);
    check(download_dispatch(dl)==ERROR_INVALID_PARAMETER,"original operation rejects medium type before scheduling");
    dl.dropped=1;dl.operation=uint32_t(WercOp::DownloadImageBegin);
    check(download_dispatch(dl)==ERROR_INVALID_PARAMETER,"medium operation rejects original type before scheduling");
    dl.operation=uint32_t(WercOp::DownloadOriginalImageBegin);dl.deadline=uint64_t(std::time(nullptr))+60;
    check(download_dispatch(dl)==ERROR_INVALID_PARAMETER,"original rejects unbound profile before scheduling");dl.dropped=2;
    dl.operation=uint32_t(WercOp::DownloadVideoBegin);
    check(download_dispatch(dl)==ERROR_INVALID_PARAMETER,"video operation rejects image type before scheduling");
    dl.dropped=4;dl.operation=uint32_t(WercOp::DownloadImageBegin);
    check(download_dispatch(dl)==ERROR_INVALID_PARAMETER,"image operation rejects video type before scheduling");
    dl.operation=uint32_t(WercOp::DownloadVideoBegin);dl.deadline=uint64_t(std::time(nullptr))+60;
    check(download_dispatch(dl)==ERROR_INVALID_PARAMETER,"video rejects unbound profile before scheduling");dl.dropped=2;
    download_result=std::make_shared<DownloadResult>();download_result->packet=dl;
    dl.operation=uint32_t(WercOp::DownloadPoll);
    check(download_dispatch(dl)==0 && dl.receipt==1 && !dl.text[0],"pending download poll has no credentials");
    dl.operation=uint32_t(WercOp::DownloadChunk);check(download_dispatch(dl)==ERROR_INVALID_STATE,"pending download never exposes partial bytes");
    dl.operation=uint32_t(WercOp::DownloadRelease);auto retained=download_result;
    check(download_dispatch(dl)==0 && !download_result && retained->cancelled,"release cancels and clears slot without send");retained.reset();
    check(mention_members_valid("a,b,member_long_123456789012345"),"mention exact IDs accepted");
    for(const auto value:{"",",a","a,","a,,b","a,a","@all","a@chatroom","a b","a,b,c,d,e,f,g,h,i"})
        check(!mention_members_valid(value),"mention invalid membership rejected");
    std::string member_limit;
    for(unsigned i=0;i<8;++i){if(i)member_limit+=',';member_limit+=char('a'+i);member_limit+=std::string(126,'x');}
    check(member_limit.size()==1023 && mention_members_valid(member_limit.c_str()),"mention maximum bounded CSV");
    check(!mention_members_valid((member_limit+"x").c_str()),"mention overlong CSV rejected");
    check(!mention_target_valid("filehelper")&&!mention_target_valid("@chatroom"),"mention requires real group ID");
    WercPacket mention{};strcpy_s(mention.target,"owned-mention@chatroom");strcpy_s(mention.attempt,"owned-mention-attempt");std::memset(mention.digest,'a',64);
    mention.deadline=uint64_t(std::time(nullptr))+60;mention.operation=uint32_t(WercOp::MentionStage);strcpy_s(mention.text,"member-a,member-b");
    clear_targets();check(mention_dispatch(mention)==ERROR_INVALID_PARAMETER,"mention unprepared group refused");
    check(prepare_target(mention)==0,"mention group prepared");
    check(mention_dispatch(mention)==0 && mention_stage && !live_tasks && !live_completions,"mention stage does not send");
    auto changed=mention;strcpy_s(changed.text,"member-c");check(mention_dispatch(changed)==ERROR_INVALID_DATA,"same attempt cannot replace staged members");
    changed=mention;changed.operation=uint32_t(WercOp::MentionSend);strcpy_s(changed.text,"owned text");changed.digest[0]='b';
    check(mention_dispatch(changed)==ERROR_INVALID_STATE,"mention digest mismatch refused before task");
    changed=mention;changed.operation=uint32_t(WercOp::MentionSend);strcpy_s(changed.text,"owned text");strcpy_s(changed.target,"other-mention@chatroom");
    check(prepare_target(changed)==0&&mention_dispatch(changed)==ERROR_INVALID_STATE,"mention target cannot change after staging");
    changed=mention;changed.operation=uint32_t(WercOp::MentionSend);changed.deadline--;
    check(mention_dispatch(changed)==ERROR_INVALID_STATE,"mention deadline bound to stage");
    changed.deadline=0;check(mention_dispatch(changed)==ERROR_INVALID_PARAMETER,"mention expired before enqueue");
    changed=mention;changed.operation=uint32_t(WercOp::MentionSend);std::memset(changed.text,'x',sizeof(changed.text));
    check(mention_dispatch(changed)==ERROR_INVALID_PARAMETER,"mention requires terminated body");
    changed=mention;changed.operation=uint32_t(WercOp::MentionSend);changed.text[0]=-1;
    check(mention_dispatch(changed)==ERROR_INVALID_PARAMETER,"mention body must be UTF8");
    const auto saved_receipt=receipts[0];const auto saved_count=receipt_count;receipt_count=1;
    receipts[0]={};strcpy_s(receipts[0].target,mention.target);strcpy_s(receipts[0].attempt,mention.attempt);strcpy_s(receipts[0].digest,mention.digest);receipts[0].kind=11;receipts[0].evidence=2;
    changed=mention;changed.operation=uint32_t(WercOp::MentionSend);strcpy_s(changed.text,"owned text");
    check(mention_dispatch(changed)==0&&changed.receipt==2&&!mention_stage&&!live_tasks,"mention duplicate is receipt only");
    changed.operation=uint32_t(WercOp::MentionProbe);check(mention_dispatch(changed)==ERROR_INVALID_DATA,"mention probe cannot reuse send attempt");
    changed.operation=uint32_t(WercOp::Send);check(submit(changed)==ERROR_INVALID_DATA,"plain text cannot reuse mention attempt");
    NativeCompletion mention_completion{1,0,completed,destroyed,0};unsigned char mention_success=1;
    completed(&mention_completion,&mention_success);check(receipts[0].evidence==0,"callback without verified member field stays unknown");
    receipts[0].native_duration=1;completed(&mention_completion,&mention_success);
    check(receipts[0].evidence==2,"mention callback requires actual factory field verification");
    receipts[0]=saved_receipt;receipt_count=saved_count;clear_targets();
    // No native client required: upload corruption never schedules a task.
    receipt_count=0;clear_receipts();
    WercPacket lt{};lt.operation=uint32_t(WercOp::TextBegin);strcpy_s(lt.target,"filehelper");strcpy_s(lt.attempt,"owned-long");std::memset(lt.digest,'a',64);lt.deadline=uint64_t(std::time(nullptr))+60;
    const std::pair<size_t,const char*> long_cases[]={{1024,"49abd65bbf7f7e40c7055093ed2e3fd75f2f602f2c5fcf955c213e3135eb03f7"},{4094,"02ed2c5d37a07056c2ffdac45c4ee59ca7f5b82efc8dcf5022072ce35a9f43a6"},{4095,"deffa2b8b74b214c6f9b3fc3897854f5c1d7ab973da346509026a1032a4ece54"},{4096,"a2e659dacb4691e887ac0139f8893d04764ee197d70fb73d3190d56113d18e3e"},{10000,"e4ee97ec252749d2096447e849628d0d7734f51700416eefbb33574bf0b3ee75"}};
    for(const auto& item:long_cases){
        lt.message_id=item.first;strcpy_s(lt.text,item.second);lt.operation=uint32_t(WercOp::TextBegin);
        long_text_stage.reset();
        check(long_text_dispatch(lt)==0&&!live_tasks,"long begin never schedules native task");
        auto chunk=lt;chunk.operation=uint32_t(WercOp::TextChunk);chunk.present=1;chunk.sequence=1;chunk.text[0]='x';
        check(long_text_dispatch(chunk)==ERROR_INVALID_PARAMETER,"long chunks require contiguous offset");
        chunk.sequence=0;chunk.deadline--;check(long_text_dispatch(chunk)==ERROR_INVALID_STATE,"long deadline bound");chunk.deadline++;
        chunk.digest[0]='b';check(long_text_dispatch(chunk)==ERROR_INVALID_STATE,"long approved digest bound");chunk.digest[0]='a';
        chunk.dropped=1;check(long_text_dispatch(chunk)==ERROR_INVALID_PARAMETER,"long mentions require group target");chunk.dropped=0;
        auto finish=lt;finish.operation=uint32_t(WercOp::TextSend);
        check(long_text_dispatch(finish)==ERROR_INVALID_STATE&&!live_tasks,"incomplete body never submits");
        for(size_t offset=0;offset<item.first;){chunk.sequence=offset;chunk.present=static_cast<uint32_t>(std::min<size_t>(1024,item.first-offset));std::memset(chunk.text,'x',sizeof(chunk.text));
            check(long_text_dispatch(chunk)==0&&!live_tasks,"long chunks stage bytes only");offset+=chunk.present;}
        check(long_text_dispatch(chunk)==ERROR_INVALID_PARAMETER,"long repeated chunk rejected");
        check(long_text_complete(*long_text_stage),"full bounded long body and SHA verified");
        check(long_text_dispatch(lt)==0&&long_text_stage->bytes.size()==item.first,"same begin cannot rewind");
        auto changed_begin=lt;changed_begin.text[0]=changed_begin.text[0]=='a'?'b':'a';
        check(long_text_dispatch(changed_begin)==ERROR_INVALID_DATA,"same begin cannot change body hash");
        long_text_stage->bytes[0]=0;check(!long_text_complete(*long_text_stage),"embedded NUL rejected");
        long_text_stage->bytes[0]=0xff;check(!long_text_complete(*long_text_stage),"invalid UTF8 rejected");
        long_text_stage->bytes[0]='y';check(!long_text_complete(*long_text_stage),"changed bytes fail SHA");
    }
    lt.message_id=10001;check(long_text_dispatch(lt)==ERROR_INVALID_PARAMETER,"10001 bytes rejected");
    lt.message_id=1023;check(long_text_dispatch(lt)==ERROR_INVALID_PARAMETER,"short text uses existing protocol");
    lt.message_id=10000;lt.operation=uint32_t(WercOp::TextSend);
    receipt_count=1;receipts[0]={};strcpy_s(receipts[0].target,lt.target);strcpy_s(receipts[0].attempt,lt.attempt);strcpy_s(receipts[0].digest,lt.digest);receipts[0].kind=13;receipts[0].evidence=2;
    check(long_text_dispatch(lt)==0&&lt.receipt==2&&!long_text_stage&&!live_tasks,"long duplicate attempt never resends");
    lt.operation=uint32_t(WercOp::TextProbe);check(long_text_dispatch(lt)==ERROR_INVALID_DATA,"long probe cannot reuse send attempt");
    lt.operation=uint32_t(WercOp::Send);strcpy_s(lt.text,"short");check(submit(lt)==ERROR_INVALID_DATA,"short text cannot reuse long attempt");
    receipts[0].kind=14;receipts[0].native_duration=0;
    completed(&mention_completion,&mention_success);check(receipts[0].evidence==0,"long mention callback without member verification unknown");
    receipts[0].native_duration=1;completed(&mention_completion,&mention_success);check(receipts[0].evidence==2,"long mention verified callback");
    lt.deadline=0;{SendLease lease(lt,0);strcpy_s(lease.body,"owned");long_text_probe_on_task(&lease);check(receipts[0].evidence==4,"expired long probe has no native effect");}
    check(!live_tasks,"long task cleanup");
#include "quote-fixture.inc"
#include "moments-fixture.inc"
#include "moments-image-fixture.inc"
#include "moments-video-fixture.inc"
#include "moments-interaction-fixture.inc"
    std::printf("{\"checks_passed\":%u,\"real_wechat_accessed\":false,\"messages_sent\":false}\n",checks);
    return 0;
}

