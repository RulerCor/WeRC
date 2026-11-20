// Research-only exact causal chain: constructor 0x3981a30 receives our
// factory object and produces task +18's strong value-model reference.
#ifdef WERC_TEXT_TASK_RESEARCH
using TextTaskConstruct=void*(__fastcall*)(void*,SharedMessage*,uint32_t);
static TextTaskConstruct original_text_task_construct=nullptr;
static bool text_task_hook=false;
static DWORD text_task_tls=TLS_OUT_OF_INDEXES;
struct TextTaskCapture {const TextResultCapture* request=nullptr;SharedMessage value{},task{};uint32_t stage=120;};
static void* __fastcall text_task_construct(void* task,SharedMessage* factory,uint32_t flag){
    _InterlockedIncrement(&active_callbacks);void* result=nullptr;
    __try {
        const auto capture=static_cast<TextTaskCapture*>(TlsGetValue(text_task_tls));
        bool selected=false;
        __try{selected=capture&&capture->request&&factory&&factory->object==capture->request->object&&factory->control&&!capture->value.object;}__except(EXCEPTION_EXECUTE_HANDLER){}
        result=original_text_task_construct(task,factory,flag);
        if(selected){
            __try {
                capture->stage=121;
                if(task&&*static_cast<const uintptr_t*>(task)==core_base+0x950a548){
                    // 178f100 allocates a shared control block at task-0x10.
                    // Its exact profile-1 vtable was verified in that constructor.
                    auto control=static_cast<unsigned char*>(task)-0x10;
                    if(*reinterpret_cast<uintptr_t*>(control)==core_base+0x90d8088){
                        _InterlockedIncrement(reinterpret_cast<volatile LONG*>(control+8));
                        capture->task={task,control};
                    }
                    const auto candidate=reinterpret_cast<const SharedMessage*>(static_cast<const unsigned char*>(task)+0x18);
                    if(candidate->object&&candidate->control&&*static_cast<const uintptr_t*>(candidate->object)==core_base+0x8d9f958&&
                        *static_cast<const uintptr_t*>(candidate->control)==core_base+0x8d9f8c8){
                        _InterlockedIncrement(reinterpret_cast<volatile LONG*>(static_cast<unsigned char*>(candidate->control)+8));
                        capture->value=*candidate;capture->stage=122;
                    }
                }
            }__except(EXCEPTION_EXECUTE_HANDLER){capture->stage=129;}
        }
    }__finally{_InterlockedDecrement(&active_callbacks);}
    return result;
}
// Only the exact task-owned value captured above can reach this reader. Keep a
// strong reference through asynchronous completion; the factory is a different object.
static uint64_t task_value_message_id(const SharedMessage* value,const char* target,const char* body,uint32_t* stage){
    alignas(16) unsigned char source[0x440]{};bool ready=false;uint64_t id=0;TestResult report{};
    __try {
        *stage=122;
        if(value->object&&value->control&&!closing&&same_account(binding.account)&&
            *static_cast<const uintptr_t*>(value->object)==core_base+0x8d9f958){
            using Describe=void*(__fastcall*)(const void*,void*);
            reinterpret_cast<Describe>(core_base+0xa58dd0)(value->object,source);ready=true;*stage=123;
            if(*reinterpret_cast<const uintptr_t*>(source)==core_base+0x8d9e9a8&&*reinterpret_cast<const uint32_t*>(source+8)==1&&
                native_text_equals(reinterpret_cast<const SmallString*>(source+0x70),target)&&
                native_text_equals(reinterpret_cast<const SmallString*>(source+0xe0),body)){
                id=*reinterpret_cast<const uint64_t*>(source+0xb8);*stage=id?125:124;
                if(id!=*reinterpret_cast<const uint64_t*>(static_cast<const unsigned char*>(value->object)+0x148)){id=0;*stage=126;}
            }
        }
    }__except(record_exception(GetExceptionInformation(),&report)){*stage=129;id=0;}
    __try{if(ready){using Destroy=void(__fastcall*)(void*);reinterpret_cast<Destroy>(core_base+0x1aee40)(source);}}__except(record_exception(GetExceptionInformation(),&report)){*stage=129;id=0;}
    SecureZeroMemory(source,sizeof(source));return id;
}
// Invoked on this request's native completion thread, after the send future
// has resolved. The function only observes the retained task's current value.
static void inspect_completed_text_task(ReceiptSlot* slot){
    auto self=slot->observation;TestResult report{};SharedMessage value{};uint32_t stage=140;uint64_t id=0;
    __try{
        if(self&&self->task_holder&&self->observed.object&&!closing&&same_account(binding.account)){
            auto candidate=reinterpret_cast<const SharedMessage*>(static_cast<const unsigned char*>(self->observed.object)+0x18);
            if(candidate->object&&candidate->control&&*static_cast<const uintptr_t*>(candidate->object)==core_base+0x8d9f958&&*static_cast<const uintptr_t*>(candidate->control)==core_base+0x8d9f8c8){
                _InterlockedIncrement(reinterpret_cast<volatile LONG*>(static_cast<unsigned char*>(candidate->control)+8));value=*candidate;
                id=task_value_message_id(&value,self->expected_target,self->expected_body,&stage);
                if(stage==124||stage==125){
                    auto bytes=static_cast<const unsigned char*>(value.object);
                    slot->task_local=*reinterpret_cast<const uint32_t*>(bytes+0x144);
                    slot->task_client=*reinterpret_cast<const uint64_t*>(bytes+0x158);
                    slot->task_millis=*reinterpret_cast<const uint64_t*>(bytes+0x150);
                    slot->task_seconds=*reinterpret_cast<const uint32_t*>(bytes+0x164);
                    stage=id?145:144;
                }
            }
        }
    }__except(record_exception(GetExceptionInformation(),&report)){stage=149;id=0;}
    guarded_release_shared(&value,&report);
    slot->observation_stage=stage;slot->observed_text_id=id;
    slot->observation=nullptr;if(self)release_completion(self);
}
static void inspect_text_task(TextTaskCapture* capture){
    NativeCompletion* observation=nullptr;
    if(capture->task.object&&!closing){
        observation=new(std::nothrow) NativeCompletion{};
        if(observation){observation->observed=capture->task;capture->task={};observation->task_holder=true;
            strcpy_s(observation->expected_target,capture->request->target);strcpy_s(observation->expected_body,capture->request->body);}
    }
    AcquireSRWLockExclusive(&receipt_lock);
    const auto index=capture->request->index;
    if(index<receipt_count){
        auto& slot=receipts[index];
        if(observation&&!slot.observation&&!closing){slot.observation=observation;slot.observation_time=GetTickCount64();slot.observation_stage=141;observation=nullptr;
            if(slot.evidence==2)inspect_completed_text_task(&slot);
        }
    }
    ReleaseSRWLockExclusive(&receipt_lock);
    if(observation)release_completion(observation);
}
static DWORD ensure_text_task_result(){
    if(text_task_hook)return 0;
    if(!core_base||core_profile_id!=1)return ERROR_REVISION_MISMATCH;
    if(text_task_tls==TLS_OUT_OF_INDEXES){text_task_tls=TlsAlloc();if(text_task_tls==TLS_OUT_OF_INDEXES)return ERROR_NOT_ENOUGH_MEMORY;}
    auto address=reinterpret_cast<void*>(core_base+0x3981a30);
    auto status=MH_CreateHook(address,reinterpret_cast<void*>(&text_task_construct),reinterpret_cast<void**>(&original_text_task_construct));
    if(status==MH_OK){status=MH_EnableHook(address);if(status!=MH_OK)MH_RemoveHook(address);}
    if(status!=MH_OK){TlsFree(text_task_tls);text_task_tls=TLS_OUT_OF_INDEXES;return 600+status;}
    text_task_hook=true;return 0;
}
static DWORD remove_text_task_hook(){
    if(!text_task_hook)return 0;
    auto address=reinterpret_cast<void*>(core_base+0x3981a30);
    auto status=MH_DisableHook(address);if(status!=MH_OK&&status!=MH_ERROR_DISABLED)return 600+status;
    for(int n=0;active_callbacks&&n<100;n++)Sleep(50);
    if(active_callbacks)return ERROR_BUSY;
    status=MH_RemoveHook(address);if(status!=MH_OK)return 600+status;
    text_task_hook=false;original_text_task_construct=nullptr;TlsFree(text_task_tls);text_task_tls=TLS_OUT_OF_INDEXES;return 0;
}
#endif
