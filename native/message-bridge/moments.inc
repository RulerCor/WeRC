#pragma once
#include <memory>
#include <functional>
#include <new>
#include <cwchar>

// Adapted under native/helper/LICENSE for profile-1 text/image publishing.
// Current model construction and event connect/cancel were verified separately.
// Canonical PNG inputs use bounded staging and native InputTemp discovery.
#ifndef WXBOT_MOMENTS_ENABLE_CANDIDATE_SUBMIT
#define WXBOT_MOMENTS_ENABLE_CANDIDATE_SUBMIT 1
#endif
#ifndef WXBOT_MOMENTS_ENABLE_EVENT_PROBE
#define WXBOT_MOMENTS_ENABLE_EVENT_PROBE 1
#endif
#if defined(_ITERATOR_DEBUG_LEVEL) && _ITERATOR_DEBUG_LEVEL != 0
#error Moments candidate requires the same release MSVC STL ABI as Weixin.
#endif

struct MomentsConfig {
    TestConfig base;
    char self_wxid[128];
    uint32_t visibility;
    uint32_t image_count;
    uint32_t timeout_ms;
    char text[10001];
};

namespace wxbot_moments_candidate {
constexpr uintptr_t kModelCtor = 0x44776d0;
constexpr uintptr_t kModelControlTable = 0x9391028;
constexpr uintptr_t kGetSns = 0x108aee0;
constexpr uintptr_t kSubmit = 0x306ade0;
constexpr uintptr_t kConnect = 0x4a8950;
constexpr uintptr_t kDisconnect = 0x4a8850;
constexpr uintptr_t kPruneCancelled = 0x4a8cc0;
constexpr uintptr_t kMoveEventAdapter = 0x19e0c40;
constexpr uintptr_t kMethodEquals = 0x311640;
constexpr size_t kFinishedEvent = 0x2a8;

// Exact profile-1 mappings; unsupported builds are rejected by host identity checks.
static uintptr_t address(uintptr_t rva) {
    if((core_profile_id!=1&&core_profile_id!=5)||!core_base)return 0;
    switch(rva){
    case kModelCtor:return core_address(core_base,0x43a5190);
    case kModelControlTable:return core_address(core_base,0x91b7a08);
    case kGetSns:return core_address(core_base,0x1067800);
    case kSubmit:return core_address(core_base,0x2ffe3b0);
    case kConnect:return core_address(core_base,0x4945b0);
    case kDisconnect:return core_address(core_base,0x4944b0);
    case kPruneCancelled:return core_address(core_base,0x494920);
    case kMoveEventAdapter:return core_address(core_base,0x19cb900);
    case kMethodEquals:return core_address(core_base,0x2fd2b0);
    case 0x754bd18:return core_address(core_base,0x74ab8d8);
    case 0x754cd48:return core_address(core_base,0x74ac908);
    case 0x754cd70:return core_address(core_base,0x74ac930);
    default:return 0;
    }
}
static size_t model_field(size_t offset){return core_profile_id==5 && offset>=0x30 ? offset+0x10 : offset;}
static bool emoticon_account_matches(const SharedMessage& app,const char* expected){return app.object&&app.control&&same_account(expected);}

struct VectorView { void* begin; void* end; void* capacity; };
struct Slot {
    void* receiver;
    void* adapter;
    void* method;
    uintptr_t method_extra;
    uint64_t flags;
    void* method_equals;
};
static_assert(sizeof(Slot) == 48, "Unexpected native event slot ABI");
static_assert(sizeof(std::shared_ptr<int>) == sizeof(SharedMessage), "Unexpected shared_ptr ABI");

struct Snapshot {
    bool matching;
    bool account_matches;
    bool ret_known;
    int32_t ret_code;
    uint64_t sns_id;
};

struct Lease : std::enable_shared_from_this<Lease> {
    MomentsConfig config;
    size_t receipt_index=0;uint64_t expires=0;
    bool execute;
    TestResult report{};
    SRWLOCK report_lock = SRWLOCK_INIT;
    HANDLE done = nullptr;
    volatile LONG started = 0;
    volatile LONG finalized = 0;
    volatile LONG accepted = 0;
    volatile LONG submitted = 0;
    volatile LONG subscribed = 0;
    volatile LONG quarantine = 0;
    volatile LONG deadline_observed = 0;
    ULONGLONG deadline = 0;
    SharedMessage app{}, kernel{}, service{}, scheduler{}, model{};
    std::vector<std::unique_ptr<ImageStage>> images;
    SharedMessage media[9]{};
    std::unique_ptr<FileStage> video;
    wchar_t video_path[1024]{};bool video_created=false;
    wchar_t image_paths[9][1024]{};
    bool image_created[9]{};
    bool image_cleanup_ok=true;
    char client_id[256]{};
    size_t text_bytes = 0;
    explicit Lease(const MomentsConfig& value, bool send) : config(value), execute(send) {
        _InterlockedIncrement(&live_tasks);
    }
    ~Lease() {
        // 原生引用必须在原生任务上清理。quarantine 会保留全局 owner，禁止卸载。
        if (done) CloseHandle(done);
        SecureZeroMemory(&config,sizeof(config));SecureZeroMemory(client_id,sizeof(client_id));
        _InterlockedDecrement(&live_tasks);
    }
};

static SRWLOCK pending_lock = SRWLOCK_INIT;
static std::shared_ptr<Lease> pending;

static void retain_pending(const std::shared_ptr<Lease>& lease) {
    AcquireSRWLockExclusive(&pending_lock);
    pending = lease;
    ReleaseSRWLockExclusive(&pending_lock);
}
static void release_pending(Lease* expected) {
    std::shared_ptr<Lease> old;
    AcquireSRWLockExclusive(&pending_lock);
    if (pending.get() == expected) old.swap(pending);
    ReleaseSRWLockExclusive(&pending_lock);
}

static void write_result(Lease& lease, DWORD status, const char* outcome,
                         bool inspected, bool ret_known = false, int32_t ret_code = 0,
                         uint64_t sns_id = 0, bool account_ok = false) {
    const bool accepted = InterlockedCompareExchange(&lease.accepted, 0, 0) != 0;
    const bool success = !std::strcmp(outcome, "completed") && accepted && ret_known &&
        ret_code == 0 && sns_id != 0 && account_ok && !lease.report.exception &&
        !InterlockedCompareExchange(&lease.deadline_observed, 0, 0) &&
        !InterlockedCompareExchange(&lease.quarantine, 0, 0) && lease.image_cleanup_ok;
    AcquireSRWLockExclusive(&lease.report_lock);
    if ((!std::strcmp(outcome, "accepted") || !std::strcmp(outcome, "timeout_cleanup_pending")) &&
        InterlockedCompareExchange(&lease.finalized, 0, 0)) {
        ReleaseSRWLockExclusive(&lease.report_lock);
        return; // 中间状态不得覆盖已写入的终态。
    }
    AcquireSRWLockExclusive(&receipt_lock);
    auto& slot=receipts[lease.receipt_index];
    slot.native_duration=lease.report.stage;
    slot.message_id=success?sns_id:0;
    slot.evidence=success?21:(inspected&&status==0&&!lease.report.exception&&!lease.quarantine&&lease.image_cleanup_ok?20:(!std::strcmp(outcome,"accepted")?1:(lease.submitted?0:4)));
    ReleaseSRWLockExclusive(&receipt_lock);
    ReleaseSRWLockExclusive(&lease.report_lock);
}

static bool anchors_match() {
    struct Anchor { uintptr_t rva; unsigned char prefix[16]; };
    static constexpr Anchor anchors[] = {
        {kModelCtor, {0x55,0x56,0x57,0x48,0x81,0xec,0x60,0x02,0,0,0x48,0x8d,0xac,0x24,0x80,0}},
        {kGetSns, {0x55,0x56,0x57,0x48,0x81,0xec,0x90,0,0,0,0x48,0x8d,0xac,0x24,0x80,0}},
        {kSubmit, {0x55,0x56,0x57,0x48,0x81,0xec,0x80,0,0,0,0x48,0x8d,0xac,0x24,0x80,0}},
        {kConnect, {0x55,0x41,0x56,0x56,0x57,0x53,0x48,0x81,0xec,0x90,0,0,0,0x48,0x8d,0xac}},
        {kPruneCancelled, {0x41,0x56,0x56,0x57,0x53,0x48,0x83,0xec,0x28,0x48,0x89,0xd6,0x80,0xba,0xb4,0}},
    };
    __try {
        for (const auto& anchor : anchors)
            if (std::memcmp(reinterpret_cast<void*>(address(anchor.rva)), anchor.prefix, 16)) return false;
        return true;
    } __except (EXCEPTION_EXECUTE_HANDLER) { return false; }
}

static bool read_string(const SmallString* value, char* output, size_t capacity) {
    if (!value || !output || value->length == 0 || value->length >= capacity ||
        value->capacity < value->length || value->capacity > 1024 * 1024) return false;
    const char* bytes = value->capacity > 15 ? *reinterpret_cast<const char* const*>(value->data) : value->data;
    if (!bytes) return false;
    std::memcpy(output, bytes, static_cast<size_t>(value->length));
    output[value->length] = 0;
    return true;
}

#include "moments-video.inc"
#include "moments-images.inc"

static bool check_model(Lease* lease) {
    auto model = static_cast<unsigned char*>(lease->model.object);
    if (!model || !lease->model.control ||
        *static_cast<const uintptr_t*>(lease->model.control) != address(kModelControlTable) ||
        *reinterpret_cast<uint32_t*>(model + model_field(0x30)) != (lease->video ? 15u : (lease->config.image_count ? 1u : 2u)) ||
        *reinterpret_cast<uint32_t*>(model + model_field(0x248)) != lease->config.visibility) return false;
    const auto text = reinterpret_cast<SmallString*>(model + model_field(0x60));
    const char* body = text->capacity > 15 ? *reinterpret_cast<char* const*>(text->data) : text->data;
    if (text->length != lease->text_bytes || text->capacity < text->length || !body ||
        std::memcmp(body, lease->config.text, lease->text_bytes)) return false;
    const auto vector = reinterpret_cast<const VectorView*>(model + model_field(0x118));
    if (reinterpret_cast<uintptr_t>(vector->end) - reinterpret_cast<uintptr_t>(vector->begin) !=
        16ull * lease->config.image_count ||
        reinterpret_cast<uintptr_t>(vector->capacity) < reinterpret_cast<uintptr_t>(vector->end)) return false;
    if(lease->config.image_count>9 || lease->images.size()!=lease->config.image_count)return false;
    for(uint32_t i=0;i<lease->config.image_count;++i){
        const auto item=static_cast<const SharedMessage*>(vector->begin)+i;
        if(item->object!=lease->media[i].object || item->control!=lease->media[i].control)return false;
    }
    return read_string(reinterpret_cast<SmallString*>(model + model_field(0x40)), lease->client_id, sizeof(lease->client_id));
}

static DWORD prepare_native(Lease* lease) {
    __try {
        lease->report.stage = 160;
        if(closing||uint64_t(std::time(nullptr))>=lease->expires||(lease->config.visibility!=1&&lease->config.visibility!=2))return ERROR_INVALID_PARAMETER;
        using GetApp = SharedMessage* (__fastcall*)(SharedMessage*);
        using GetService = SharedMessage* (__fastcall*)(void*, SharedMessage*);
        reinterpret_cast<GetApp>(core_address(core_base, 0x47220))(&lease->app);
        if (!emoticon_account_matches(lease->app, lease->config.self_wxid)) return ERROR_NOT_READY;
        reinterpret_cast<GetService>(core_address(core_base, 0x36c740))(lease->app.object, &lease->kernel);
        if (!lease->kernel.object || !lease->kernel.control) return ERROR_NOT_READY;
        // 默认 Inspect 不触发 SNS 服务的可能惰性创建，只构造本地模型。
        if (lease->execute || WXBOT_MOMENTS_ENABLE_EVENT_PROBE) {
            reinterpret_cast<GetService>(address(kGetSns))(lease->kernel.object, &lease->service);
            if (!lease->service.object || !lease->service.control) return ERROR_NOT_READY;
        }
        using Factory=SharedMessage*(__fastcall*)(SharedMessage*,const uint32_t*);
        const uint32_t type=lease->config.image_count?8:9;
        reinterpret_cast<Factory>(core_address(core_base,0x2227b80))(&lease->model,&type);
        if(!lease->model.object||!lease->model.control)return ERROR_INVALID_DATA;
        auto model=static_cast<unsigned char*>(lease->model.object);
        auto text = reinterpret_cast<SmallString*>(model + model_field(0x60));
        if (text->length || text->capacity != 15) return ERROR_INVALID_DATA;
        // 88ad0 原生字符串 ctor，确认支持 >=4096 的对齐分配及释放元数据。
        using ConstructString = SmallString* (__fastcall*)(SmallString*, const char*);
        reinterpret_cast<ConstructString>(core_address(core_base, 0x88ad0))(text, lease->config.text);
        *reinterpret_cast<uint32_t*>(model + model_field(0x30)) = lease->video?15:(lease->config.image_count?1:2);
        *reinterpret_cast<uint32_t*>(model + model_field(0x248)) = lease->config.visibility;
        if(!lease->config.image_count)*reinterpret_cast<uint32_t*>(model + model_field(0x84)) = 2;
        const auto media_status=prepare_images(lease);if(media_status)return media_status;
        lease->report.stage = 161;
        if (!check_model(lease) || !emoticon_account_matches(lease->app, lease->config.self_wxid))
            return ERROR_INVALID_DATA;
        lease->report.stage = 162;
        return ERROR_SUCCESS;
    } __except (record_exception(GetExceptionInformation(), &lease->report)) { return ERROR_GEN_FAILURE; }
}

static bool release_native(Lease* lease) {
    // 构造/核对失败不等于清理失败。单独记录本轮清理异常，避免失败探针
    // 已成功释放所有引用，却因先前的 exception 字段永久卡住 live_tasks。
    TestResult cleanup{};
    for(auto& media:lease->media)guarded_release_shared(&media,&cleanup);
    guarded_release_shared(&lease->model, &cleanup);
    if(!InterlockedCompareExchange(&lease->submitted,0,0)){
        if(lease->video_created) {
            if(!DeleteFileW(lease->video_path) && GetLastError()!=ERROR_FILE_NOT_FOUND)lease->image_cleanup_ok=false;
            else lease->video_created=false;
        }
        for(uint32_t i=0;i<9;++i)if(lease->image_created[i]){
            if(!DeleteFileW(lease->image_paths[i]) && GetLastError()!=ERROR_FILE_NOT_FOUND)lease->image_cleanup_ok=false;
            else lease->image_created[i]=false;
        }
    }
    // Submitted inputs belong to Weixin's async uploader/cache; never delete on Unknown.
    guarded_release_shared(&lease->scheduler, &cleanup);
    guarded_release_shared(&lease->service, &cleanup);
    guarded_release_shared(&lease->kernel, &cleanup);
    guarded_release_shared(&lease->app, &cleanup);
    if (cleanup.exception) {
        if (!lease->report.exception) lease->report = cleanup;
        return false;
    }
    return true;
}

static void __fastcall event_entry(void* receiver, SharedMessage* event);

static DWORD connect_native(Lease* lease, const SharedMessage* owner) {
    __try {
        if (!emoticon_account_matches(lease->app, lease->config.self_wxid)) return ERROR_NOT_READY;
        using App = void* (__fastcall*)();
        using GetScheduler = SharedMessage* (__fastcall*)(void*, SharedMessage*);
        void* app = reinterpret_cast<App>(core_address(core_base, 0x45d10))();
        if (!app) return ERROR_NOT_READY;
        reinterpret_cast<GetScheduler>(core_address(core_base, 0x47310))(app, &lease->scheduler);
        if (!lease->scheduler.object || !lease->scheduler.control) return ERROR_NOT_READY;
        void* executor = *reinterpret_cast<void**>(static_cast<unsigned char*>(lease->scheduler.object) + 0x10);
        if (!executor) return ERROR_NOT_READY;
        Slot slot{lease, reinterpret_cast<void*>(address(kMoveEventAdapter)),
                  reinterpret_cast<void*>(&event_entry), 0, 0,
                  reinterpret_cast<void*>(address(kMethodEquals))};
        using Connect = void (__fastcall*)(void*, const Slot*, void*, uint32_t, void*, const SharedMessage*);
        lease->report.stage = 163;
        reinterpret_cast<Connect>(address(kConnect))(
            static_cast<unsigned char*>(lease->service.object) + kFinishedEvent,
            &slot, nullptr, 0, executor, owner);
        _InterlockedExchange(&lease->subscribed, 1);
        return ERROR_SUCCESS;
    } __except (record_exception(GetExceptionInformation(), &lease->report)) {
        // Connect 可能已经把回调交给原生系统，不能假定失败即未注册。
        _InterlockedExchange(&lease->quarantine, 1);
        return ERROR_GEN_FAILURE;
    }
}

struct NativeEventGuard {
    void* mutex;
    bool owns;
};
static_assert(sizeof(NativeEventGuard) == 16, "Unexpected native event guard ABI");
static_assert(offsetof(NativeEventGuard, owns) == 8, "Unexpected native event guard state");

static bool drain_cancelled_connections(Lease* lease) {
    // Disconnect only marks nodes. Without another SNS event, their strong owner
    // references remain alive indefinitely. Use the same cleanup path as Emit's
    // scope guard, under its separate recursive emission mutex (+50).
    NativeEventGuard guard{};
    bool success = false;
    __try {
        __try {
            auto source = static_cast<unsigned char*>(lease->service.object) + kFinishedEvent;
            auto backend = *reinterpret_cast<unsigned char**>(source + 8);
            if (!backend) __leave;
            guard.mutex = backend + 0x50;
            using Lock = int (__fastcall*)(void*);
            using Prune = void (__fastcall*)(NativeEventGuard*, void*);
            if (reinterpret_cast<Lock>(address(0x754cd48))(guard.mutex) != 0) __leave;
            guard.owns = true;
            if (*reinterpret_cast<const unsigned char*>(backend + 0xb4)) __leave;
            // A callback may be running inside Emit on this same thread. In
            // that case Emit's own guard prunes after all callbacks return.
            if (*reinterpret_cast<const uint32_t*>(backend + 0xb0) == 0)
                reinterpret_cast<Prune>(address(kPruneCancelled))(&guard, backend);
            success = true;
        } __finally {
            if (guard.owns) {
                using Unlock = int (__fastcall*)(void*);
                reinterpret_cast<Unlock>(address(0x754cd70))(guard.mutex);
                guard.owns = false;
            }
        }
    } __except (record_exception(GetExceptionInformation(), &lease->report)) {
        success = false;
    }
    return success;
}

static bool disconnect_native(Lease* lease) {
    if (!InterlockedCompareExchange(&lease->subscribed, 0, 0)) return true;
    __try {
        if (!lease->service.object) return false;
        using Disconnect = void (__fastcall*)(void*, void*);
        reinterpret_cast<Disconnect>(address(kDisconnect))(
            static_cast<unsigned char*>(lease->service.object) + kFinishedEvent, lease);
        _InterlockedExchange(&lease->subscribed, 0);
        return drain_cancelled_connections(lease);
    } __except (record_exception(GetExceptionInformation(), &lease->report)) { return false; }
}

static DWORD submit_native(Lease* lease) {
    __try {
        if (InterlockedCompareExchange(&lease->quarantine, 0, 0) ||
            !check_model(lease) || !emoticon_account_matches(lease->app, lease->config.self_wxid))
            return ERROR_NOT_READY;
        if(closing||uint64_t(std::time(nullptr))>=lease->expires||(lease->config.visibility!=1&&lease->config.visibility!=2))return ERROR_NOT_READY;
        using Submit = bool (__fastcall*)(void*, SharedMessage*);
        lease->report.stage = 164;
        // Current profile-1 native caller transfers one additional shared reference.
        // 传递一份额外强引用；本地 lease.model 的引用留待完成/超时清理。
        SharedMessage transferred = lease->model;
        _InterlockedIncrement(reinterpret_cast<volatile LONG*>(
            static_cast<unsigned char*>(transferred.control) + 8));
        _InterlockedExchange(&lease->submitted, 1); // 此后失败/超时也不自动重试。
        const bool accepted = reinterpret_cast<Submit>(address(kSubmit))(lease->service.object, &transferred);
        if (accepted) _InterlockedExchange(&lease->accepted, 1);
        if (!emoticon_account_matches(lease->app, lease->config.self_wxid)) return ERROR_NOT_READY;
        return accepted ? ERROR_SUCCESS : ERROR_GEN_FAILURE;
    } __except (record_exception(GetExceptionInformation(), &lease->report)) { return ERROR_GEN_FAILURE; }
}

static DWORD snapshot_native(Lease* lease, const SharedMessage* event, Snapshot* snapshot) {
    __try {
        if (!event || !event->object || !event->control) return ERROR_INVALID_DATA;
        const auto item = static_cast<const unsigned char*>(event->object);
        char client_id[256]{};
        if (!read_string(reinterpret_cast<const SmallString*>(item + 8), client_id, sizeof(client_id)))
            return ERROR_INVALID_DATA;
        if (std::strcmp(client_id, lease->client_id)) return ERROR_SUCCESS;
        snapshot->matching = true;
        snapshot->account_matches = emoticon_account_matches(lease->app, lease->config.self_wxid);
        snapshot->ret_code = *reinterpret_cast<const int32_t*>(item);
        snapshot->ret_known = true;
        const auto model = reinterpret_cast<const SharedMessage*>(item + 0x28);
        if (snapshot->ret_code == 0 && model->object && model->control) {
            // 同时核对 event 回传模型的 client_id，不能只信任外层事件标签。
            char returned_id[256]{};
            const auto data = static_cast<const unsigned char*>(model->object);
            if (!read_string(reinterpret_cast<const SmallString*>(data + model_field(0x40)), returned_id,
                             sizeof(returned_id)) || std::strcmp(returned_id, lease->client_id))
                return ERROR_INVALID_DATA;
            const auto sns = reinterpret_cast<const SharedMessage*>(data + model_field(0x280));
            if (sns->object && sns->control)
                snapshot->sns_id = *static_cast<const uint64_t*>(sns->object);
        }
        return ERROR_SUCCESS;
    } __except (record_exception(GetExceptionInformation(), &lease->report)) { return ERROR_INVALID_DATA; }
}

// 只从已调度原生任务或带同一 executor 的原生完成回调调用。
static void finish_on_task(const std::shared_ptr<Lease>& lease, DWORD status, const char* outcome,
                           bool inspected, const Snapshot* snapshot = nullptr) {
    if (_InterlockedCompareExchange(&lease->finalized, 1, 0) != 0) return;
    if (lease->done) SetEvent(lease->done);
    const bool account_ok = snapshot && snapshot->account_matches;
    const bool ret_known = snapshot && snapshot->ret_known;
    const int32_t ret_code = ret_known ? snapshot->ret_code : 0;
    const uint64_t sns_id = snapshot ? snapshot->sns_id : 0;
    bool safe = !InterlockedCompareExchange(&lease->quarantine, 0, 0) && disconnect_native(lease.get());
    if (safe) safe = release_native(lease.get());
    if (!safe) {
        _InterlockedExchange(&lease->quarantine, 1);
        status = ERROR_GEN_FAILURE;
        outcome = "cleanup_unconfirmed";
        // 保留 pending 和 live_tasks；宁可拒绝卸载，不释放可能仍被原生回调使用的代码。
    }
    lease->report.stage = safe ? 166 : 169;
    write_result(*lease, status, outcome, inspected, ret_known, ret_code, sns_id, account_ok);
    if (safe) release_pending(lease.get());
}

static void __fastcall event_entry(void* receiver, SharedMessage* event) {
    _InterlockedIncrement(&active_callbacks);
    try {
        auto raw = static_cast<Lease*>(receiver);
        auto lease = raw->shared_from_this(); // 原生事件 node 已持有 ownerShared 的强引用。
        if (!InterlockedCompareExchange(&lease->finalized, 0, 0)) {
            Snapshot snapshot{};
            const DWORD status = snapshot_native(lease.get(), event, &snapshot);
            if (snapshot.matching) {
                _InterlockedExchange(&lease->accepted, 1);
                if (GetTickCount64() >= lease->deadline) {
                    _InterlockedExchange(&lease->deadline_observed, 1);
                    finish_on_task(lease, WAIT_TIMEOUT, "timeout", false, &snapshot);
                } else if (status != ERROR_SUCCESS) {
                    finish_on_task(lease, status, "invalid_completion", false, &snapshot);
                } else if (!snapshot.account_matches) {
                    finish_on_task(lease, ERROR_NOT_READY, "account_changed", false, &snapshot);
                } else if (snapshot.ret_code != 0) {
                    finish_on_task(lease, ERROR_GEN_FAILURE, "failed", false, &snapshot);
                } else if (!snapshot.sns_id) {
                    finish_on_task(lease, ERROR_INVALID_DATA, "missing_sns_id", false, &snapshot);
                } else {
                    finish_on_task(lease, ERROR_SUCCESS, "completed", false, &snapshot);
                }
            }
        }
        // kMoveEventAdapter 转移了这份引用；与原生 e949f0 一样由被调用方释放。
        if (event) guarded_release_shared(event, &lease->report);
    } catch (...) {
        TestResult error{};
        if (event) guarded_release_shared(event, &error);
        // 不让 C++ 异常跨入微信；event owner/pending 继续阻止卸载，timeout 接管。
    }
    _InterlockedDecrement(&active_callbacks);
}

static DWORD WINAPI deadline_thread(void* parameter) {
    _InterlockedIncrement(&active_callbacks);
    auto holder = static_cast<std::shared_ptr<Lease>*>(parameter);
    auto lease = *holder;
    delete holder;
    try {
        const DWORD wait = WaitForSingleObject(lease->done, lease->config.timeout_ms);
        if (wait != WAIT_OBJECT_0 && !InterlockedCompareExchange(&lease->finalized, 0, 0)) {
            _InterlockedExchange(&lease->deadline_observed, 1);
            std::function<void()> task = [lease]() {
                _InterlockedIncrement(&active_callbacks);
                finish_on_task(lease, WAIT_TIMEOUT, "timeout", false);
                _InterlockedDecrement(&active_callbacks);
            };
            const DWORD status = schedule_task(lease->config.base, &task);
            if (status && !InterlockedCompareExchange(&lease->finalized, 0, 0)) {
                // 无法在原生线程取消时，绝不从 Windows worker 直接操作事件源。
                _InterlockedExchange(&lease->quarantine, 1);
                write_result(*lease, status, "timeout_cleanup_pending", false);
                // pending 继续保留租约；禁止卸载和自动重发。
            }
        }
    } catch (...) {
        _InterlockedExchange(&lease->quarantine, 1);
        write_result(*lease, ERROR_GEN_FAILURE, "timeout_cleanup_pending", false);
    }
    lease.reset();
    _InterlockedDecrement(&active_callbacks);
    return 0;
}

static bool start_deadline(const std::shared_ptr<Lease>& lease) {
    auto holder = new (std::nothrow) std::shared_ptr<Lease>(lease);
    if (!holder) return false;
    HANDLE thread = CreateThread(nullptr, 0, deadline_thread, holder, 0, nullptr);
    if (!thread) { delete holder; return false; }
    CloseHandle(thread);
    return true;
}

static void on_task(const std::shared_ptr<Lease>& lease) {
    _InterlockedIncrement(&active_callbacks);
    // 调度器失败可能发生在复制 callable 后；-1 表示调用方已撤销尚未开始的任务。
    if (_InterlockedCompareExchange(&lease->started, 1, 0) != 0) {
        _InterlockedDecrement(&active_callbacks);
        return;
    }
    lease->text_bytes = std::strlen(lease->config.text);
    const DWORD prepared = prepare_native(lease.get());
    if (prepared || !lease->execute) {
        Snapshot snapshot{};
        snapshot.account_matches = prepared == ERROR_SUCCESS;
        DWORD inspected = prepared;
        if (!prepared && !lease->execute && WXBOT_MOMENTS_ENABLE_EVENT_PROBE) {
            SharedMessage owner{};
            std::memcpy(&owner, &lease, sizeof(owner));
            // 可选研究探针：只注册后立即取消；绝不调用 Submit 或伪造服务端完成事件。
            inspected = connect_native(lease.get(), &owner);
        }
        finish_on_task(lease, inspected, inspected ? "prepare_failed" : "inspect_ready",
                       inspected == ERROR_SUCCESS, &snapshot);
        _InterlockedDecrement(&active_callbacks);
        return;
    }
    // 必须先订阅完成事件，再提交；收到非零 SNS ID 才确认发布。
    lease->done = CreateEventW(nullptr, TRUE, FALSE, nullptr);
    if (!lease->done) {
        finish_on_task(lease, GetLastError(), "prepare_failed", false);
        _InterlockedDecrement(&active_callbacks);
        return;
    }
    lease->deadline = GetTickCount64() + lease->config.timeout_ms;
    SharedMessage owner{};
    std::memcpy(&owner, &lease, sizeof(owner)); // 仅输入视图；Connect 自己原子 retain。
    const DWORD connected = connect_native(lease.get(), &owner);
    if (connected) {
        finish_on_task(lease, connected, "subscribe_failed", false);
    } else if (!start_deadline(lease)) {
        finish_on_task(lease, ERROR_NOT_ENOUGH_MEMORY, "deadline_unavailable", false);
    } else {
        const DWORD submitted = submit_native(lease.get());
        if (submitted) {
            finish_on_task(lease, submitted,
                submitted == ERROR_NOT_READY ? "account_changed" : "submit_unconfirmed", false);
        } else if (!InterlockedCompareExchange(&lease->finalized, 0, 0)) {
            lease->report.stage = 165;
            write_result(*lease, ERROR_SUCCESS, "accepted", false, false, 0, 0, true);
        }
    }
    _InterlockedDecrement(&active_callbacks);
}

} // namespace wxbot_moments_candidate

