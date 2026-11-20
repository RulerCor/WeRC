// The same 178f100 send path constructs 3981a30's task for all ordinary media.
// 179cb90 waits through 39f570's locked ready-state loop before returning.
// Capture only our constructor input, retain that task, and inspect after this
// request's returned native result. No polling of a concurrently changing task.
static void send_media_with_identity(void* manager,void* output,SharedMessage* message,
                                     size_t index,const char* target,uint64_t type) {
    TextResultCapture request{};request.object=message->object;request.index=index;
    request.expected_type=type;request.verify_body=false;strcpy_s(request.target,target);
    TextTaskCapture capture{};capture.request=&request;
    void* previous=TlsGetValue(text_task_tls);bool entered=false;
    __try {
        if(!message->object||!message->control||
           *reinterpret_cast<const uint64_t*>(static_cast<const unsigned char*>(message->object)+0x118)!=type||
           !native_text_equals(reinterpret_cast<const SmallString*>(static_cast<const unsigned char*>(message->object)+0xb0),target)||
           !TlsSetValue(text_task_tls,&capture))RaiseException(ERROR_INVALID_DATA,0,0,nullptr);
        entered=true;
        reinterpret_cast<SendFunction>(core_address(core_base,0x178f100))(manager,output,message,1);
        // Zero native status is necessary for a returned task observation. It is
        // still not server acceptance: the application independently reads the ID.
        if(!*static_cast<const uint32_t*>(output)&&!*(static_cast<const uint32_t*>(output)+1)) {
            inspect_text_task(&capture);
            AcquireSRWLockExclusive(&receipt_lock);
            if(index<receipt_count&&receipts[index].observation&&receipts[index].observation->task_holder)
                inspect_completed_text_task(&receipts[index]);
            ReleaseSRWLockExclusive(&receipt_lock);
        }
    } __finally {
        if(entered)TlsSetValue(text_task_tls,previous);
        TestResult cleanup{};if(!entered)guarded_release_shared(message,&cleanup);guarded_release_shared(&capture.task,&cleanup);
        SecureZeroMemory(&request,sizeof(request));
    }
}
