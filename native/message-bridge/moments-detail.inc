// Profile 1 native SNS detail fallback. The local timeline omits some owned
// private posts. Use the same service entry as the client's detail task; never
// reconstruct an SNS object or alter its database. Include inside the interaction
// namespace after Lease/account_ok, and invoke only from a retained scheduler task.
struct DetailResult {
    int32_t error_type, error_code;
    SmallString error;
    SharedMessage value; // shared_ptr<shared_ptr<SnsItem>>
};
static_assert(sizeof(DetailResult) == 56 && offsetof(DetailResult, value) == 0x28,
              "SNS detail result ABI");

static Snapshot remote_detail(Lease* lease) {
    if (lease->remote_started || !account_ok(lease) || lease->cancel ||
        GetTickCount64() >= lease->deadline) return SNAP_REJECTED;
    // Keep the native result in the lease on an exception. Native code may own
    // an asynchronous request even when its synchronous facade fails to return.
    auto result = static_cast<DetailResult*>(HeapAlloc(GetProcessHeap(), HEAP_ZERO_MEMORY, sizeof(DetailResult)));
    if (!result) return SNAP_REJECTED;
    lease->remote_context = result;
    Snapshot outcome = SNAP_INVALID;
    __try {
        static const unsigned char prefix[16] = {
            0x55,0x41,0x57,0x41,0x56,0x41,0x55,0x41,0x54,0x56,0x57,0x53,0x48,0x81,0xec,0xd8};
        if ((core_profile_id != 1 && (core_profile_id!=5&&core_profile_id!=14)) ||
            std::memcmp(reinterpret_cast<const void*>(core_address(core_base,0x2fd0fa0)), prefix, sizeof(prefix))) {
            HeapFree(GetProcessHeap(), 0, result); lease->remote_context = nullptr;
            return SNAP_INVALID;
        }
        // 0x2cc4200 is the client's own scheduled local-then-remote detail task.
        // This facade cooperatively awaits the native request; the caller's
        // ticket and references remain pinned until it returns, including cancel.
        using Detail = DetailResult* (__fastcall*)(void*, DetailResult*, uint64_t, bool);
        lease->remote_started = true;
        reinterpret_cast<Detail>(core_address(core_base,0x2fd0fa0))(
            lease->service.object, result, lease->config.sns_id, false);
        lease->remote_returned = true;
        if (result->value.object && result->value.control &&
            result->value.object == static_cast<unsigned char*>(result->value.control)+0x10 &&
            *reinterpret_cast<uintptr_t*>(result->value.control) == core_address(core_base,0x9400128)) {
            const auto item = static_cast<const SharedMessage*>(result->value.object);
            if (!result->error_type && !result->error_code && !item->object && !item->control)
                outcome = SNAP_MISSING;
            else if (!result->error_type && !result->error_code && item->object && item->control &&
                *reinterpret_cast<const uint64_t*>(item->object) == lease->config.sns_id &&
                *reinterpret_cast<uintptr_t*>(item->control) == core_address(core_base,0x932ce68) &&
                sns_interaction_call::retain_owned(&lease->item, item)) outcome = SNAP_FOUND;
            else if (result->error_type || result->error_code) outcome = SNAP_REJECTED;
        }
        TestResult cleanup{};
        if (!sns_interaction_call::clean_string(&result->error, &cleanup) ||
            !release_once(&result->value)) return SNAP_UNCERTAIN;
        if (!HeapFree(GetProcessHeap(), 0, result)) return SNAP_UNCERTAIN;
        lease->remote_context = nullptr;
        return outcome;
    } __except (EXCEPTION_EXECUTE_HANDLER) {
        _InterlockedExchange(&lease->exception, GetExceptionCode());
        return SNAP_UNCERTAIN;
    }
}
