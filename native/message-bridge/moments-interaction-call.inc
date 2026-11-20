#pragma once
#include <climits>
#include <cstddef>
// Version-specific SNS call adapter. Include after text_bridge.cpp has defined
// SharedMessage, SmallString, image_free_string and the native allocators.
// The caller must hold the account/session, service and post strong references,
// run in the verified scheduler context and quarantine the DLL on uncertainty.
// No caller may retry a submitted operation merely because it timed out or failed.
#ifndef WXBOT_ENABLE_SNS_INTERACTION
#define WXBOT_ENABLE_SNS_INTERACTION 0
#endif

namespace sns_interaction_call {

struct Vector { void* begin; void* end; void* capacity; };
struct CommentPayload { SmallString text; Vector extra_a; Vector extra_b; };
static_assert(sizeof(SharedMessage) == 16 && sizeof(SmallString) == 32, "native ABI");
static_assert(sizeof(Vector) == 24 && sizeof(CommentPayload) == 80, "comment ABI");
static_assert(offsetof(CommentPayload, extra_a) == 0x20 &&
              offsetof(CommentPayload, extra_b) == 0x38, "comment offsets");

struct Result {
    DWORD status;             // Wrapper/cleanup status, not server result.
    DWORD exception;          // Structured exception, zero if none was observed.
    bool attempted;           // Entry was about to be called; outcome may be unknown.
    bool native_known;        // Function returned normally.
    bool native_value;        // High-level bool; not a server receipt.
    bool quarantine;          // Caller must pin bridge and prevent unload/retry.
    void* retained_context;   // Opaque allocation kept alive in quarantine; never free blindly.
};

struct Context {
    SharedMessage transferred_post;
    SharedMessage transferred_reply;
    CommentPayload comment;
    SmallString error;
    TestResult cleanup;
};

// Borrowed from a retained SnsItem. Match both supplied IDs and the author,
// never comment text or position. The native constructor is 0x42e1b80:
// proto+2c -> id+0, proto+48 -> id64+8, proto+8 -> username+10.
static const SharedMessage* find_reply(const SharedMessage* item, uint32_t id,
                                      uint64_t id64, const char* username) {
    if (!item || !item->object || !username || !*username || (!id && !id64)) return nullptr;
    const auto list = reinterpret_cast<const Vector*>(static_cast<const unsigned char*>(item->object)+0x70);
    const auto begin = reinterpret_cast<uintptr_t>(list->begin);
    const auto end = reinterpret_cast<uintptr_t>(list->end);
    const auto capacity = reinterpret_cast<uintptr_t>(list->capacity);
    if (!begin || end < begin || capacity < end || (end-begin)%sizeof(SharedMessage) ||
        (end-begin)/sizeof(SharedMessage)>100000) return nullptr;
    const SharedMessage* found = nullptr;
    for (auto address=begin;address<end;address+=sizeof(SharedMessage)) {
        const auto value = reinterpret_cast<const SharedMessage*>(address);
        if (!value->object || !value->control ||
            value->object != static_cast<const unsigned char*>(value->control)+0x10 ||
            *reinterpret_cast<const uintptr_t*>(value->control)!=core_address(core_base,0x93ff9d8)) return nullptr;
        const auto object = static_cast<const unsigned char*>(value->object);
        if ((!id || *reinterpret_cast<const uint32_t*>(object)==id) &&
            (!id64 || *reinterpret_cast<const uint64_t*>(object+8)==id64) &&
            native_text_equals(reinterpret_cast<const SmallString*>(object+0x10),username)) {
            if (found) return nullptr;
            found=value;
        }
    }
    return found;
}

static bool fingerprint_anchors(unsigned action) {
    static const unsigned char like[16] = {
        0x55,0x41,0x56,0x56,0x57,0x53,0x48,0x81,0xec,0x20,0x02,0,0,0x48,0x8d,0xac};
    static const unsigned char comment[16] = {
        0x55,0x41,0x57,0x41,0x56,0x41,0x55,0x41,0x54,0x56,0x57,0x53,0x48,0x81,0xec,0xe8};
    static const unsigned char validate[16] = {
        0x48,0x83,0x39,0,0x74,0x66,0x48,0x83,0xb9,0xa0,1,0,0,0,0x74,0x5c};
    static const unsigned char unlike[16] = {
        0x55,0x56,0x57,0x48,0x81,0xec,0xa0,0x01,0,0,0x48,0x8d,0xac,0x24,0x80,0};
    static const unsigned char remove[16] = {
        0x55,0x41,0x57,0x41,0x56,0x41,0x54,0x56,0x57,0x53,0x48,0x81,0xec,0x10,0x02,0};
    if (action == 5) action=2;
    if (action < 1 || action > 4) return false;
    const uintptr_t entry = action == 1 ? 0x30640f0 : action == 2 ? 0x3065ac0 : action == 3 ? 0x2ff8bc0 : 0x2ffb740;
    const unsigned char* prefix = action == 1 ? like : action == 2 ? comment : action == 3 ? unlike : remove;
    return core_base && (core_profile_id==1 || core_profile_id==5) &&
        std::memcmp(reinterpret_cast<const void*>(interaction_address(core_base, entry)), prefix, 16) == 0 &&
        std::memcmp(reinterpret_cast<const void*>(interaction_address(core_base, 0x43b1800)), validate, 16) == 0;
}

// The input strong reference must remain alive throughout this operation. CAS
// refuses zero/resurrection and overflow; it does not make a stale pointer safe.
static bool retain_owned(SharedMessage* destination, const SharedMessage* source) {
    auto count = reinterpret_cast<volatile LONG*>(static_cast<unsigned char*>(source->control) + 8);
    LONG old = InterlockedCompareExchange(count, 0, 0);
    for (unsigned attempt = 0; attempt < 128; ++attempt) {
        if (old <= 0 || old == LONG_MAX) return false;
        const LONG seen = InterlockedCompareExchange(count, old + 1, old);
        if (seen == old) { *destination = *source; return true; }
        old = seen;
    }
    return false;
}

// image_free_string supports the native aligned allocation path. It silently
// skips huge capacities, so validate beforehand and quarantine on any anomaly.
static bool clean_string(SmallString* value, TestResult* cleanup) {
    if (value->capacity < 15 || value->capacity >= 1024 * 1024 ||
        value->length > value->capacity || (value->capacity == 15 && value->length > 15) ||
        (value->capacity > 15 && !*reinterpret_cast<void**>(value->data))) return false;
    image_free_string(value, cleanup);
    if (cleanup->exception) return false;
    *value = {};
    value->capacity = 15;
    return true;
}

// `expected` is the already selected 64-bit SNS id; never emit it in logs.
// The caller must independently check the exact on-disk fingerprint and account.
static Result run(const SharedMessage* service, const SharedMessage* item,
                  unsigned action, const char* text, uint64_t expected, const char* account,
                  uint32_t reply_id, uint64_t reply_id64, const char* reply_user) {
    Result out{};
    out.status = ERROR_INVALID_PARAMETER;
#if !WXBOT_ENABLE_SNS_INTERACTION
    (void)service; (void)item; (void)action; (void)text; (void)expected; (void)account;
    (void)reply_id; (void)reply_id64; (void)reply_user;
    out.status = ERROR_NOT_SUPPORTED;
    return out;
#else
    if (!service || !item || !expected || action < 1 || action > 5) return out;
    auto ctx = static_cast<Context*>(HeapAlloc(GetProcessHeap(), HEAP_ZERO_MEMORY, sizeof(Context)));
    if (!ctx) { out.status = ERROR_NOT_ENOUGH_MEMORY; return out; }
    ctx->comment.text.capacity = 15;
    ctx->error.capacity = 15;
    bool transferred = false;
    __try {
        if (!fingerprint_anchors(action)) { out.status = ERROR_REVISION_MISMATCH; __leave; }
        if (!service->object || !service->control || !item->object || !item->control ||
            *reinterpret_cast<uintptr_t*>(service->object) != interaction_address(core_base, 0x95e00d8) ||
            *reinterpret_cast<const uint64_t*>(item->object) != expected) {
            out.status = ERROR_INVALID_DATA; __leave;
        }
        if (action == 2 || action == 5) {
            if (!text) { out.status = ERROR_INVALID_PARAMETER; __leave; }
            const size_t bytes = strnlen_s(text, 1024);
            if (!bytes || bytes >= 1024 ||
                !MultiByteToWideChar(CP_UTF8, MB_ERR_INVALID_CHARS, text,
                                     static_cast<int>(bytes), nullptr, 0) ||
                !set_native_string(&ctx->comment.text, text)) {
                out.status = ERROR_INVALID_DATA; __leave;
            }
        }
        // Native Delete also checks this author. Reject before entering it.
        if (action == 4 && (!account || !native_text_equals(
            reinterpret_cast<const SmallString*>(static_cast<const unsigned char*>(item->object)+0x190), account))) {
            out.status = ERROR_ACCESS_DENIED; __leave;
        }
        const SharedMessage* reply = nullptr;
        if (action == 5) {
            reply=find_reply(item,reply_id,reply_id64,reply_user);
            if (!reply) { out.status=ERROR_NOT_FOUND; __leave; }
        }
        if (!retain_owned(&ctx->transferred_post, item)) {
            out.status = ERROR_NOT_READY; __leave;
        }
        transferred = true;
        if (reply && !retain_owned(&ctx->transferred_reply,reply)) {
            guarded_release_shared(&ctx->transferred_post,&ctx->cleanup);
            transferred=ctx->cleanup.exception!=0;
            out.status=ERROR_NOT_READY; __leave;
        }
        // From this point an exception leaves ownership ambiguous.  Keep the
        // complete context, including both pairs and strings, for quarantine.
        out.attempted = true;
        if (action == 1) {
            using Like = bool (__fastcall*)(void*, SharedMessage*, SmallString*);
            out.native_value = reinterpret_cast<Like>(interaction_address(core_base, 0x30640f0))(
                service->object, &ctx->transferred_post, &ctx->error);
        } else if (action == 2 || action == 5) {
            using Comment = bool (__fastcall*)(void*, SharedMessage*, const CommentPayload*,
                                               SharedMessage*, SmallString*);
            out.native_value = reinterpret_cast<Comment>(interaction_address(core_base, 0x3065ac0))(
                service->object, &ctx->transferred_post, &ctx->comment,
                &ctx->transferred_reply, &ctx->error);
        } else {
            using Operation = bool (__fastcall*)(void*, SharedMessage*);
            out.native_value = reinterpret_cast<Operation>(interaction_address(core_base,
                action == 3 ? 0x2ff8bc0 : 0x2ffb740))(service->object, &ctx->transferred_post);
        }
        // Both input pairs were consumed by the native function. Do not release.
        ctx->transferred_post = {};
        ctx->transferred_reply = {};
        transferred = false;
        out.native_known = true;
        out.status = ERROR_SUCCESS;
    } __except (EXCEPTION_EXECUTE_HANDLER) {
        out.exception = GetExceptionCode();
        out.status = ERROR_GEN_FAILURE;
        out.quarantine = true;
    }
    if (out.quarantine || transferred) {
        out.quarantine = true;
        out.retained_context = ctx;
        return out;
    }
    __try {
        if (!clean_string(&ctx->comment.text, &ctx->cleanup) ||
            !clean_string(&ctx->error, &ctx->cleanup)) {
            out.status = ERROR_INVALID_DATA;
            out.exception = ctx->cleanup.exception;
            out.quarantine = true;
        }
    } __except (EXCEPTION_EXECUTE_HANDLER) {
        out.status = ERROR_GEN_FAILURE;
        out.exception = GetExceptionCode();
        out.quarantine = true;
    }
    if (out.quarantine) { out.retained_context = ctx; return out; }
    if (!HeapFree(GetProcessHeap(), 0, ctx)) {
        out.status = ERROR_GEN_FAILURE;
        out.quarantine = true;
        out.retained_context = ctx;
    }
    return out;
#endif
}
} // namespace sns_interaction_call
