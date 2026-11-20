// Explicit research build only. The public bool completion has no message model.
// Observe 0x178f100's result only for the exact object captured by our text factory.
// Never infer identity from a matching body, clock, recipient, or unrelated send.
#ifdef WERC_TEXT_RESULT_RESEARCH
struct TextResultCapture {void* object=nullptr;size_t index=size_t(-1);char target[128]{},body[10001]{};};
static TextResultCapture text_result_capture{};
static void register_text_result(NativeCompletion* completion){
    AcquireSRWLockExclusive(&receipt_lock);
    text_result_capture.object=completion->observed.object;text_result_capture.index=completion->receipt_index;
    strcpy_s(text_result_capture.target,completion->expected_target);strcpy_s(text_result_capture.body,completion->expected_body);
    ReleaseSRWLockExclusive(&receipt_lock);
}
static void clear_text_result(size_t index){
    AcquireSRWLockExclusive(&receipt_lock);
    if(text_result_capture.index==index){SecureZeroMemory(&text_result_capture,sizeof(text_result_capture));text_result_capture.index=size_t(-1);}
    ReleaseSRWLockExclusive(&receipt_lock);
}
#include "text-task-research.inc"
// A scalar result is only a candidate. Resolve it through the client's exact
// ID lookup before accepting it; never call Describe on an unknown object.
static uint64_t resolve_result_candidate(uint64_t candidate,const TextResultCapture* capture,uint32_t* stage){
    alignas(16) unsigned char content[0x2f0]{},source[0x440]{};
    SharedMessage app{},provider{},manager{};SmallString target{};target.capacity=15;
    bool content_ready=false,source_ready=false;uint64_t verified=0;TestResult report{};
    __try {
        if(!candidate || closing || !same_account(binding.account))__leave;
        using GetApp=SharedMessage*(__fastcall*)(SharedMessage*);
        using Service=SharedMessage*(__fastcall*)(void*,SharedMessage*);
        using Query=void*(__fastcall*)(void*,void*,const SmallString*,uint64_t,uint32_t);
        using Describe=void*(__fastcall*)(const void*,void*);
        *stage=111;reinterpret_cast<GetApp>(core_base+0x47220)(&app);if(!app.object)__leave;
        reinterpret_cast<Service>(core_base+0x36c740)(app.object,&provider);if(!provider.object)__leave;
        reinterpret_cast<Service>(core_base+0x6f4d90)(provider.object,&manager);if(!manager.object)__leave;
        if(!set_native_string(&target,capture->target))__leave;
        *stage=112;reinterpret_cast<Query>(core_base+0x17c5260)(manager.object,content,&target,candidate,0);
        content_ready=content[0x2e8]==1;if(!content_ready)__leave;
        *stage=113;if(*reinterpret_cast<uintptr_t*>(content)!=core_base+0x8d9f958)__leave;
        reinterpret_cast<Describe>(core_base+0xa58dd0)(content,source);source_ready=true;
        *stage=114;if(*reinterpret_cast<uintptr_t*>(source)!=core_base+0x8d9e9a8 ||
            *reinterpret_cast<uint32_t*>(source+8)!=1 ||
            *reinterpret_cast<uint64_t*>(source+0xb8)!=candidate ||
            !native_text_equals(reinterpret_cast<const SmallString*>(source+0x70),capture->target) ||
            !native_text_equals(reinterpret_cast<const SmallString*>(source+0xe0),capture->body))__leave;
        *stage=115;verified=candidate;
    }__except(record_exception(GetExceptionInformation(),&report)){*stage=116;}
    using Destroy=void(__fastcall*)(void*);
    __try{if(source_ready)reinterpret_cast<Destroy>(core_base+0x1aee40)(source);}__except(record_exception(GetExceptionInformation(),&report)){}
    __try{if(content_ready)reinterpret_cast<Destroy>(core_base+0x1c50f0)(content);}__except(record_exception(GetExceptionInformation(),&report)){}
    __try{free_native_string(&target);}__except(record_exception(GetExceptionInformation(),&report)){}
    guarded_release_shared(&manager,&report);guarded_release_shared(&provider,&report);guarded_release_shared(&app,&report);
    if(report.exception){*stage=116;return 0;}return verified;
}
static void inspect_text_result(void* output,const TextResultCapture* capture){
    uint32_t stage=100;uint64_t id=0;alignas(16) unsigned char source[0x440]{};bool source_ready=false;
    __try {
        if(!output || !same_account(binding.account))__leave;
        stage=101;if(*static_cast<const uint64_t*>(output)!=0)__leave;
        const auto result=reinterpret_cast<const SharedMessage*>(static_cast<const unsigned char*>(output)+40);
        stage=102;if(!result->object||!result->control)__leave;
        const auto object=static_cast<const unsigned char*>(result->object);
        const auto vtable=*reinterpret_cast<const uintptr_t*>(object);
        stage=uint32_t(0x80000000u|uint32_t(vtable-core_base));
        if(vtable==core_base+0x8e53588){
            stage=103;if(*reinterpret_cast<const uint64_t*>(object+0x118)!=1 ||
                !native_text_equals(reinterpret_cast<const SmallString*>(object+0xb0),capture->target) ||
                !native_text_equals(reinterpret_cast<const SmallString*>(object+0x758),capture->body))__leave;
            id=*reinterpret_cast<const uint64_t*>(object+0x148);stage=id?104:105;
        }else if(vtable==core_base+0x8d9f958){
            // The same value-to-message description used by the verified quote reader.
            using Describe=void*(__fastcall*)(const void*,void*);
            reinterpret_cast<Describe>(core_base+0xa58dd0)(object,source);source_ready=true;
            stage=106;if(*reinterpret_cast<uintptr_t*>(source)!=core_base+0x8d9e9a8 ||
                *reinterpret_cast<uint32_t*>(source+8)!=1 ||
                !native_text_equals(reinterpret_cast<const SmallString*>(source+0x70),capture->target) ||
                !native_text_equals(reinterpret_cast<const SmallString*>(source+0xe0),capture->body))__leave;
            id=*reinterpret_cast<const uint64_t*>(source+0xb8);stage=id?107:108;
        }else{
            id=resolve_result_candidate(vtable,capture,&stage);
        }
    }__except(EXCEPTION_EXECUTE_HANDLER){stage=109;id=0;}
    __try{if(source_ready){using Destroy=void(__fastcall*)(void*);reinterpret_cast<Destroy>(core_base+0x1aee40)(source);}}
    __except(EXCEPTION_EXECUTE_HANDLER){stage=110;id=0;}
    AcquireSRWLockExclusive(&receipt_lock);
    if(capture->index<receipt_count){receipts[capture->index].observation_stage=stage;receipts[capture->index].observed_text_id=id;}
    ReleaseSRWLockExclusive(&receipt_lock);
}
static void* __fastcall text_result_send(void* manager,void* output,SharedMessage* message,uint8_t flag){
    _InterlockedIncrement(&active_callbacks);TextResultCapture capture{};bool selected=false;void* result=nullptr;
#ifdef WERC_TEXT_TASK_RESEARCH
    TextTaskCapture task_capture{};task_capture.request=&capture;void* prior=nullptr;bool tls_set=false;
#endif
    __try{
        AcquireSRWLockShared(&receipt_lock);
        if(message&&message->object&&message->object==text_result_capture.object&&text_result_capture.index<receipt_count){capture=text_result_capture;selected=true;}
        ReleaseSRWLockShared(&receipt_lock);
        if(selected){
            __try{const auto object=static_cast<const unsigned char*>(message->object);selected=message->control&&*reinterpret_cast<const uintptr_t*>(object)==core_base+0x8e53588&&*reinterpret_cast<const uint64_t*>(object+0x118)==1&&native_text_equals(reinterpret_cast<const SmallString*>(object+0xb0),capture.target)&&native_text_equals(reinterpret_cast<const SmallString*>(object+0x758),capture.body);}
            __except(EXCEPTION_EXECUTE_HANDLER){selected=false;}
        }
#ifdef WERC_TEXT_TASK_RESEARCH
        if(selected){prior=TlsGetValue(text_task_tls);tls_set=TlsSetValue(text_task_tls,&task_capture)!=FALSE;}
#endif
        result=original_send(manager,output,message,flag);
#ifdef WERC_TEXT_TASK_RESEARCH
        if(selected&&tls_set)inspect_text_task(&task_capture);
#else
        if(selected&&result==output)inspect_text_result(output,&capture);
#endif
    }__finally{
#ifdef WERC_TEXT_TASK_RESEARCH
        if(tls_set)TlsSetValue(text_task_tls,prior);TestResult report{};guarded_release_shared(&task_capture.value,&report);guarded_release_shared(&task_capture.task,&report);
#endif
        SecureZeroMemory(&capture,sizeof(capture));_InterlockedDecrement(&active_callbacks);
    }
    return result;
}
static DWORD ensure_text_result(){
#ifdef WERC_TEXT_TASK_RESEARCH
    const auto task_status=ensure_text_task_result();if(task_status)return task_status;
#endif
    if(hook_created)return 0;
    if(!core_base||core_profile_id!=1)return ERROR_REVISION_MISMATCH;
    void* address=reinterpret_cast<void*>(core_base+0x178f100);
    auto status=MH_CreateHook(address,reinterpret_cast<void*>(&text_result_send),reinterpret_cast<void**>(&original_send));
    if(status==MH_OK){status=MH_EnableHook(address);if(status!=MH_OK)MH_RemoveHook(address);}
    if(status!=MH_OK)return 600+status;hook_created=true;return 0;
}
#endif
