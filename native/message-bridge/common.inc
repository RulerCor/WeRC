// Derived from helper/native/selftest_bridge.cpp; see ../helper/LICENSE.
// WeRC modification: internal helpers only, no disk reports, authenticated
// conversation receive with native IDs. The public API is in bridge.cpp.
#define WIN32_LEAN_AND_MEAN
#include <windows.h>
#include <bcrypt.h>
#include <cstdint>
#include <cstring>
#include <algorithm>
#include <cstdio>
#include <functional>
#include <memory>
#include "../helper/native/vendor/minhook-1.3.4/include/MinHook.h"
#include "core_version.h"

static int werc_no_file_report(FILE** out, const wchar_t*, const wchar_t*) {
    *out = nullptr;
    return EACCES;
}
#define _wfopen_s werc_no_file_report

struct SharedMessage { void* object; void* control; };
struct SmallString { char data[16]; uint64_t length; uint64_t capacity; };
struct TestConfig {
    uint32_t size;
    uint32_t pid;
    uint64_t manager;
    wchar_t log_path[MAX_PATH];
    char text[1024];
};
struct TestResult {
    DWORD stage;
    DWORD exception;
    uint64_t vtable_rva;
    uint32_t native_status;
    uint64_t exception_pc;
    uint64_t fault_address;
    uint64_t access_kind;
    uint32_t native_status_high;
};

using SendFunction = void* (__fastcall*)(void*, void*, SharedMessage*, uint8_t);
static SendFunction original_send = nullptr;
static uintptr_t core_base = 0;
static TestConfig test_config{};
static volatile LONG active_callbacks = 0;
static volatile LONG triggered_once = 0;
static volatile LONG live_completions = 0;
static volatile LONG live_tasks = 0;
static bool hook_created = false;
static bool factory_hook_created = false;
#ifdef WXBOT_QUOTE_SEND
static bool quote_manager_hook_created = false;
#endif
#ifdef WXBOT_LOCAL_RECALL
static bool local_recall_hook_created = false;
static volatile LONG local_recall_enabled = 0;
#endif
static bool image_download_factory_hook_created = false;
static DWORD factory_tls = TLS_OUT_OF_INDEXES;
using ReceiveFunction = uintptr_t (__fastcall*)(void*, const uintptr_t*, uint8_t, uint8_t);
static ReceiveFunction original_receive = nullptr;
static bool receive_hook_created = false;
#ifdef WXBOT_RECALL_RECEIVE
static bool recall_receive_hook_created = false;
#endif
#ifdef WXBOT_FRIEND_RECEIVE
static bool friend_receive_hook_created = false;
#endif
static TestConfig receive_config{};
static bool receive_queue_mode = false;

// Only immutable login bytes are used inside the hook; no client calls or disk IO.
// Set before hook installation; cleared only after all callbacks stop.
static char receive_login[128]{};
static bool route_id(const char* value) {
    const size_t n=strnlen_s(value,128);
    if(!n || n==128)return false;
    for(size_t i=0;i<n;++i) {
        const unsigned char c=value[i];
        if(!((c>='a'&&c<='z')||(c>='A'&&c<='Z')||(c>='0'&&c<='9')||c=='_'||c=='-'||c=='.'||c=='@'))return false;
    }
    return true;
}
static const char* receive_conversation(const char* login,const char* sender,const char* receiver) {
    if(!route_id(login)||!route_id(sender)||!route_id(receiver))return nullptr;
    if(!std::strcmp(sender,login))return receiver;
    if(!std::strcmp(receiver,login))return sender;
    return nullptr;
}
struct ReceivedMessage {
    uint32_t size, present, type, thread_id, dropped, reserved;
    uint64_t sequence;
    char sender[128], receiver[128], text[16384];
#ifdef WERC_ONLY_RECEIVE
    uint32_t voice_length;
    unsigned char voice_bytes[262144];
#endif
#ifdef WXBOT_EXTENDED_RECEIVE
    char mention_source[4096];
#endif
#ifdef WXBOT_MEDIA_RECEIVE
#ifdef WXBOT_LARGE_MEDIA_RECEIVE
    char image_xml[262144];
#else
    char image_xml[16384];
#endif
#endif
#ifdef WXBOT_MESSAGE_ID_RECEIVE
    uint64_t native_message_id;
#endif
#ifdef WXBOT_VOICE_RECEIVE
    uint32_t voice_offset, voice_size; // Binary slice within image_xml, after its XML NUL.
#endif
};
#if defined(WERC_ONLY_RECEIVE)
static_assert(sizeof(ReceivedMessage) == 278832, "Unexpected WeRC receive ABI");
#elif defined(WXBOT_VOICE_RECEIVE)
static_assert(sizeof(ReceivedMessage) == 267568, "Unexpected media receive V6 ABI");
#elif defined(WXBOT_MESSAGE_ID_RECEIVE)
static_assert(sizeof(ReceivedMessage) == 267560, "Unexpected media receive V5 ABI");
#elif defined(WXBOT_LARGE_MEDIA_RECEIVE)
static_assert(sizeof(ReceivedMessage) == 267552, "Unexpected media receive V4 ABI");
#elif defined(WXBOT_MEDIA_RECEIVE)
static_assert(sizeof(ReceivedMessage) == 21792, "Unexpected media receive V3 ABI");
#elif defined(WXBOT_EXTENDED_RECEIVE)
static_assert(sizeof(ReceivedMessage) == 5408, "Unexpected text receive V2 ABI");
#else
static_assert(sizeof(ReceivedMessage) == 1312, "Unexpected receive ABI");
#endif
static SRWLOCK receive_lock = SRWLOCK_INIT;
static ReceivedMessage received_messages[64]{};
static uint32_t receive_head = 0, receive_count = 0, receive_dropped = 0;
static uint64_t receive_sequence = 0;
static ReceivedMessage receive_inflight{};
static uint64_t receive_acked = 0;
static void receive_gap() {
    AcquireSRWLockExclusive(&receive_lock);
    if(receive_dropped!=UINT32_MAX) ++receive_dropped;
    ReleaseSRWLockExclusive(&receive_lock);
}

static bool copy_voice_bytes(void* destination,const void* source,size_t bytes) {
    __try {std::memcpy(destination,source,bytes);return true;}
    __except(EXCEPTION_EXECUTE_HANDLER){SecureZeroMemory(destination,bytes);return false;}
}
static void enqueue_received(const char* sender, const char* receiver, const char* text,
                             const char* source = nullptr, uint32_t type = 1, const char* image_xml = nullptr,
                             uint32_t reserved = 0, uint64_t native_message_id = 0,
                             const void* voice = nullptr, uint32_t voice_size = 0) {
    // 大型转发内容留在堆上，避免消耗微信接收线程的栈。
    auto storage = std::unique_ptr<ReceivedMessage>(new (std::nothrow) ReceivedMessage{});
    if (!storage) {receive_gap();return;}
    auto& message = *storage;
    message.size = sizeof(message);
    message.present = 1;
    message.type = type;
    message.reserved = reserved;
    message.thread_id = GetCurrentThreadId();
#ifdef WXBOT_MESSAGE_ID_RECEIVE
    message.native_message_id = native_message_id;
#else
    (void)native_message_id;
#endif
    if (strnlen_s(sender,sizeof(message.sender)) == sizeof(message.sender) ||
        strnlen_s(receiver,sizeof(message.receiver)) == sizeof(message.receiver) ||
        strnlen_s(text,sizeof(message.text)) == sizeof(message.text)) { receive_gap(); return; }
    std::memcpy(message.sender, sender, std::strlen(sender) + 1);
    std::memcpy(message.receiver, receiver, std::strlen(receiver) + 1);
    std::memcpy(message.text, text, std::strlen(text) + 1);
#ifdef WXBOT_EXTENDED_RECEIVE
    if (source) std::memcpy(message.mention_source, source, std::strlen(source) + 1);
#else
    (void)source;
#endif
#ifdef WXBOT_MEDIA_RECEIVE
    if (image_xml) strcpy_s(message.image_xml, image_xml);
#else
    (void)image_xml;
#endif
#ifdef WERC_ONLY_RECEIVE
    if(type==34 && voice && voice_size && voice_size<=sizeof(message.voice_bytes)) {
        if(copy_voice_bytes(message.voice_bytes,voice,voice_size))message.voice_length=voice_size;
    }
#elif defined(WXBOT_VOICE_RECEIVE)
    if (type == 34 && voice && voice_size) {
        const size_t offset = std::strlen(message.image_xml) + 1;
        if (offset < sizeof(message.image_xml) && voice_size <= sizeof(message.image_xml) - offset) {
            std::memcpy(message.image_xml + offset, voice, voice_size);
            message.voice_offset = static_cast<uint32_t>(offset);
            message.voice_size = voice_size;
        }
    }
#else
    (void)voice;
    (void)voice_size;
#endif
    AcquireSRWLockExclusive(&receive_lock);
    if (receive_count == std::size(received_messages)) {
        receive_head = (receive_head + 1) % 64;
        --receive_count;
        if(receive_dropped!=UINT32_MAX) ++receive_dropped;
    }
    message.sequence = ++receive_sequence;
    message.dropped = receive_dropped;
    received_messages[(receive_head + receive_count) % 64] = message;
    ++receive_count;
    ReleaseSRWLockExclusive(&receive_lock);
    SecureZeroMemory(&message,sizeof(message));
}

static int record_exception(EXCEPTION_POINTERS* exception, TestResult* result) {
    result->exception = exception->ExceptionRecord->ExceptionCode;
    result->exception_pc = reinterpret_cast<uintptr_t>(exception->ExceptionRecord->ExceptionAddress);
    if (exception->ExceptionRecord->NumberParameters >= 2) {
        result->access_kind = exception->ExceptionRecord->ExceptionInformation[0];
        result->fault_address = exception->ExceptionRecord->ExceptionInformation[1];
    }
    return EXCEPTION_EXECUTE_HANDLER;
}

static bool core_image_identity(HMODULE core, uint32_t* timestamp, uint32_t* image_size) {
    __try {
        const auto bytes = reinterpret_cast<const unsigned char*>(core);
        const auto dos = reinterpret_cast<const IMAGE_DOS_HEADER*>(bytes);
        if (dos->e_magic != IMAGE_DOS_SIGNATURE || dos->e_lfanew < 64 || dos->e_lfanew > 1048576) return false;
        const auto nt = reinterpret_cast<const IMAGE_NT_HEADERS64*>(bytes + dos->e_lfanew);
        if (nt->Signature != IMAGE_NT_SIGNATURE || nt->FileHeader.Machine != IMAGE_FILE_MACHINE_AMD64 ||
                nt->OptionalHeader.Magic != IMAGE_NT_OPTIONAL_HDR64_MAGIC) return false;
        *timestamp = nt->FileHeader.TimeDateStamp;
        *image_size = nt->OptionalHeader.SizeOfImage;
        return true;
    } __except (EXCEPTION_EXECUTE_HANDLER) { return false; }
}

static bool core_fingerprint(HMODULE core) {
    uint32_t timestamp = 0, image_size = 0;
    if (!core_image_identity(core, &timestamp, &image_size)) return false;
    wchar_t path[32768]{};
    if (!GetModuleFileNameW(core, path, static_cast<DWORD>(std::size(path)))) return false;
    HANDLE file = CreateFileW(path, GENERIC_READ, FILE_SHARE_READ | FILE_SHARE_DELETE,
                              nullptr, OPEN_EXISTING, FILE_FLAG_SEQUENTIAL_SCAN, nullptr);
    if (file == INVALID_HANDLE_VALUE) return false;
    LARGE_INTEGER size{};
    bool valid = GetFileSizeEx(file, &size) && core_size_supported(static_cast<uint64_t>(size.QuadPart));
    BCRYPT_ALG_HANDLE algorithm = nullptr;
    BCRYPT_HASH_HANDLE hash = nullptr;
    if (valid && BCryptOpenAlgorithmProvider(&algorithm, BCRYPT_SHA256_ALGORITHM, nullptr, 0) >= 0) {
        // Windows allocates the hash object when pbHashObject is NULL on supported Windows.
        if (BCryptCreateHash(algorithm, &hash, nullptr, 0, nullptr, 0, 0) >= 0) {
            unsigned char buffer[65536], digest[32];
            DWORD count = 0;
            while (valid) {
                valid = ReadFile(file, buffer, sizeof(buffer), &count, nullptr) != FALSE;
                if (!valid || !count) break;
                valid = BCryptHashData(hash, buffer, count, 0) >= 0;
            }
            valid = valid && BCryptFinishHash(hash, digest, sizeof(digest), 0) >= 0 &&
                core_select_build(static_cast<uint64_t>(size.QuadPart), digest, timestamp, image_size);
            BCryptDestroyHash(hash);
        } else valid = false;
        BCryptCloseAlgorithmProvider(algorithm, 0);
    } else valid = false;
    CloseHandle(file);
    return valid;
}

static DWORD WINAPI WxInspectCoreVersion(void*) {
    const auto core = GetModuleHandleW(L"Weixin.dll");
    if (!core || !core_fingerprint(core)) return 0;
    return static_cast<DWORD>(InterlockedCompareExchange(&core_profile_id, 0, 0));
}

static void release_shared(SharedMessage& message) {
    if (!message.control) return;
    auto control = static_cast<unsigned char*>(message.control);
    if (_InterlockedDecrement(reinterpret_cast<volatile LONG*>(control + 8)) == 0) {
        using VirtualFunction = void(__fastcall*)(void*);
        auto table = *reinterpret_cast<VirtualFunction**>(control);
        table[0](control);
        if (_InterlockedDecrement(reinterpret_cast<volatile LONG*>(control + 12)) == 0) {
            table = *reinterpret_cast<VirtualFunction**>(control);
            table[1](control);
        }
    }
    message = {};
}

static void guarded_release_shared(SharedMessage* message, TestResult* report) {
    __try {
        release_shared(*message);
    } __except (record_exception(GetExceptionInformation(), report)) {
    }
}

static bool set_inline(SmallString* string, const char* text) {
    const size_t length = std::strlen(text);
    if (length > 15 || string->capacity != 15 || string->length != 0) return false;
    std::memcpy(string->data, text, length + 1);
    string->length = length;
    return true;
}

static bool set_native_string(SmallString* string, const char* text) {
    const size_t length = std::strlen(text);
    if (length < 16) return set_inline(string, text);
    if (length > 1023 || string->capacity != 15 || string->length != 0) return false;
    using Allocate = void* (__fastcall*)(size_t);
    char* buffer = static_cast<char*>(reinterpret_cast<Allocate>(core_address(core_base, 0x74ab89c))(length + 1));
    if (!buffer) return false;
    std::memcpy(buffer, text, length + 1);
    std::memcpy(string->data, &buffer, sizeof(buffer));
    string->length = string->capacity = length;
    return true;
}

// Long bodies use the client's own aligned string allocator (exact profile 1).
static bool set_native_body(SmallString* string,const char* text) {
    const auto length=strnlen_s(text,10001);
    if(length<=1023)return set_native_string(string,text);
    if(length>10000||(core_profile_id!=1&&core_profile_id!=5&&core_profile_id!=11&&core_profile_id!=12&&core_profile_id!=13)||string->length||string->capacity!=15)return false;
    using Construct=SmallString*(__fastcall*)(SmallString*,const char*);
    reinterpret_cast<Construct>(core_address(core_base,0x88ad0))(string,text);
    const auto bytes=string->capacity<=15?string->data:*reinterpret_cast<const char*const*>(string->data);
    return string->length==length&&string->capacity>=length&&string->capacity<=16383&&bytes&&!std::memcmp(bytes,text,length+1);
}
static void free_native_string(SmallString* string) {
    if (string->capacity > 15) {
        void* allocation = *reinterpret_cast<void**>(string->data);
        size_t bytes=string->capacity+1;
        if(bytes>=4096){allocation=*(reinterpret_cast<void**>(allocation)-1);bytes+=39;}
        using Free = void (__fastcall*)(void*, size_t);
        reinterpret_cast<Free>(core_address(core_base, 0x74ab8e0))(allocation, bytes);
    }
    *string = {};
    string->capacity = 15;
}

static void write_json_string(FILE* log, const char* text) {
    std::fputc('"', log);
    for (const unsigned char* current = reinterpret_cast<const unsigned char*>(text); *current; ++current) {
        if (*current == '"' || *current == '\\') { std::fputc('\\', log); std::fputc(*current, log); }
        else if (*current < 32) std::fprintf(log, "\\u%04x", *current);
        else std::fputc(*current, log);
    }
    std::fputc('"', log);
}

static void native_test(uintptr_t base, void* manager, TestResult* report) {
    // Keep this SEH boundary free of C++ objects that require unwinding.
    SharedMessage message{};
    alignas(16) unsigned char result[128]{};
    __try {
        report->stage = 1;
        using Factory = SharedMessage* (__fastcall*)(SharedMessage*);
        reinterpret_cast<Factory>(core_address(base, 0x6f4c70))(&message);
        if (!message.object || !message.control) { report->stage = 11; return; }
        auto object = static_cast<unsigned char*>(message.object);
        report->vtable_rva = *reinterpret_cast<uintptr_t*>(object) - base;
        if (report->vtable_rva != core_rva(0x8e53588)) { report->stage = 12; release_shared(message); return; }
        if (!set_inline(reinterpret_cast<SmallString*>(object + 0xb0), "filehelper") ||
            !set_inline(reinterpret_cast<SmallString*>(object + 0x758), "WXNATIVE_TEST_2")) {
            report->stage = 13; release_shared(message); return;
        }
        *reinterpret_cast<uint32_t*>(object + 0x118) = 1;
        // The client text builder writes only type, receiver and body. Keep all
        // other fields at the constructor's defaults rather than borrowing
        // unverified offsets from an older version.
        report->stage = 2;
        auto send = original_send ? original_send : reinterpret_cast<SendFunction>(core_address(base, 0x178f100));
        // The native function consumes one strong reference to its argument.
        // Retain the factory's reference separately from that transferred copy.
        SharedMessage transferred = message;
        _InterlockedIncrement(reinterpret_cast<volatile LONG*>(
            static_cast<unsigned char*>(message.control) + 8));
        send(manager, result, &transferred, 1);
        report->stage = 3;
        report->native_status = *reinterpret_cast<uint32_t*>(result);
        report->native_status_high = *reinterpret_cast<uint32_t*>(result + 4);
        release_shared(message);
        auto result_message = reinterpret_cast<SharedMessage*>(result + 40);
        release_shared(*result_message);
        // This result contains a native std::string after its eight-byte status field.
        auto error_text = reinterpret_cast<SmallString*>(result + 8);
        if (error_text->capacity > 15 && error_text->capacity < 1024 * 1024) {
            void* allocation = *reinterpret_cast<void**>(error_text->data);
            size_t allocation_size = static_cast<size_t>(error_text->capacity + 1);
            if (allocation_size >= 4096) {
                allocation = *(reinterpret_cast<void**>(allocation) - 1);
                allocation_size += 39;
            }
            using Free = void(__fastcall*)(void*, size_t);
            reinterpret_cast<Free>(core_address(base, 0x74ab8e0))(allocation, allocation_size);
        }
        report->stage = 4;
    } __except (record_exception(GetExceptionInformation(), report)) {
    }
}

static void write_report(const TestConfig& config, const TestResult& result,
                         const char* marker = "WXNATIVE_TEST_2") {
    FILE* log = nullptr;
    if (_wfopen_s(&log, config.log_path, L"wb") == 0 && log) {
        std::fprintf(log, "{\"stage\":%lu,\"exception\":%lu,\"vtable_rva\":\"0x%llx\","
                          "\"native_status\":%u,\"exception_pc\":\"0x%llx\","
                          "\"exception_rva\":\"0x%llx\",\"fault_address\":\"0x%llx\","
                          "\"access_kind\":%llu,\"native_status_high\":%u,\"target\":\"filehelper\","
                          "\"text\":\"%s\"}\n",
                     result.stage, result.exception, result.vtable_rva, result.native_status,
                     result.exception_pc, result.exception_pc ? result.exception_pc - core_base : 0,
                     result.fault_address, result.access_kind, result.native_status_high, marker);
        std::fclose(log);
    }
}

static bool is_trigger(SharedMessage* message) {
    __try {
        if (!message || !message->object) return false;
        auto object = static_cast<unsigned char*>(message->object);
        const auto receiver = reinterpret_cast<const SmallString*>(object + 0xb0);
        const auto text = reinterpret_cast<const SmallString*>(object + 0x758);
        if (receiver->length != 10 || receiver->capacity != 15 ||
            std::memcmp(receiver->data, "filehelper", 10) != 0 || text->length != 16) return false;
        const char* buffer = text->capacity < 16 ? text->data : *reinterpret_cast<const char* const*>(text->data);
        return std::memcmp(buffer, "WXNATIVE_TRIGGER", 16) == 0;
    } __except (EXCEPTION_EXECUTE_HANDLER) { return false; }
}

static void* __fastcall hooked_send(void* manager, void* output, SharedMessage* message, uint8_t flag) {
    _InterlockedIncrement(&active_callbacks);
    const bool trigger = is_trigger(message);
    void* result = nullptr;
    __try {
        result = original_send(manager, output, message, flag);
        if (trigger && _InterlockedCompareExchange(&triggered_once, 1, 0) == 0) {
            TestResult report{};
            native_test(core_base, manager, &report);
            write_report(test_config, report);
        }
    } __finally { _InterlockedDecrement(&active_callbacks); }
    return result;
}

static DWORD WINAPI WxArmSelfTest(void* parameter) {
    if (!parameter) return 2;
    const TestConfig config = *static_cast<const TestConfig*>(parameter);
    if (config.size != sizeof(TestConfig) || config.pid != GetCurrentProcessId()) return 3;
    HMODULE core = GetModuleHandleW(L"Weixin.dll");
    if (!core || !core_fingerprint(core)) return 4;
    core_base = reinterpret_cast<uintptr_t>(core);
    test_config = config;
    triggered_once = 0;
    MH_STATUS status = MH_Initialize();
    if (status != MH_OK) return 600 + status;
    status = MH_CreateHook(reinterpret_cast<void*>(core_address(core_base, 0x178f100)),
                           reinterpret_cast<void*>(&hooked_send), reinterpret_cast<void**>(&original_send));
    if (status != MH_OK) { MH_Uninitialize(); return 600 + status; }
    status = MH_EnableHook(reinterpret_cast<void*>(core_address(core_base, 0x178f100)));
    if (status != MH_OK) { MH_RemoveHook(reinterpret_cast<void*>(core_address(core_base, 0x178f100))); MH_Uninitialize(); }
    hook_created = status == MH_OK;
    return status == MH_OK ? 0 : 600 + status;
}

// The client's text builder accepts this intrusive completion object. Its ABI
// has a reference count at +0, an invoke pointer at +8, and a destroy pointer
// at +16, as confirmed in 0x5429c30 and 0x107050.
struct Completion {
    volatile LONG references;
    uint32_t padding;
    void (__fastcall* invoke)(Completion*, const unsigned char*);
    void (__fastcall* destroy)(Completion*);
    TestConfig config;
};

static void __fastcall completion_invoke(Completion* self, const unsigned char* result) {
    _InterlockedIncrement(&active_callbacks);
    __try {
        TestResult report{};
        report.stage = 21;
        report.native_status = result && result[0] == 1 ? 0 : 1;
        FILE* log = nullptr;
        if (_wfopen_s(&log, self->config.log_path, L"wb") == 0 && log) {
            std::fprintf(log, "{\"stage\":21,\"exception\":0,\"native_status\":%u,"
                              "\"target\":\"filehelper\",\"text\":", report.native_status);
            write_json_string(log, self->config.text);
            std::fprintf(log, ",\"thread_id\":%lu,\"completion_success\":%s}\n", GetCurrentThreadId(),
                         report.native_status == 0 ? "true" : "false");
            std::fclose(log);
        }
    } __finally { _InterlockedDecrement(&active_callbacks); }
}

static void __fastcall completion_destroy(Completion* self) {
    delete self;
    _InterlockedDecrement(&live_completions);
}

static DWORD dispatch_on_native_task(void* parameter) {
    if (!parameter || live_completions != 0) return ERROR_BUSY;
    const TestConfig config = *static_cast<const TestConfig*>(parameter);
    if (config.size != sizeof(TestConfig) || config.pid != GetCurrentProcessId()) return 3;
    HMODULE core = GetModuleHandleW(L"Weixin.dll");
    if (!core || !core_fingerprint(core)) return 4;
    core_base = reinterpret_cast<uintptr_t>(core);
    struct TextRequest { SmallString receiver; SmallString body; } request{};
    request.receiver.capacity = request.body.capacity = 15;
    SmallString description{};
    description.capacity = 15;
    if (!set_inline(&request.receiver, "filehelper") ||
        !set_native_string(&request.body, config.text) ||
        !set_inline(&description, "wxbot.selftest")) return 13;
    Completion* completion = new Completion{1, 0, completion_invoke, completion_destroy, config};
    _InterlockedIncrement(&live_completions);
    TestResult report{};
    __try {
        // The native builder owns scheduling, message allocation and the
        // completion reference. No UI input or user message is required.
        using Builder = void (__fastcall*)(void*, TextRequest*, SmallString*, Completion**);
        reinterpret_cast<Builder>(core_address(core_base, 0x54b02f0))(nullptr, &request, &description, &completion);
        report.stage = 20;
    } __except (record_exception(GetExceptionInformation(), &report)) {
        write_report(config, report, "WXNATIVE_TEST_3");
    }
    if (completion) completion_destroy(completion);
    free_native_string(&request.body);
    return report.exception ? 5 : 0;
}

struct TaskLease {
    TestConfig config;
    explicit TaskLease(const TestConfig& value) : config(value) {
        _InterlockedIncrement(&live_tasks);
    }
    ~TaskLease() { _InterlockedDecrement(&live_tasks); }
};

static DWORD schedule_task(const TestConfig& config, std::function<void()>* callable) {
    static_assert(sizeof(std::function<void()>) == 64, "Unexpected function ABI");
    struct Location { const char* function; const char* file; uint32_t line; uint32_t pad; void* caller; };
    static_assert(sizeof(Location) == 32, "Unexpected source location ABI");
    Location location{"wxbot.selftest", "selftest_bridge.cpp", 1, 0, nullptr};
    SharedMessage scheduler{}, future{};
    TestResult error{};
    __try {
        __try {
            // 0x45d10 reads the process-wide application pointer; 0x47310 copies
            // its owned scheduler. These helpers do not require a task-local TLS.
            using App = void* (__fastcall*)();
            using Scheduler = SharedMessage* (__fastcall*)(void*, SharedMessage*);
            void* app = reinterpret_cast<App>(core_address(core_base, 0x45d10))();
            if (!app) return ERROR_NOT_READY;
            reinterpret_cast<Scheduler>(core_address(core_base, 0x47310))(app, &scheduler);
            if (!scheduler.object || !scheduler.control) return ERROR_NOT_READY;
            using Schedule = SharedMessage* (__fastcall*)(void*, SharedMessage*, Location*, std::function<void()>*, uint32_t);
            reinterpret_cast<Schedule>(core_address(core_base, 0x2fb010))(scheduler.object, &future, &location, callable, 1);
        } __finally {
            guarded_release_shared(&future, &error);
            guarded_release_shared(&scheduler, &error);
        }
    } __except (record_exception(GetExceptionInformation(), &error)) {
    }
    if (error.exception) {
        // Research-only diagnostic encoded return; not a Win32 status.
        write_report(config, error, "WXNATIVE_TEST_3");
        return 5;
    }
    return 0;
}

static DWORD WINAPI WxDispatchSelfTest(void* parameter) {
    if (!parameter || live_completions != 0 || live_tasks != 0) return ERROR_BUSY;
    const TestConfig config = *static_cast<const TestConfig*>(parameter);
    if (config.size != sizeof(TestConfig) || config.pid != GetCurrentProcessId()) return 3;
    const size_t text_length = strnlen_s(config.text, sizeof(config.text));
    if (!text_length || text_length >= sizeof(config.text) ||
        !MultiByteToWideChar(CP_UTF8, MB_ERR_INVALID_CHARS, config.text, static_cast<int>(text_length), nullptr, 0)) return 13;
    HMODULE core = GetModuleHandleW(L"Weixin.dll");
    if (!core || !core_fingerprint(core)) return 4;
    core_base = reinterpret_cast<uintptr_t>(core);
    auto lease = std::make_shared<TaskLease>(config);
    std::function<void()> task = [lease]() {
        _InterlockedIncrement(&active_callbacks);
        dispatch_on_native_task(&lease->config);
        _InterlockedDecrement(&active_callbacks);
    };
    return schedule_task(config, &task);
}

static bool read_pb_string(const unsigned char* message, size_t offset, char* output, size_t capacity) {
    const auto wrapper = *reinterpret_cast<const unsigned char* const*>(message + offset);
    if (!wrapper) return false;
    const auto string = *reinterpret_cast<const SmallString* const*>(wrapper + 8);
    if (!string || string->length >= capacity || string->capacity < string->length || string->capacity > 1024*1024) return false;
    const char* bytes = string->capacity > 15 ? *reinterpret_cast<const char* const*>(string->data) : string->data;
    std::memcpy(output, bytes, static_cast<size_t>(string->length));
    output[string->length] = 0;
    return std::strlen(output)==string->length;
}

static bool observed_type(uint32_t type) {
    return type==1 || type==3 || type==34 || type==37 || type==42 || type==43 || type==47 || type==48 || type==49 || type==50 || type==62 || type==10000 || type==10002;
}
static void sample_receive(const uintptr_t* batch) {
    __try {
        if (!batch || batch[1] < batch[0] || (batch[1]-batch[0])%0x78 || (batch[1]-batch[0])/0x78>512) {receive_gap();return;}
        for (uintptr_t address=batch[0]; address<batch[1]; address+=0x78) {
            const auto message=reinterpret_cast<const unsigned char*>(address);
            char sender[128]{},receiver[128]{};
            if (!read_pb_string(message,8,sender,sizeof(sender)) || !read_pb_string(message,0x18,receiver,sizeof(receiver)) ||
                !receive_conversation(receive_login,sender,receiver)) {receive_gap();continue;}
            const uint32_t type=*reinterpret_cast<const uint32_t*>(message+0x14);
            if (!observed_type(type)) {receive_gap();continue;}
            // Profile1 constructor RVA A50E50, branch A5152C..A51668:
            // type34 copies AddMsg+30 -> buffer(+8 string,+10 declared) to
            // native voice field +98. Copy before original_receive releases it.
            auto content=static_cast<char*>(HeapAlloc(GetProcessHeap(),HEAP_ZERO_MEMORY,16384));
            if (!content) {receive_gap();continue;}
            __try {
                if (!read_pb_string(message,0x20,content,16384) || !content[0] ||
                    !MultiByteToWideChar(CP_UTF8,MB_ERR_INVALID_CHARS,content,static_cast<int>(std::strlen(content)),nullptr,0)) {receive_gap();continue;}
                const char* voice=nullptr;uint32_t voice_size=0;
                if(type==34) {
                    __try {
                        const auto buffer=*reinterpret_cast<const unsigned char*const*>(message+0x30);
                        if(buffer){
                            const uint32_t n=*reinterpret_cast<const uint32_t*>(buffer+0x10);
                            const auto value=*reinterpret_cast<const SmallString*const*>(buffer+8);
                            if(value && n && n<=262144 && value->length==n && value->capacity>=n &&
                               (value->capacity==15 || (value->capacity>=16 && value->capacity<=1048576))){
                                voice=value->capacity>15?*reinterpret_cast<const char*const*>(value->data):value->data;
                                if(voice)voice_size=n;
                            }
                        }
                    } __except(EXCEPTION_EXECUTE_HANDLER){voice=nullptr;voice_size=0;}
                }
                if (receive_queue_mode) enqueue_received(sender,receiver,content,nullptr,type,nullptr,0,*reinterpret_cast<const uint64_t*>(message+0x50),voice,voice_size);
            } __finally {SecureZeroMemory(content,16384);HeapFree(GetProcessHeap(),0,content);}
        }
    } __except(EXCEPTION_EXECUTE_HANDLER) {receive_gap();}
}

static uintptr_t __fastcall hooked_receive(void* manager, const uintptr_t* batch, uint8_t flag1, uint8_t flag2) {
    _InterlockedIncrement(&active_callbacks);
    uintptr_t result = 0;
    __try {
        sample_receive(batch);
        result = original_receive(manager, batch, flag1, flag2);
    } __finally { _InterlockedDecrement(&active_callbacks); }
    return result;
}

static DWORD start_receive(void* parameter, bool queue_mode) {
    if (!parameter || hook_created || receive_hook_created) return ERROR_BUSY;
    const TestConfig config = *static_cast<const TestConfig*>(parameter);
    if (config.size != sizeof(TestConfig) || config.pid != GetCurrentProcessId()) return 3;
    HMODULE core = GetModuleHandleW(L"Weixin.dll");
    if (!core || !core_fingerprint(core)) return 4;
    core_base = reinterpret_cast<uintptr_t>(core);
    receive_config = config;
    receive_queue_mode = queue_mode;
    AcquireSRWLockExclusive(&receive_lock);
    receive_head = receive_count = receive_dropped = 0;
    receive_sequence = 0;
    SecureZeroMemory(&receive_inflight,sizeof(receive_inflight));
    receive_acked = 0;
    ReleaseSRWLockExclusive(&receive_lock);
    MH_STATUS status = MH_Initialize();
    if (status != MH_OK) return 600 + status;
    void* target = reinterpret_cast<void*>(core_address(core_base, 0x17b9010));
    status = MH_CreateHook(target, reinterpret_cast<void*>(&hooked_receive), reinterpret_cast<void**>(&original_receive));
    if (status != MH_OK) { MH_Uninitialize(); return 600 + status; }
    status = MH_EnableHook(target);
    if (status != MH_OK) { MH_RemoveHook(target); MH_Uninitialize(); return 600 + status; }
    receive_hook_created = true;
    return 0;
}

static DWORD WINAPI WxArmReceiveTest(void* parameter) {
    return start_receive(parameter, false);
}

static DWORD WINAPI WxStartFileHelperReceiver(void* parameter) {
    return start_receive(parameter, true);
}

static DWORD poll_received(ReceivedMessage* output) {
    if (!output) return ERROR_INVALID_PARAMETER;
    _InterlockedIncrement(&active_callbacks);
    AcquireSRWLockExclusive(&receive_lock);
    *output = {};
    output->size = sizeof(*output);
    if (!receive_inflight.present && receive_count) {
        receive_inflight = received_messages[receive_head];
        SecureZeroMemory(&received_messages[receive_head], sizeof(ReceivedMessage));
        receive_head = (receive_head + 1) % 64;
        --receive_count;
    }
    if (receive_inflight.present) *output = receive_inflight;
    output->dropped = receive_dropped;
    ReleaseSRWLockExclusive(&receive_lock);
    _InterlockedDecrement(&active_callbacks);
    return 0;
}

static DWORD ack_received(uint64_t sequence) {
    AcquireSRWLockExclusive(&receive_lock);
    DWORD result=ERROR_INVALID_PARAMETER;
    if (sequence && sequence==receive_acked) result=0;
    else if (sequence && receive_inflight.present && receive_inflight.sequence==sequence) {
        SecureZeroMemory(&receive_inflight,sizeof(receive_inflight));
        receive_acked=sequence;result=0;
    }
    ReleaseSRWLockExclusive(&receive_lock);
    return result;
}

static DWORD WINAPI WxPollFileHelperMessage(void* parameter) {
    // This export predates extended text/media receive and always writes V1.
    if (!parameter) return ERROR_INVALID_PARAMETER;
    ReceivedMessage message{};
    const DWORD code = poll_received(&message);
    std::memcpy(parameter, &message, 1312);
    *static_cast<uint32_t*>(parameter) = 1312;
    return code;
}

static DWORD WINAPI WxWaitForTasks(void*) {
    for (int attempt = 0; (active_callbacks != 0 || live_completions != 0 || live_tasks != 0) && attempt < 100; ++attempt) Sleep(50);
    return active_callbacks != 0 || live_completions != 0 || live_tasks != 0 ? ERROR_BUSY : 0;
}

static DWORD WINAPI WxRemoveSelfTest(void*) {
#ifdef WXBOT_LOCAL_RECALL
    InterlockedExchange(&local_recall_enabled, 0);
#endif
    const auto target = reinterpret_cast<void*>(core_address(core_base, 0x178f100));
    MH_STATUS status = MH_OK;
    if (hook_created) {
        status = MH_DisableHook(target);
        if (status != MH_OK && status != MH_ERROR_DISABLED) return 600 + status;
    }
    void* receive_target = reinterpret_cast<void*>(core_address(core_base, 0x17b9010));
    if (receive_hook_created) {
        status = MH_DisableHook(receive_target);
        if (status != MH_OK && status != MH_ERROR_DISABLED) return 600 + status;
    }
#ifdef WXBOT_RECALL_RECEIVE
    void* recall_receive_target = reinterpret_cast<void*>(core_address(core_base, 0x229e790));
    if (recall_receive_hook_created) {
        status = MH_DisableHook(recall_receive_target);
        if (status != MH_OK && status != MH_ERROR_DISABLED) return 600 + status;
    }
#endif
#ifdef WXBOT_FRIEND_RECEIVE
    void* friend_receive_target = reinterpret_cast<void*>(core_address(core_base, 0x3866510));
    if (friend_receive_hook_created) {
        status = MH_DisableHook(friend_receive_target);
        if (status != MH_OK && status != MH_ERROR_DISABLED) return 600 + status;
    }
#endif
    void* factory_target = reinterpret_cast<void*>(core_address(core_base, 0x6f4c70));
    if (factory_hook_created) {
        status = MH_DisableHook(factory_target);
        if (status != MH_OK && status != MH_ERROR_DISABLED) return 600 + status;
    }
#ifdef WXBOT_QUOTE_SEND
    auto quote_manager_target = quote_manager_hook_created
        ? reinterpret_cast<void*>(core_address(core_base, 0x6f4d90)) : nullptr;
    if (quote_manager_hook_created) {
        status = MH_DisableHook(quote_manager_target);
        if (status != MH_OK && status != MH_ERROR_DISABLED) return 600 + status;
    }
#endif
#ifdef WXBOT_LOCAL_RECALL
    auto local_recall_target = local_recall_hook_created
        ? reinterpret_cast<void*>(core_address(core_base, 0x1852d70)) : nullptr;
    if (local_recall_hook_created) {
        status = MH_DisableHook(local_recall_target);
        if (status != MH_OK && status != MH_ERROR_DISABLED) return 600 + status;
    }
#endif
    void* image_factory_target = reinterpret_cast<void*>(core_address(core_base, 0x5240720));
    if (image_download_factory_hook_created) {
        status = MH_DisableHook(image_factory_target);
        if (status != MH_OK && status != MH_ERROR_DISABLED) return 600 + status;
    }
    for (int attempt = 0; (active_callbacks != 0 || live_completions != 0 || live_tasks != 0) && attempt < 100; ++attempt) Sleep(50);
    if (active_callbacks != 0 || live_completions != 0 || live_tasks != 0) return ERROR_BUSY;
    bool hooks_present = hook_created || receive_hook_created || factory_hook_created ||
        image_download_factory_hook_created;
#ifdef WXBOT_RECALL_RECEIVE
    hooks_present = hooks_present || recall_receive_hook_created;
#endif
#ifdef WXBOT_FRIEND_RECEIVE
    hooks_present = hooks_present || friend_receive_hook_created;
#endif
#ifdef WXBOT_QUOTE_SEND
    hooks_present = hooks_present || quote_manager_hook_created;
#endif
#ifdef WXBOT_LOCAL_RECALL
    hooks_present = hooks_present || local_recall_hook_created;
#endif
    if (!hooks_present) return 0;
    if (hook_created) {
        status = MH_RemoveHook(target);
        if (status != MH_OK) return 600 + status;
    }
    if (receive_hook_created) {
        status = MH_RemoveHook(receive_target);
        if (status != MH_OK) return 600 + status;
    }
#ifdef WXBOT_RECALL_RECEIVE
    if (recall_receive_hook_created) {
        status = MH_RemoveHook(recall_receive_target);
        if (status != MH_OK) return 600 + status;
    }
#endif
#ifdef WXBOT_FRIEND_RECEIVE
    if (friend_receive_hook_created) {
        status = MH_RemoveHook(friend_receive_target);
        if (status != MH_OK) return 600 + status;
    }
#endif
    if (factory_hook_created) {
        status = MH_RemoveHook(factory_target);
        if (status != MH_OK) return 600 + status;
    }
    if (image_download_factory_hook_created) {
        status = MH_RemoveHook(image_factory_target);
        if (status != MH_OK) return 600 + status;
    }
#ifdef WXBOT_QUOTE_SEND
    if (quote_manager_hook_created) {
        status = MH_RemoveHook(quote_manager_target);
        if (status != MH_OK) return 600 + status;
    }
#endif
#ifdef WXBOT_LOCAL_RECALL
    if (local_recall_hook_created) {
        status = MH_RemoveHook(local_recall_target);
        if (status != MH_OK) return 600 + status;
    }
#endif
    status = MH_Uninitialize();
    if (status == MH_OK) {
        hook_created = false; receive_hook_created = false; factory_hook_created = false;
        image_download_factory_hook_created = false;
#ifdef WXBOT_LOCAL_RECALL
        local_recall_hook_created = false;
#endif
#ifdef WXBOT_QUOTE_SEND
        quote_manager_hook_created = false;
#endif
#ifdef WXBOT_RECALL_RECEIVE
        recall_receive_hook_created = false;
#endif
#ifdef WXBOT_FRIEND_RECEIVE
        friend_receive_hook_created = false;
#endif
        if (factory_tls != TLS_OUT_OF_INDEXES) { TlsFree(factory_tls); factory_tls = TLS_OUT_OF_INDEXES; }
    }
    return status == MH_OK ? 0 : 600 + status;
}

BOOL WINAPI DllMain(HINSTANCE module, DWORD reason, void*) {
    if (reason == DLL_PROCESS_ATTACH) DisableThreadLibraryCalls(module);
    return TRUE;
}



