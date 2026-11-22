// Adapted from helper article_bridge.cpp under its noncommercial license.
// Only a fixed history read is exposed; no arbitrary JS, cookies or native RPC.
#include <set>
#include <stdexcept>
#include <charconv>
#include <cmath>
#include <cctype>
#include "article-json.inc"
namespace article_native {
constexpr char FEED_URL[]="weixin://resourceid/SubscriptionDisorder/disorder.html";
constexpr char HISTORY_URL[]="/cgi-bin/mmbiz-bin/bizattr/bizprofilev2h5";
static uintptr_t host=0;
static uintptr_t host_address(uintptr_t legacy){
    if(core_profile_id==1)return host+legacy;
    if((core_profile_id!=5&&core_profile_id!=14))return 0;
    switch(legacy){
        case 0x5ccf18:return host+0x5d3720;
        case 0x4e02d0:return host+0x4e71e0;
        case 0x22c850:return host+0x1cb6b0;
        case 0x4e31d8:return host+0x4ea3e0;
        default:return 0;
    }
}
using H5=void(__fastcall*)(void*,void*,uint32_t,const SmallString*,void*,void*);
using Completion=void(__fastcall*)(void*,void*);
using Execute=void(__fastcall*)(void*,uint32_t,const char*);
static H5 original_h5=nullptr;
static Completion original_completion=nullptr;
static bool hook_h5=false,hook_completion=false;
static volatile LONG active=0;
static SRWLOCK lock=SRWLOCK_INIT;
struct Lock {Lock(){AcquireSRWLockExclusive(&lock);}~Lock(){ReleaseSRWLockExclusive(&lock);}};
struct Request {
    WercPacket packet{};std::vector<unsigned char> offset;std::string response;
    uint32_t frame=0,stage=0,error=0,seen_frame=0,seen_params=0,seen_history=0;void* callback=nullptr;
    ~Request(){if(!offset.empty())SecureZeroMemory(offset.data(),offset.size());if(!response.empty())SecureZeroMemory(response.data(),response.size());SecureZeroMemory(&packet,sizeof(packet));}
};
static std::unique_ptr<Request> request;
bool fingerprint(HMODULE module, uint64_t expected_size, const unsigned char* expected) {
    wchar_t path[32768]{};
    if (!module || !GetModuleFileNameW(module, path, static_cast<DWORD>(std::size(path)))) return false;
    HANDLE file = CreateFileW(path, GENERIC_READ, FILE_SHARE_READ | FILE_SHARE_DELETE,
                             nullptr, OPEN_EXISTING, FILE_FLAG_SEQUENTIAL_SCAN, nullptr);
    if (file == INVALID_HANDLE_VALUE) return false;
    LARGE_INTEGER size{};
    bool valid = GetFileSizeEx(file, &size) && static_cast<uint64_t>(size.QuadPart) == expected_size;
    BCRYPT_ALG_HANDLE algorithm = nullptr;
    BCRYPT_HASH_HANDLE hash = nullptr;
    if (valid && BCryptOpenAlgorithmProvider(&algorithm, BCRYPT_SHA256_ALGORITHM, nullptr, 0) >= 0) {
        if (BCryptCreateHash(algorithm, &hash, nullptr, 0, nullptr, 0, 0) >= 0) {
            unsigned char buffer[65536]{}, digest[32]{};
            DWORD amount = 0;
            while (valid) {
                valid = ReadFile(file, buffer, sizeof(buffer), &amount, nullptr) != FALSE;
                if (!valid || !amount) break;
                valid = BCryptHashData(hash, buffer, amount, 0) >= 0;
            }
            valid = valid && BCryptFinishHash(hash, digest, sizeof(digest), 0) >= 0 &&
                std::equal(digest, digest + 32, expected);
            SecureZeroMemory(buffer, sizeof(buffer));
            BCryptDestroyHash(hash);
        } else valid = false;
        BCryptCloseAlgorithmProvider(algorithm, 0);
    } else valid = false;
    CloseHandle(file);
    return valid;
}

struct Tree { uintptr_t begin, root; uint64_t count; };
struct Frame { uintptr_t manager, creation, handler; uint32_t id; Tree entries, handlers; };
// This reads only the exact builtin URI prefix, never its potentially private query.
bool feed_string(uintptr_t node) {
    __try {
        unsigned char header[24]{};
        std::memcpy(header, reinterpret_cast<const void*>(node + 0xe8), sizeof(header));
        const bool heap = (header[23] & 0x80) != 0;
        const auto length = heap ? *reinterpret_cast<const uint64_t*>(header + 8) : header[23];
        const auto capacity = heap ? *reinterpret_cast<const uint64_t*>(header + 16) & 0x7fffffffffffffffull : 22ull;
        const char* data = heap ? *reinterpret_cast<const char* const*>(header) : reinterpret_cast<const char*>(node + 0xe8);
        const size_t prefix = sizeof(FEED_URL) - 1;
        if (!data || length < prefix || length > 4096 || capacity < length || std::memcmp(data, FEED_URL, prefix)) return false;
        if (length > prefix && data[prefix] != '?') return false;
        return !std::memcmp(header, reinterpret_cast<const void*>(node + 0xe8), sizeof(header));
    } __except (EXCEPTION_EXECUTE_HANDLER) { return false; }
}
bool tree_snapshot(uintptr_t manager, Tree* entries, Tree* handlers) {
    __try {
        std::memcpy(entries, reinterpret_cast<const void*>(manager + 0xa0), sizeof(Tree));
        std::memcpy(handlers, reinterpret_cast<const void*>(manager + 0x40), sizeof(Tree));
        return entries->count && entries->count <= 128 && handlers->count && handlers->count <= 128;
    } __except (EXCEPTION_EXECUTE_HANDLER) { return false; }
}
bool frame_impl(Frame* frame) {
    const auto manager = *reinterpret_cast<const uintptr_t*>(host_address(0x5ccf18));
    if (!manager || *reinterpret_cast<const uintptr_t*>(manager) != host_address(0x4e02d0) ||
        *reinterpret_cast<const uintptr_t*>(host_address(0x4e02d0) + 0x58) != host_address(0x22c850)) return false;
    Tree entries{}, handlers{};
    if (!tree_snapshot(manager, &entries, &handlers)) return false;
    struct Branch { uintptr_t node, parent; uint64_t low, high; };
    std::vector<Branch> todo{{entries.root, 0, 0, 0x100000000ull}}; std::set<uintptr_t> visited;
    uintptr_t chosen = 0; uint32_t identifier = 0;
    while (!todo.empty()) {
        const auto item = todo.back(); todo.pop_back(); const auto node = item.node;
        if (!node) continue;
        if (visited.size() >= entries.count || !visited.insert(node).second) return false;
        const auto left = *reinterpret_cast<const uintptr_t*>(node), right = *reinterpret_cast<const uintptr_t*>(node + 8);
        const auto key = *reinterpret_cast<const uint32_t*>(node + 0x20);
        if ((item.parent && *reinterpret_cast<const uintptr_t*>(node + 0x10) != item.parent) || key <= item.low || key >= item.high) return false;
        if (left) todo.push_back({left, node, item.low, key}); if (right) todo.push_back({right, node, key, item.high});
        if (*reinterpret_cast<const uintptr_t*>(node + 0x28) != host_address(0x4e31d8) || !feed_string(node)) continue;
        if (!key || key > INT32_MAX || chosen) return false;
        chosen = node; identifier = key;
    }
    if (!chosen || visited.size() != entries.count) return false;
    unsigned char creation_header[48]{}, url_header[24]{};
    std::memcpy(creation_header, reinterpret_cast<const void*>(chosen), sizeof(creation_header));
    std::memcpy(url_header, reinterpret_cast<const void*>(chosen + 0xe8), sizeof(url_header));
    todo = {{handlers.root, 0, 0, 0x100000000ull}}; visited.clear(); uintptr_t handler = 0;
    while (!todo.empty()) {
        const auto item = todo.back(); todo.pop_back(); const auto node = item.node;
        if (!node) continue;
        if (visited.size() >= handlers.count || !visited.insert(node).second) return false;
        const auto left = *reinterpret_cast<const uintptr_t*>(node), right = *reinterpret_cast<const uintptr_t*>(node + 8);
        const auto key = *reinterpret_cast<const uint32_t*>(node + 0x20);
        if ((item.parent && *reinterpret_cast<const uintptr_t*>(node + 0x10) != item.parent) || key <= item.low || key >= item.high) return false;
        if (left) todo.push_back({left, node, item.low, key}); if (right) todo.push_back({right, node, key, item.high});
        if (key == identifier) {
            if (handler || !*reinterpret_cast<const uintptr_t*>(node + 0x28)) return false;
            handler = node;
        }
    }
    unsigned char handler_header[48]{};
    if (handler) std::memcpy(handler_header, reinterpret_cast<const void*>(handler), sizeof(handler_header));
    Tree after_entries{}, after_handlers{};
    if (!handler || visited.size() != handlers.count || !tree_snapshot(manager, &after_entries, &after_handlers) ||
        std::memcmp(&entries, &after_entries, sizeof(Tree)) || std::memcmp(&handlers, &after_handlers, sizeof(Tree)) ||
        *reinterpret_cast<const uintptr_t*>(host_address(0x5ccf18)) != manager ||
        *reinterpret_cast<const uintptr_t*>(manager) != host_address(0x4e02d0) ||
        *reinterpret_cast<const uintptr_t*>(host_address(0x4e02d0) + 0x58) != host_address(0x22c850) || !feed_string(chosen) ||
        std::memcmp(creation_header, reinterpret_cast<const void*>(chosen), sizeof(creation_header)) ||
        std::memcmp(url_header, reinterpret_cast<const void*>(chosen + 0xe8), sizeof(url_header)) ||
        std::memcmp(handler_header, reinterpret_cast<const void*>(handler), sizeof(handler_header))) return false;
    *frame = {manager, chosen, handler, identifier, entries, handlers};
    return true;
}
bool find_frame(Frame* frame) {
    __try { return frame_impl(frame); }
    __except (EXCEPTION_EXECUTE_HANDLER) { return false; }
}

class Json {
    const std::string& text;
    size_t position = 0;
    void whitespace() { while (position < text.size() && (text[position] == ' ' || text[position] == '\r' || text[position] == '\n' || text[position] == '\t')) ++position; }
    bool string(std::string* result) {
        if (position >= text.size() || text[position++] != '"') return false;
        result->clear();
        while (position < text.size()) {
            unsigned char ch = text[position++];
            if (ch == '"') return true;
            if (ch < 32) return false;
            if (ch != '\\') { result->push_back(static_cast<char>(ch)); continue; }
            if (position == text.size()) return false;
            ch = text[position++];
            if (ch == '"' || ch == '\\' || ch == '/') result->push_back(static_cast<char>(ch));
            else if (ch == 'b') result->push_back('\b'); else if (ch == 'f') result->push_back('\f');
            else if (ch == 'n') result->push_back('\n'); else if (ch == 'r') result->push_back('\r'); else if (ch == 't') result->push_back('\t');
            else if (ch == 'u') {
                if (position + 4 > text.size()) return false;
                unsigned value = 0;
                for (unsigned i = 0; i < 4; ++i) {
                    const char digit = text[position++];
                    unsigned decoded = digit >= '0' && digit <= '9' ? digit - '0' : digit >= 'a' && digit <= 'f' ? digit - 'a' + 10 : digit >= 'A' && digit <= 'F' ? digit - 'A' + 10 : 99;
                    if (decoded > 15) return false; value = value * 16 + decoded;
                }
                if (value > 127) return false; result->push_back(static_cast<char>(value));
            } else return false;
        }
        return false;
    }
    bool skip(unsigned depth) {
        whitespace(); if (position >= text.size() || depth > 16) return false;
        if (text[position] == '"') { std::string unused; return string(&unused); }
        if (text[position] == '{' || text[position] == '[') {
            const char ending = text[position++] == '{' ? '}' : ']';
            bool quoted = false, escaped = false; unsigned nested = 1;
            while (position < text.size()) {
                const char ch = text[position++];
                if (quoted) { if (escaped) escaped = false; else if (ch == '\\') escaped = true; else if (ch == '"') quoted = false; continue; }
                if (ch == '"') quoted = true;
                else if (ch == '{' || ch == '[') { if (++nested > 16) return false; }
                else if (ch == '}' || ch == ']') { if (!--nested) return ch == ending; }
            }
            return false;
        }
        const size_t start = position;
        while (position < text.size() && text[position] != ',' && text[position] != '}' && text[position] != ']' && text[position] != ' ' && text[position] != '\r' && text[position] != '\n' && text[position] != '\t') ++position;
        return position > start;
    }
public:
    explicit Json(const std::string& input): text(input) {}
    bool member(const char* wanted, std::string* value, bool* quoted) {
        whitespace(); if (position >= text.size() || text[position++] != '{') return false;
        std::set<std::string> keys; bool found = false;
        while (true) {
            whitespace(); if (position < text.size() && text[position] == '}') { ++position; break; }
            std::string key; if (!string(&key) || key.size() > 256 || !keys.insert(key).second || keys.size() > 128) return false;
            whitespace(); if (position >= text.size() || text[position++] != ':') return false; whitespace();
            if (key == wanted) {
                *quoted = position < text.size() && text[position] == '"';
                if (*quoted) { if (!string(value)) return false; }
                else { const size_t start = position; if (!skip(0)) return false; *value = text.substr(start, position - start); }
                found = true;
            } else if (!skip(0)) return false;
            whitespace(); if (position >= text.size()) return false;
            if (text[position] == '}') { ++position; break; }
            if (text[position++] != ',') return false;
        }
        whitespace(); return found && position == text.size();
    }
};
bool member(const std::string& source, const char* key, const std::string& expected, bool quoted = true) {
    std::string value; bool is_string = false;
    return Json(source).member(key, &value, &is_string) && is_string == quoted && value == expected;
}

#include "article-request.inc"
