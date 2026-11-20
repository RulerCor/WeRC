// Helper-derived text factory adapter, scoped to one scheduled task by TLS.
// See ../helper/LICENSE. Exact profile validation remains in connection setup.
#include <intrin.h>
struct QuoteScope { SharedMessage prepared{}; unsigned char* object=nullptr; bool applied=false; size_t receipt_index=size_t(-1); };
struct MentionScope { const char* members; bool applied; size_t receipt_index = size_t(-1); QuoteScope* quote=nullptr; NativeCompletion* completion=nullptr; };
using MentionFactory = SharedMessage* (__fastcall*)(SharedMessage*);
static MentionFactory original_mention_factory = nullptr;
static bool mention_members_valid(const char* value) {
    const auto length=strnlen_s(value,1024);
    if(!length || length==1024)return false;
    unsigned count=0;size_t start=0;
    for(size_t i=0;i<=length;++i) {
        if(i==length || value[i]==',') {
            const auto part=i-start;
            if(!part || part>=128 || ++count>8)return false;
            // Repeated IDs cannot turn a short selection into repeated notifications.
            for(size_t prior=0;prior<start;) {
                size_t end=prior;while(end<start&&value[end]!=',')++end;
                if(end-prior==part&&!std::memcmp(value+prior,value+start,part))return false;
                prior=end+1;
            }
            start=i+1;
        } else {
            const unsigned char c=value[i];
            if(!((c>='a'&&c<='z')||(c>='A'&&c<='Z')||(c>='0'&&c<='9')||c=='_'||c=='-'||c=='.'))return false;
        }
    }
    return true;
}
static bool mention_target_valid(const char* target) {
    const auto n=strnlen_s(target,128);
    return target_id_valid(target)&&n>9&&!std::strcmp(target+n-9,"@chatroom");
}
static bool native_text_equals(const SmallString* text,const char* value) {
    const auto n=std::strlen(value);
    if(text->length!=n || text->capacity<n || text->capacity>1048576)return false;
    const auto bytes=text->capacity<=15?text->data:*reinterpret_cast<const char*const*>(text->data);
    return bytes&&!std::memcmp(bytes,value,n);
}
static void apply_mention(SharedMessage* output,MentionScope* scope) {
    auto object=output?static_cast<unsigned char*>(output->object):nullptr;
    if(!object || scope->applied || *reinterpret_cast<uintptr_t*>(object)!=core_address(core_base,0x8e53588))
        RaiseException(ERROR_INVALID_DATA,0,0,nullptr);
    auto members=reinterpret_cast<SmallString*>(object+0x778);
    if(members->length || members->capacity!=15 || !set_native_string(members,scope->members) || !native_text_equals(members,scope->members))
        RaiseException(ERROR_INVALID_DATA,0,0,nullptr);
    scope->applied=true;
    if(scope->receipt_index<receipts.size()) {
        AcquireSRWLockExclusive(&receipt_lock);
        receipts[scope->receipt_index].native_duration=1; // Mention field verified, not video duration.
        ReleaseSRWLockExclusive(&receipt_lock);
    }
}
static SharedMessage* __fastcall mention_factory(SharedMessage* output) {
    _InterlockedIncrement(&active_callbacks);SharedMessage* result=nullptr;
    __try {
        const auto scope=factory_tls==TLS_OUT_OF_INDEXES?nullptr:static_cast<MentionScope*>(TlsGetValue(factory_tls));
        if(scope&&scope->quote&&reinterpret_cast<uintptr_t>(_ReturnAddress())==core_address(core_base,0x54b0425)) {
            auto& quote=*scope->quote;
            if(!output||!quote.prepared.object||!quote.prepared.control||quote.applied)
                RaiseException(ERROR_INVALID_DATA,0,0,nullptr);
            *output=quote.prepared;quote.prepared={};result=output;
        } else {
            result=original_mention_factory(output);
            if(scope&&scope->members&&scope->members[0])apply_mention(output,scope);
        }
        if(scope&&scope->completion&&reinterpret_cast<uintptr_t>(_ReturnAddress())==core_address(core_base,0x54b0425)) {
            auto& held=scope->completion->observed;
            if(!held.object&&result==output&&output&&output->object&&output->control&&
               *reinterpret_cast<uintptr_t*>(output->object)==core_address(core_base,0x8e53588)) {
                _InterlockedIncrement(reinterpret_cast<volatile LONG*>(static_cast<unsigned char*>(output->control)+8));
                held=*output;
#ifdef WERC_EXACT_TEXT_IDENTITY
                register_text_result(scope->completion);
#endif
            }
        }
    } __finally {_InterlockedDecrement(&active_callbacks);}
    return result;
}
static DWORD ensure_mention_factory() {
    if(factory_hook_created)return 0;
    if(!core_base || (core_profile_id!=1&&core_profile_id!=5&&core_profile_id!=11&&core_profile_id!=12&&core_profile_id!=13))return ERROR_REVISION_MISMATCH;
    factory_tls=TlsAlloc();if(factory_tls==TLS_OUT_OF_INDEXES)return ERROR_NOT_ENOUGH_MEMORY;
    auto target=reinterpret_cast<void*>(core_address(core_base,0x6f4c70));
    auto status=MH_CreateHook(target,reinterpret_cast<void*>(&mention_factory),reinterpret_cast<void**>(&original_mention_factory));
    if(status==MH_OK){status=MH_EnableHook(target);if(status!=MH_OK)MH_RemoveHook(target);}
    if(status!=MH_OK){TlsFree(factory_tls);factory_tls=TLS_OUT_OF_INDEXES;return 600+status;}
    factory_hook_created=true;return 0;
}



