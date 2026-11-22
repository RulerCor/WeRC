// Noncommercial helper-derived adapter. See ../helper/LICENSE and SOURCE.json.
// No upstream self-test or unrestricted send exports are exposed.
#include <cerrno>
#define WERC_EXACT_TEXT_IDENTITY
#define WXBOT_MESSAGE_ID_RECEIVE
#define WERC_ONLY_RECEIVE
#define WXBOT_QUOTE_SEND
#include "common.inc"
#undef _wfopen_s
#include "protocol.h"
#include <array>
#include <ctime>

static SRWLOCK api_lock = SRWLOCK_INIT;
static SRWLOCK receipt_lock = SRWLOCK_INIT;
static bool attached = false;
static bool recovered_resident = false;
static volatile LONG closing = 0;
static uintptr_t bound_context = 0;
static HANDLE controller = nullptr, stop_event = nullptr, watcher = nullptr;
static WercPacket binding{};
static WercPacket* transport_packet=nullptr;
struct NativeCompletion;
static void release_completion(NativeCompletion*);
struct ReceiptSlot { char attempt[64], digest[65], target[128]; uint32_t evidence,kind,native_duration; uint64_t message_id, observed_text_id; NativeCompletion* observation=nullptr; uint32_t observation_stage=0; uint64_t observation_time=0;
#ifdef WERC_EXACT_TEXT_IDENTITY
uint32_t task_local=0,task_seconds=0;uint64_t task_client=0,task_millis=0,task_type=0;
#endif
};
// Indexes are retained by asynchronous callbacks. Grow only with no live work,
// and never recycle an attempt: old retries must remain receipt-only.
static std::vector<ReceiptSlot> receipts(128);
static size_t receipt_count = 0;
static bool reserve_receipt() {
    // Completed native promises may leave one short-lived observation reference.
    // Expire it even when the caller never asks for a receipt.
    AcquireSRWLockExclusive(&receipt_lock);
    const auto now=GetTickCount64();
    for(auto& slot:receipts)if(slot.observation&&now-slot.observation_time>60000){release_completion(slot.observation);slot.observation=nullptr;}
    ReleaseSRWLockExclusive(&receipt_lock);
    if(receipt_count < receipts.size())return true;
    AcquireSRWLockExclusive(&receipt_lock);
    bool ok=true;
    try { receipts.resize(receipts.size()+128); }
    catch(...) { ok=false; }
    ReleaseSRWLockExclusive(&receipt_lock);
    return ok;
}
static void clear_receipts() {
    for(auto& slot:receipts){if(slot.observation)release_completion(slot.observation);SecureZeroMemory(&slot,sizeof(slot));}
    receipt_count=0;
}

static uint64_t creation_time(HANDLE process) {
    FILETIME created{}, exited{}, kernel{}, user{};
    if (!GetProcessTimes(process, &created, &exited, &kernel, &user)) return 0;
    return (uint64_t(created.dwHighDateTime) << 32) | created.dwLowDateTime;
}
static bool ascii_id(const char* value, size_t capacity) {
    const size_t n = strnlen_s(value, capacity);
    if (!n || n == capacity) return false;
    for (size_t i=0; i<n; ++i) {
        const unsigned char c = value[i];
        if (!((c>='a' && c<='z') || (c>='A' && c<='Z') ||
              (c>='0' && c<='9') || c=='_' || c=='-' || c=='.')) return false;
    }
    return true;
}
static bool digest_valid(const char* value) {
    if (strnlen_s(value, 65) != 64) return false;
    for (size_t i=0; i<64; ++i)
        if (!((value[i]>='0' && value[i]<='9') || (value[i]>='a' && value[i]<='f'))) return false;
    return true;
}
// The controller prepares exact targets from an authenticated client or conversation.
// This lock is separate from api_lock because scheduled client tasks recheck it.
struct PreparedTarget { char target[128]; uint64_t deadline; };
static SRWLOCK target_lock = SRWLOCK_INIT;
static std::vector<PreparedTarget> prepared_targets(16);
static bool target_id_valid(const char* value) {
    const auto n=strnlen_s(value,128);
    if(!n || n==128)return false;
    for(size_t i=0;i<n;++i){const unsigned char c=value[i];
        if(!((c>='a'&&c<='z')||(c>='A'&&c<='Z')||(c>='0'&&c<='9')||c=='_'||c=='-'||c=='.'||c=='@'))return false;}
    return true;
}
static void clear_targets() {
    AcquireSRWLockExclusive(&target_lock);
    for(auto& target:prepared_targets)SecureZeroMemory(&target,sizeof(target));
    ReleaseSRWLockExclusive(&target_lock);
}
static DWORD prepare_target(const WercPacket& p) {
    const auto now=uint64_t(std::time(nullptr));
    if(!target_id_valid(p.target)||p.deadline<=now||p.deadline>now+300)return ERROR_INVALID_PARAMETER;
    AcquireSRWLockExclusive(&target_lock);
    PreparedTarget* slot=nullptr;
    for(auto& t:prepared_targets){if(!std::strcmp(t.target,p.target)){slot=&t;break;}if(!slot&&t.deadline<=now)slot=&t;}
    if(!slot) {
        try { prepared_targets.emplace_back();slot=&prepared_targets.back(); }
        catch(...) { ReleaseSRWLockExclusive(&target_lock);return ERROR_NOT_ENOUGH_MEMORY; }
    }
    if(slot){SecureZeroMemory(slot,sizeof(*slot));strcpy_s(slot->target,p.target);slot->deadline=p.deadline;}
    ReleaseSRWLockExclusive(&target_lock);return slot?0:ERROR_BUSY;
}
static bool target_allowed(const WercPacket& p) {
    if(!target_id_valid(p.target))return false;
    if(!std::strcmp(p.target,"filehelper") || (binding.account[0] && !std::strcmp(p.target,binding.account)))return true;
    const auto now=uint64_t(std::time(nullptr));bool allowed=false;
    AcquireSRWLockShared(&target_lock);
    for(const auto& t:prepared_targets)if(!std::strcmp(t.target,p.target)&&now<t.deadline&&p.deadline<=t.deadline){allowed=true;break;}
    ReleaseSRWLockShared(&target_lock);return allowed;
}
static bool account_snapshot(char* output) {
    // Profile 1 only. Read twice; do not retain account object pointers.
    __try {
    if (core_profile_id == 11) {
        __try {
            auto app = *reinterpret_cast<unsigned char**>(core_address(core_base,0xb912690));
            if (!app || *reinterpret_cast<uintptr_t*>(app) != core_address(core_base,0x8d804e8)) return false;
            auto context = *reinterpret_cast<unsigned char**>(app+0x68);
            if (!context || *reinterpret_cast<uintptr_t*>(context) != core_address(core_base,0x8dfe2f8)) return false;
            if (*reinterpret_cast<uintptr_t*>(core_address(core_base,0x8d804e8)+0x60) != core_address(core_base,0x469a0) ||
                *reinterpret_cast<uintptr_t*>(core_address(core_base,0x8dfe2f8)+0x18) != core_address(core_base,0x36f930) ||
                std::memcmp(reinterpret_cast<void*>(core_address(core_base,0x36f930)), "\x48\x8d\x41\x48\xc3", 5)) return false;
            const auto raw = *reinterpret_cast<const unsigned char* const*>(context+0x48);
            if (!raw) return false;
            size_t length=0;
            while(length<128 && raw[length]) ++length;
            if(!length||length==128)return false;
            std::memcpy(output,raw,length);output[length]=0;
            return ascii_id(output,128) && std::strcmp(output,"filehelper") &&
                *reinterpret_cast<const unsigned char* const*>(context+0x48)==raw &&
                !std::memcmp(raw,output,length) &&
                *reinterpret_cast<unsigned char**>(core_address(core_base,0xb912690))==app &&
                *reinterpret_cast<unsigned char**>(app+0x68)==context;
        } __except (EXCEPTION_EXECUTE_HANDLER) { return false; }
    }
    if (core_profile_id == 13) {
        __try {
            auto app = *reinterpret_cast<unsigned char**>(core_address(core_base,0xb912690));
            if (!app) return false;
            auto context = *reinterpret_cast<unsigned char**>(app+0x68);
            if (!context) return false;
            const auto raw = reinterpret_cast<const char*>(context+0x210);
            if (!raw) return false;
            size_t length=0;
            while(length<128 && raw[length]) ++length;
            if(!length||length==128)return false;
            std::memcpy(output,raw,length);output[length]=0;
            return ascii_id(output,128) && std::strcmp(output,"filehelper");
        } __except (EXCEPTION_EXECUTE_HANDLER) { return false; }
    }
    if (core_profile_id == 12) {
        __try {
            auto app = *reinterpret_cast<unsigned char**>(core_address(core_base,0xb912690));
            if (!app) return false;
            auto context = *reinterpret_cast<unsigned char**>(app+0x68);
            if (!context) return false;
            const auto raw = reinterpret_cast<const char*>(context+0x150);
            if (!raw) return false;
            size_t length=0;
            while(length<128 && raw[length]) ++length;
            if(!length||length==128)return false;
            std::memcpy(output,raw,length);output[length]=0;
            return ascii_id(output,128) && std::strcmp(output,"filehelper");
        } __except (EXCEPTION_EXECUTE_HANDLER) { return false; }
    }
        if (!core_base || (core_profile_id != 1 && (core_profile_id!=5&&core_profile_id!=14) && core_profile_id != 6 && core_profile_id != 7 && core_profile_id != 8 && core_profile_id != 9 && core_profile_id != 10 && core_profile_id != 12 && core_profile_id != 13)) return false;
        auto app = *reinterpret_cast<unsigned char**>(core_address(core_base,0xb912690));
        if (!app || *reinterpret_cast<uintptr_t*>(app) != core_address(core_base,0x8d804e8)) return false;
        auto context = *reinterpret_cast<unsigned char**>(app+0x68);
        if (!context || *reinterpret_cast<uintptr_t*>(context) != core_address(core_base,0x8dfe2f8)) return false;
        if (*reinterpret_cast<uintptr_t*>(core_address(core_base,0x8d804e8)+0x60) != core_address(core_base,0x469a0) ||
            *reinterpret_cast<uintptr_t*>(core_address(core_base,0x8dfe2f8)+0x18) != core_address(core_base,0x36f930) ||
            std::memcmp(reinterpret_cast<void*>(core_address(core_base,0x36f930)), "\x48\x8d\x41\x48\xc3", 5)) return false;
        auto str = reinterpret_cast<SmallString*>(context+0x48);
        const SmallString before = *str;
        if (!before.length || before.length > 127 || before.capacity < before.length ||
            !(before.capacity == 15 || (before.capacity >= 16 && before.capacity <= 1048576))) return false;
        auto bytes = before.capacity == 15 ? str->data : *reinterpret_cast<char**>(str->data);
        std::memcpy(output, bytes, before.length);
        output[before.length] = 0;
        return ascii_id(output,128) && std::strcmp(output,"filehelper") &&
            !std::memcmp(str, &before, sizeof(before)) && !std::memcmp(bytes,output,before.length) &&
            *reinterpret_cast<unsigned char**>(core_address(core_base,0xb912690))==app &&
            *reinterpret_cast<unsigned char**>(app+0x68)==context;
    } __except (EXCEPTION_EXECUTE_HANDLER) { return false; }
}
static bool same_account(const char* expected) {
    char current[128]{};
    bool valid = account_snapshot(current) && !std::strcmp(current, expected);
    __try {
        auto app=*reinterpret_cast<unsigned char**>(core_address(core_base,0xb912690));
        if(bound_context && (!app || *reinterpret_cast<uintptr_t*>(app+0x68)!=bound_context)) valid=false;
    } __except(EXCEPTION_EXECUTE_HANDLER) {valid=false;}
    SecureZeroMemory(current,sizeof(current));
    return valid;
}
static bool valid_binding(const WercPacket& p) {
    unsigned char diff = 0;
    for (size_t i=0;i<32;++i) diff |= p.nonce[i] ^ binding.nonce[i];
    return attached && !closing && !diff && p.pid==binding.pid &&
        p.process_start==binding.process_start && p.controller_pid==binding.controller_pid &&
        p.controller_start==binding.controller_start &&
        !std::memcmp(p.account,binding.account,128) &&
        WaitForSingleObject(controller,0)==WAIT_TIMEOUT && same_account(binding.account);
}
static void receipt_set(size_t index, uint32_t value) {
    AcquireSRWLockExclusive(&receipt_lock);
    receipts[index].evidence = value;
    ReleaseSRWLockExclusive(&receipt_lock);
}
struct NativeCompletion {
    LONG refs; uint32_t pad;
    void (__fastcall* invoke)(NativeCompletion*,const unsigned char*);
    void (__fastcall* destroy)(NativeCompletion*);
    size_t receipt_index;
    SharedMessage observed{};
    bool keep_observation=false;
    bool task_value=false;
    bool task_holder=false;
    uint64_t expected_type=1;
    bool verify_body=true;
    char expected_target[128]{}, expected_body[10001]{};
};
static bool native_text_equals(const SmallString*,const char*);
#ifdef WERC_EXACT_TEXT_IDENTITY
static void register_text_result(NativeCompletion*);
static void clear_text_result(size_t);
#endif
#ifdef WERC_EXACT_TEXT_IDENTITY
static uint64_t task_value_message_id(const SharedMessage*,const char*,const char*,uint32_t*,uint64_t=1,bool=true);
static void inspect_completed_text_task(ReceiptSlot*);
#endif
// Candidate identity from this exact factory object, held by a strong reference.
// It is not a delivery proof; the controller must separately read it back.
static uint64_t completion_message_id(const NativeCompletion* self,uint32_t* stage) {
#ifdef WERC_EXACT_TEXT_IDENTITY
    if(self->task_value)return task_value_message_id(&self->observed,self->expected_target,self->expected_body,stage);
#endif
    __try {
        *stage=1;const auto object=static_cast<unsigned char*>(self->observed.object);
        if(!object || !self->observed.control)return 0;
        *stage=2;if(!core_base || (core_profile_id!=1&&(core_profile_id!=5&&core_profile_id!=14)&&core_profile_id!=6&&core_profile_id!=7&&core_profile_id!=8&&core_profile_id!=9&&core_profile_id!=10&&core_profile_id!=11&&core_profile_id!=12&&core_profile_id!=13) || !same_account(binding.account))return 0;
        *stage=3;if(*reinterpret_cast<uintptr_t*>(object)!=core_address(core_base,0x8e53588))return 0;
        *stage=4;if(*reinterpret_cast<uint64_t*>(object+0x118)!=1)return 0;
        *stage=5;if(!native_text_equals(reinterpret_cast<SmallString*>(object+0xb0),self->expected_target))return 0;
        *stage=6;if(!native_text_equals(reinterpret_cast<SmallString*>(object+0x758),self->expected_body))return 0;
        *stage=7;const auto id=*reinterpret_cast<const uint64_t*>(object+0x148);
        if(id&&id==*reinterpret_cast<const uint64_t*>(object+0x148)){*stage=8;return id;}
        return 0;
    } __except(EXCEPTION_EXECUTE_HANDLER) {*stage=9;return 0;}
}
static void __fastcall completed(NativeCompletion* self,const unsigned char* result) {
    _InterlockedIncrement(&active_callbacks);
    __try {
        const bool success=result&&result[0]==1;
        uint32_t stage=0;const uint64_t observed=0;
        self->keep_observation=success&&!observed&&self->observed.object;
        AcquireSRWLockExclusive(&receipt_lock);
        auto& slot=receipts[self->receipt_index];
        #ifdef WERC_EXACT_TEXT_IDENTITY
        if(!slot.observation_stage)slot.observation_stage=stage;
#ifdef WERC_EXACT_TEXT_IDENTITY
        if(slot.observation&&slot.observation->task_holder){
            if(success)inspect_completed_text_task(&slot);
            else{release_completion(slot.observation);slot.observation=nullptr;}
        }
#endif
        self->keep_observation=false;
#else
        slot.observation_stage=stage;
#endif
        if(observed)slot.observed_text_id=observed;
        slot.evidence=success ? ((slot.kind!=11 && slot.kind!=14 && slot.kind!=18) || slot.native_duration==1 ? 2 : 0) : 3;
        ReleaseSRWLockExclusive(&receipt_lock);
    }
    __finally { _InterlockedDecrement(&active_callbacks); }
}
static void release_completion(NativeCompletion* self) {
    TestResult report{};
    guarded_release_shared(&self->observed,&report);
    SecureZeroMemory(self->expected_target,sizeof(self->expected_target));
    SecureZeroMemory(self->expected_body,sizeof(self->expected_body));
    delete self;
}
static void __fastcall destroyed(NativeCompletion* self) {
#ifdef WERC_EXACT_TEXT_IDENTITY
    clear_text_result(self->receipt_index);
#endif
    bool kept=false;
    if(self->keep_observation&&self->observed.object&&!closing) {
        AcquireSRWLockExclusive(&receipt_lock);
        auto& slot=receipts[self->receipt_index];
        if(!slot.observation){slot.observation=self;slot.observation_time=GetTickCount64();kept=true;}
        ReleaseSRWLockExclusive(&receipt_lock);
    }
    if(!kept)release_completion(self);
    _InterlockedDecrement(&live_completions);
}
#include "mention-factory.inc"
#include "text-identity.inc"
#include "media-identity.inc"
static DWORD stop_article_hooks();
static DWORD remove_message_hooks(){
    const auto article_status=stop_article_hooks();if(article_status)return article_status;
#ifdef WERC_EXACT_TEXT_IDENTITY
    const auto task_status=remove_text_task_hook();if(task_status)return task_status;
#endif
    return WxRemoveSelfTest(nullptr);
}
struct SendLease {
    WercPacket packet; size_t index; char members[1024]{},body[10001]{};
    SendLease(const WercPacket& p,size_t i):packet(p),index(i) { _InterlockedIncrement(&live_tasks); }
    ~SendLease() { SecureZeroMemory(body,sizeof(body)); SecureZeroMemory(members,sizeof(members)); SecureZeroMemory(&packet,sizeof(packet)); _InterlockedDecrement(&live_tasks); }
};
static void native_send(const SendLease* lease,QuoteScope* quote=nullptr) {
    // Recheck inside the client's scheduled task, including expiry and identity.
    if (InterlockedCompareExchange(&closing,0,0) || uint64_t(std::time(nullptr)) >= lease->packet.deadline ||
        WaitForSingleObject(controller,0)!=WAIT_TIMEOUT || !same_account(lease->packet.account) || !target_allowed(lease->packet)) {
        receipt_set(lease->index,4); return;
    }
    bool factory_needed=quote || lease->members[0];
#ifdef WERC_EXACT_TEXT_IDENTITY
    factory_needed=true;
#endif
    if(factory_needed && ensure_mention_factory()) {receipt_set(lease->index,4);return;}
#ifdef WERC_EXACT_TEXT_IDENTITY
    if(ensure_text_result()){receipt_set(lease->index,4);return;}
#endif
    struct Request { SmallString target,body; } request{};
    SmallString description{};
    request.target.capacity=request.body.capacity=description.capacity=15;
    if (!set_native_string(&request.target,lease->packet.target) ||
        !set_native_body(&request.body,lease->body[0]?lease->body:lease->packet.text) || !set_inline(&description,"werc.text")) {
        free_native_string(&request.target); free_native_string(&request.body);
        receipt_set(lease->index,4); return;
    }
    auto completion = new(std::nothrow) NativeCompletion{1,0,completed,destroyed,lease->index};
    if (!completion) {
        free_native_string(&request.target); free_native_string(&request.body);
        receipt_set(lease->index,4); return;
    }
    _InterlockedIncrement(&live_completions);
#ifdef WERC_EXACT_TEXT_IDENTITY
    completion->expected_type=quote?((57ull<<32)|49):1;
    completion->verify_body=!quote;
    strcpy_s(completion->expected_target,lease->packet.target);
    strcpy_s(completion->expected_body,lease->body[0]?lease->body:lease->packet.text);
#endif
    MentionScope scope{lease->members,false,lease->index,quote,nullptr};
#ifdef WERC_EXACT_TEXT_IDENTITY
    scope.completion=completion;
#endif
    if(factory_needed && !TlsSetValue(factory_tls,&scope)) {
        destroyed(completion);free_native_string(&request.target);free_native_string(&request.body);receipt_set(lease->index,4);return;
    }
    __try {
        using Builder = void (__fastcall*)(void*,Request*,SmallString*,NativeCompletion**);
        reinterpret_cast<Builder>(core_address(core_base,0x54b02f0))(nullptr,&request,&description,&completion);
    } __except(EXCEPTION_EXECUTE_HANDLER) { receipt_set(lease->index,0); }
    if(factory_needed)TlsSetValue(factory_tls,nullptr);
    if (completion) destroyed(completion);
    free_native_string(&request.target); free_native_string(&request.body);
}
static void guarded_native_send(const SendLease* lease) {
    __try { native_send(lease); }
    __except(EXCEPTION_EXECUTE_HANDLER) { receipt_set(lease->index,0); }
}
static DWORD submit(WercPacket& p) {
    const size_t length = strnlen_s(p.text,sizeof(p.text));
    if (!target_allowed(p) || !ascii_id(p.attempt,sizeof(p.attempt)) ||
        !digest_valid(p.digest) || !length || length==sizeof(p.text) ||
        !MultiByteToWideChar(CP_UTF8,MB_ERR_INVALID_CHARS,p.text,int(length),nullptr,0) ||
        p.deadline <= uint64_t(std::time(nullptr)) || p.deadline > uint64_t(std::time(nullptr))+300)
        return ERROR_INVALID_PARAMETER;
    // Repeated attempt queries never invoke the native builder again.
    for (size_t i=0;i<receipt_count;++i) {
        if (!std::strcmp(receipts[i].attempt,p.attempt)) {
            if (receipts[i].kind || std::strcmp(receipts[i].digest,p.digest) || std::strcmp(receipts[i].target,p.target)) return ERROR_INVALID_DATA;
            AcquireSRWLockShared(&receipt_lock); p.receipt=receipts[i].evidence; ReleaseSRWLockShared(&receipt_lock);
            return ERROR_SUCCESS;
        }
    }
    if (live_tasks || live_completions || !reserve_receipt()) return ERROR_BUSY;
    const size_t index=receipt_count++;
    strcpy_s(receipts[index].target,p.target); strcpy_s(receipts[index].attempt,p.attempt); strcpy_s(receipts[index].digest,p.digest);
    receipt_set(index,1);
    auto lease=std::make_shared<SendLease>(p,index);
    std::function<void()> task=[lease]() {
        _InterlockedIncrement(&active_callbacks);
        __try { guarded_native_send(lease.get()); }
        __finally { _InterlockedDecrement(&active_callbacks); }
    };
    TestConfig config{};
    config.size=sizeof(config); config.pid=GetCurrentProcessId();
    const DWORD status=schedule_task(config,&task);
    if (status) receipt_set(index,0); // Ambiguous scheduler failure, never resend.
    AcquireSRWLockShared(&receipt_lock); p.receipt=receipts[index].evidence; ReleaseSRWLockShared(&receipt_lock);
    return status;
}
#include "mention-stage.inc"
#include "voice.inc"
#include "long-text.inc"
#include "quote.inc"
#include "image.inc"
#include "file.inc"
#include "moments.inc"
#include "moments-stage.inc"
#include "moments-interaction.inc"
#include "moments-interaction-stage.inc"
#include "video.inc"
#include "emoticon.inc"
#include "download.inc"
#include "articles.inc"
#include "settings.inc"
static DWORD stop_article_hooks(){return article_native::stop();}
static DWORD WINAPI watch_controller(void*) {
    HANDLE handles[]={stop_event,controller};
    for (;;) {
        const auto wait=WaitForMultipleObjects(2,handles,FALSE,500);
        if(wait==WAIT_OBJECT_0) break;
        AcquireSRWLockExclusive(&api_lock);
        const bool stop=wait==WAIT_OBJECT_0+1 || wait==WAIT_FAILED || !same_account(binding.account);
        const bool interaction_pending=interaction_tick(stop);
        if(!stop) {ReleaseSRWLockExclusive(&api_lock);continue;}
        if(interaction_pending){ReleaseSRWLockExclusive(&api_lock);Sleep(100);continue;}
        InterlockedExchange(&closing,1);clear_targets();
        moment_stage.reset();quote_stage.reset();long_text_stage.reset();mention_stage.reset();voice_stage.reset();image_stage.reset();file_stage.reset();video_stage.reset();emoticon_stage.reset();
        if(download_result)InterlockedExchange(&download_result->cancelled,1);download_result.reset();
        // On parent death disable hooks. Keep an inert module mapped rather
        // than unmapping code which a native callback might still reference.
        remove_message_hooks();
        if (wait==WAIT_OBJECT_0+1 && !live_tasks && !live_completions) {
            SecureZeroMemory(&binding,sizeof(binding));
            clear_receipts();
            // Clear the orphan's last packet but retain its allocation: an old
            // remote caller may still hold that address after controller death.
            __try {if(transport_packet) SecureZeroMemory(transport_packet,sizeof(WercPacket));}
            __except(EXCEPTION_EXECUTE_HANDLER) {}
            transport_packet=nullptr;
        }
        AcquireSRWLockExclusive(&receive_lock);
        SecureZeroMemory(received_messages,sizeof(received_messages));
        SecureZeroMemory(&receive_inflight,sizeof(receive_inflight));
        receive_head=receive_count=0;
        ReleaseSRWLockExclusive(&receive_lock);
        ReleaseSRWLockExclusive(&api_lock);
        break;
    }
    return 0;
}
static DWORD receive_voice_chunk(WercPacket& p) {
    AcquireSRWLockShared(&receive_lock);DWORD status=ERROR_INVALID_STATE;
    if(receive_inflight.present && receive_inflight.type==34 && p.sequence==receive_inflight.sequence &&
       receive_inflight.voice_length<=sizeof(receive_inflight.voice_bytes) && p.dropped<receive_inflight.voice_length){
        const auto count=std::min<size_t>(sizeof(p.text),receive_inflight.voice_length-p.dropped);
        SecureZeroMemory(p.text,sizeof(p.text));std::memcpy(p.text,receive_inflight.voice_bytes+p.dropped,count);
        p.present=static_cast<uint32_t>(count);p.receipt=34;p.deadline=receive_inflight.voice_length;
        p.message_id=receive_inflight.native_message_id;status=0;
    }
    ReleaseSRWLockShared(&receive_lock);return status;
}
static DWORD receive_chunk(WercPacket& p) {
    AcquireSRWLockShared(&receive_lock);
    DWORD status=ERROR_INVALID_STATE;
    const size_t length=strnlen_s(receive_inflight.text,sizeof(receive_inflight.text));
    if(receive_inflight.present && p.sequence==receive_inflight.sequence && p.dropped<length && length<sizeof(receive_inflight.text)) {
        const size_t count=(std::min)(sizeof(p.text),length-p.dropped);
        SecureZeroMemory(p.text,sizeof(p.text));
        std::memcpy(p.text,receive_inflight.text+p.dropped,count);
        p.present=static_cast<uint32_t>(count);p.receipt=receive_inflight.type;
        p.deadline=length;p.message_id=receive_inflight.native_message_id;status=0;
    }
    ReleaseSRWLockShared(&receive_lock);return status;
}
static DWORD poll_packet(WercPacket& p) {
        auto storage=std::make_unique<ReceivedMessage>();auto& message=*storage;
        const auto result=poll_received(&message);
        p.present=message.present;p.dropped=message.dropped;p.sequence=message.sequence;p.message_id=message.native_message_id;
        if (message.present) {
            const char* conversation=receive_conversation(binding.account,message.sender,message.receiver);
            if(!conversation){SecureZeroMemory(&message,sizeof(message));return ERROR_INVALID_DATA;}
            p.receipt=message.type;p.deadline=std::strlen(message.text);p.reserved=message.voice_length;
            SecureZeroMemory(p.text,sizeof(p.text));strcpy_s(p.target,conversation);
            strcpy_s(p.text,128,message.sender);strcpy_s(p.text+128,128,message.receiver);
        }
        SecureZeroMemory(&message,sizeof(message));return result;
}
static DWORD dispatch(WercPacket& p) {
    const auto op=static_cast<WercOp>(p.operation);
    if(op==WercOp::Recover) {
        if(p.pid!=GetCurrentProcessId() || p.process_start!=creation_time(GetCurrentProcess())) return ERROR_INVALID_PARAMETER;
        if(!attached && recovered_resident && !watcher && !controller) return 0;
        // The owning controller must have died; a live session or pending native
        // task cannot be replaced. A timed-out old remote call may still exist,
        // so recovery keeps this exact image resident for the process lifetime.
        if(!attached || !closing || !controller || !watcher ||
           WaitForSingleObject(controller,0)!=WAIT_OBJECT_0 ||
           WaitForSingleObject(watcher,0)!=WAIT_OBJECT_0 ||
           live_tasks || live_completions || active_callbacks) return ERROR_BUSY;
        if(remove_message_hooks()) return ERROR_BUSY;
        CloseHandle(controller);CloseHandle(stop_event);CloseHandle(watcher);
        controller=stop_event=watcher=nullptr;attached=false;bound_context=0;receipt_count=0;
        SecureZeroMemory(&binding,sizeof(binding));clear_receipts();
        recovered_resident=true;return 0;
    }
    if (op==WercOp::Inspect || op==WercOp::Attach) {
        if (p.pid!=GetCurrentProcessId() || p.process_start!=creation_time(GetCurrentProcess())) return ERROR_INVALID_PARAMETER;
        const auto core=GetModuleHandleW(L"Weixin.dll");
        if (!core || !core_fingerprint(core) || (core_profile_id!=1&&(core_profile_id!=5&&core_profile_id!=14)&&core_profile_id!=6&&core_profile_id!=7&&core_profile_id!=8&&core_profile_id!=9&&core_profile_id!=10&&core_profile_id!=11&&core_profile_id!=12&&core_profile_id!=13)) return ERROR_REVISION_MISMATCH;
        core_base=reinterpret_cast<uintptr_t>(core);
        char account[128]{};
        if (!account_snapshot(account)) return ERROR_NOT_LOGGED_ON;
        if (op==WercOp::Inspect) { std::memcpy(p.account,account,128); return 0; }
        if (attached || std::memcmp(account,p.account,128)) return ERROR_INVALID_STATE;
        unsigned char any=0; for (auto c:p.nonce) any|=c;
        if (!any || !p.controller_pid || p.controller_pid==p.pid) return ERROR_INVALID_PARAMETER;
        controller=OpenProcess(SYNCHRONIZE|PROCESS_QUERY_LIMITED_INFORMATION,FALSE,p.controller_pid);
        if (!controller || creation_time(controller)!=p.controller_start) {
            if(controller) CloseHandle(controller); controller=nullptr; return ERROR_ACCESS_DENIED;
        }
        stop_event=CreateEventW(nullptr,TRUE,FALSE,nullptr);
        if (!stop_event) { CloseHandle(controller); controller=nullptr; return ERROR_NOT_ENOUGH_MEMORY; }
        binding=p; transport_packet=&p; InterlockedExchange(&closing,0);
        __try {
            auto app=*reinterpret_cast<unsigned char**>(core_address(core_base,0xb912690));
            bound_context=app ? *reinterpret_cast<uintptr_t*>(app+0x68) : 0;
        } __except(EXCEPTION_EXECUTE_HANDLER) {bound_context=0;}
        if(!bound_context) {CloseHandle(controller);CloseHandle(stop_event);controller=stop_event=nullptr;return ERROR_NOT_LOGGED_ON;}
        TestConfig config{}; config.size=sizeof(config); config.pid=p.pid;
        strcpy_s(receive_login,account);
        const DWORD status=WxStartFileHelperReceiver(&config);
        if (status) { CloseHandle(controller);CloseHandle(stop_event);controller=stop_event=nullptr;return status; }
        attached=true;
        watcher=CreateThread(nullptr,0,watch_controller,nullptr,0,nullptr);
        if (!watcher) { remove_message_hooks();attached=false;CloseHandle(controller);CloseHandle(stop_event);controller=stop_event=nullptr;return ERROR_NOT_ENOUGH_MEMORY; }
        return 0;
    }
    // Close remains possible following logout or account change; it still
    // requires the original controller nonce and PID creation identity.
    if (op==WercOp::Close) {
        if (!attached || std::memcmp(p.nonce,binding.nonce,32) ||
            p.controller_pid!=binding.controller_pid || p.controller_start!=binding.controller_start) return ERROR_ACCESS_DENIED;
        InterlockedExchange(&closing,1);clear_targets();
        const auto result=remove_message_hooks();
        if (result) return result;
        SetEvent(stop_event);
        // Do not join under api_lock: a dying-controller watcher may be waiting
        // for that lock. Host observes status=0 then calls WercJoin before unload.
        return 0;
    }
    if (!valid_binding(p)) return ERROR_ACCESS_DENIED;
    // Feature gates run before every native dispatcher, including settings.
    if (!core_operation_supported(p.operation)) return ERROR_NOT_SUPPORTED;
    if (p.operation==70||p.operation==71)return native_settings::dispatch(p);
    if (p.operation>=63&&p.operation<=69)return article_native::dispatch(p);
    if (p.operation==58 || p.operation==59)return interaction_dispatch(p);
    if ((p.operation>=50 && p.operation<=56) || (p.operation>=60 && p.operation<=62)) return moments_dispatch(p);
    if (p.operation>=45 && p.operation<=48) return quote_dispatch(p);
    if (p.operation>=41 && p.operation<=44) return long_text_dispatch(p);
    if (p.operation>=38 && p.operation<=40) return mention_dispatch(p);
    if (p.operation>=9 && p.operation<=12) return voice_dispatch(p);
    if (p.operation>=13 && p.operation<=16) return image_dispatch(p);
    if (p.operation>=29 && p.operation<=32) return video_dispatch(p);
    if(p.operation>=33 && p.operation<=36)return emoticon_dispatch(p);
    if (p.operation>=17 && p.operation<=20) return file_dispatch(p);
    if((p.operation>=22 && p.operation<=25) || op==WercOp::DownloadFileBegin || op==WercOp::DownloadVideoBegin || op==WercOp::DownloadOriginalImageBegin || op==WercOp::DownloadFavoriteBegin)return download_dispatch(p);
    if (op==WercOp::ReceiveVoiceChunk) return receive_voice_chunk(p);
    if (op==WercOp::ReceiveChunk) return receive_chunk(p);
    if (op==WercOp::PrepareTarget) return prepare_target(p);
    if (op==WercOp::Send) return submit(p);
    if (op==WercOp::Ack) return ack_received(p.sequence);
    if (op==WercOp::Receipt) {
        if (!ascii_id(p.attempt,sizeof(p.attempt)) || !target_id_valid(p.target)) return ERROR_INVALID_PARAMETER;
        for (size_t i=0;i<receipt_count;++i) if (!std::strcmp(receipts[i].attempt,p.attempt)) {
            if(std::strcmp(receipts[i].target,p.target))return ERROR_INVALID_DATA;
            AcquireSRWLockExclusive(&receipt_lock);
            auto& slot=receipts[i];
            if(slot.observation&&!slot.observation->task_holder) {
                slot.observed_text_id=completion_message_id(slot.observation,&slot.observation_stage);
                if(slot.observed_text_id || GetTickCount64()-slot.observation_time>60000) {release_completion(slot.observation);slot.observation=nullptr;}
            }
            p.receipt=slot.evidence;p.dropped=slot.native_duration;p.message_id=slot.message_id;p.present=slot.kind;
            p.sequence=slot.observed_text_id;p.reserved=slot.observation_stage;
#ifdef WERC_EXACT_TEXT_IDENTITY
            sprintf_s(p.text,"{\"local\":%u,\"client\":%llu,\"millis\":%llu,\"seconds\":%u,\"message_type\":%llu}",slot.task_local,slot.task_client,slot.task_millis,slot.task_seconds,slot.task_type);
#endif
            ReleaseSRWLockExclusive(&receipt_lock);return 0;
        }
        p.receipt=0;return ERROR_NOT_FOUND;
    }
    if (op==WercOp::Poll) return poll_packet(p);
    return ERROR_INVALID_FUNCTION;
}
static DWORD dispatch_noexcept(WercPacket& p) noexcept {
    try {return dispatch(p);} catch(...) {return ERROR_NOT_ENOUGH_MEMORY;}
}
extern "C" __declspec(dllexport) DWORD WINAPI WercCall(void* data) {
    if (!data) return ERROR_INVALID_PARAMETER;
    auto p=static_cast<WercPacket*>(data);
    if (p->size!=sizeof(WercPacket) || p->version!=WercProtocolVersion || p->reserved) return ERROR_REVISION_MISMATCH;
    AcquireSRWLockExclusive(&api_lock);
    DWORD status=ERROR_INVALID_DATA;
    __try {
        __try {status=dispatch_noexcept(*p);}
        __except(EXCEPTION_EXECUTE_HANDLER) {status=ERROR_INVALID_DATA;}
        p->status=status;
    }
    __finally { ReleaseSRWLockExclusive(&api_lock); }
    return status;
}
extern "C" __declspec(dllexport) DWORD WINAPI WercJoin(void*) {
    if (!closing || !watcher) return ERROR_INVALID_STATE;
    if (WaitForSingleObject(watcher,6000)!=WAIT_OBJECT_0) return ERROR_BUSY;
    if (WxWaitForTasks(nullptr)) return ERROR_BUSY;
    CloseHandle(watcher);CloseHandle(controller);CloseHandle(stop_event);
    watcher=controller=stop_event=nullptr;attached=false;
    transport_packet=nullptr;
    moment_stage.reset();quote_stage.reset();long_text_stage.reset();mention_stage.reset();voice_stage.reset();image_stage.reset();file_stage.reset();video_stage.reset();emoticon_stage.reset();
        if(download_result)InterlockedExchange(&download_result->cancelled,1);download_result.reset();
    SecureZeroMemory(receive_login,sizeof(receive_login));SecureZeroMemory(&binding,sizeof(binding));clear_receipts();
    AcquireSRWLockExclusive(&receive_lock);
    SecureZeroMemory(received_messages,sizeof(received_messages));SecureZeroMemory(&receive_inflight,sizeof(receive_inflight));
    receive_head=receive_count=0;
    ReleaseSRWLockExclusive(&receive_lock);
    receipt_count=0;
    return 0;
}

















