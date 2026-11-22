// Profile-1 exact outgoing identity. The public bool completion has no message model.
// Observe only the task causally constructed from this request's factory object.
// Never infer identity from a matching body, clock, recipient, or unrelated send.
struct TextResultCapture {void* object=nullptr;size_t index=size_t(-1);char target[128]{},body[10001]{};uint64_t expected_type=1;bool verify_body=true;};
static std::vector<TextResultCapture> text_result_captures;
static void register_text_result(NativeCompletion* completion){
    AcquireSRWLockExclusive(&receipt_lock);
    try {
        TextResultCapture capture{};capture.object=completion->observed.object;capture.index=completion->receipt_index;capture.expected_type=completion->expected_type;capture.verify_body=completion->verify_body;
        strcpy_s(capture.target,completion->expected_target);strcpy_s(capture.body,completion->expected_body);
        text_result_captures.push_back(capture);SecureZeroMemory(&capture,sizeof(capture));
    } catch(...) { /* Sending can still complete without a durable identity. */ }
    ReleaseSRWLockExclusive(&receipt_lock);
}
static void clear_text_result(size_t index){
    AcquireSRWLockExclusive(&receipt_lock);
    for(auto it=text_result_captures.begin();it!=text_result_captures.end();){
        if(it->index==index){SecureZeroMemory(&*it,sizeof(*it));it=text_result_captures.erase(it);}else ++it;
    }
    ReleaseSRWLockExclusive(&receipt_lock);
}
#include "text-task-identity.inc"
static void* __fastcall text_result_send(void* manager,void* output,SharedMessage* message,uint8_t flag){
    _InterlockedIncrement(&active_callbacks);TextResultCapture capture{};bool selected=false;void* result=nullptr;
    TextTaskCapture task_capture{};task_capture.request=&capture;void* prior=nullptr;bool tls_set=false;
    __try{
        AcquireSRWLockShared(&receipt_lock);
        if(message&&message->object)for(const auto& candidate:text_result_captures){
            if(message->object==candidate.object&&candidate.index<receipt_count){capture=candidate;selected=true;break;}
        }
        ReleaseSRWLockShared(&receipt_lock);
        if(selected){
            __try{const auto object=static_cast<const unsigned char*>(message->object);selected=message->control&&*reinterpret_cast<const uintptr_t*>(object)==core_address(core_base,0x8e53588)&&*reinterpret_cast<const uint64_t*>(object+0x118)==capture.expected_type&&native_text_equals(reinterpret_cast<const SmallString*>(object+0xb0),capture.target)&&(!capture.verify_body||native_text_equals(reinterpret_cast<const SmallString*>(object+0x758),capture.body));}
            __except(EXCEPTION_EXECUTE_HANDLER){selected=false;}
        }
        if(selected){prior=TlsGetValue(text_task_tls);tls_set=TlsSetValue(text_task_tls,&task_capture)!=FALSE;}
        result=original_send(manager,output,message,flag);
        if(selected&&tls_set)inspect_text_task(&task_capture);
    }__finally{
        if(tls_set)TlsSetValue(text_task_tls,prior);TestResult report{};guarded_release_shared(&task_capture.task,&report);
        SecureZeroMemory(&capture,sizeof(capture));_InterlockedDecrement(&active_callbacks);
    }
    return result;
}
static DWORD ensure_text_result(){
    const auto task_status=ensure_text_task_result();if(task_status)return task_status;
    if(hook_created)return 0;
    if(!core_base||(core_profile_id!=1&&(core_profile_id!=5&&core_profile_id!=14)&&core_profile_id!=11&&core_profile_id!=12&&core_profile_id!=13))return ERROR_REVISION_MISMATCH;
    void* address=reinterpret_cast<void*>(core_address(core_base,0x178f100));
    auto status=MH_CreateHook(address,reinterpret_cast<void*>(&text_result_send),reinterpret_cast<void**>(&original_send));
    if(status==MH_OK){status=MH_EnableHook(address);if(status!=MH_OK)MH_RemoveHook(address);}
    if(status!=MH_OK)return 600+status;hook_created=true;return 0;
}



