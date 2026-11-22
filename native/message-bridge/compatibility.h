#pragma once
#include <cstring>
#include <cwchar>
#include <string>

// These are two variants of the same native layout, not arbitrary patches.
enum class CoreVariant { WeChat42053, Unsupported, Original, AntiRecall, WeChat42052, WeChat42013, WeChat4209, WeChat42012, WeChat411550, WeChat411553, WeChat41137, WeChat411310, WeChat41159 };
inline CoreVariant core_variant(const char* sha256) {
    if (!std::strcmp(sha256,"ac6b174d98ee1b69e95c99a7a39c3197c8c167b7c6d32f5b72c7a7659c104825")) return CoreVariant::WeChat42053;
    if (!std::strcmp(sha256,"d733489eea0b62af77e1aa61ba645cd941583d2a64fb2d7572da818d285f8d84")) return CoreVariant::WeChat42052;
    if (!std::strcmp(sha256,"afe3d02c58bada3c35009c0cd58ee1f57b2a00fe53237d0dce649107fddb9a20")) return CoreVariant::WeChat42013;
    if (!std::strcmp(sha256,"a81c4770350a4edf3e774413da832a785d7aac50025471b30ea26d1f846b9fb4")) return CoreVariant::WeChat4209;
    if (!std::strcmp(sha256,"2b3677c3f932daaa4ef84c8b2acd0aee12cbe5a779ff9e740c370761e5c291ce")) return CoreVariant::WeChat42012;
    if (!std::strcmp(sha256,"0cf6379405432385a1c442766443450f26a7689f45e53c1b4ad2822061f3113b")) return CoreVariant::WeChat411550;
    if (!std::strcmp(sha256,"88eabf78f961e0abe26b397521b05f4ba6cd752727fb9db74a2b3f660b0400f2")) return CoreVariant::WeChat411553;
    if (!std::strcmp(sha256,"f4c4118d85ee743a0832c301288edf77b041e02f84b73d7905ffe10090206d01")) return CoreVariant::WeChat41137;
    if (!std::strcmp(sha256,"4b49d0ddcef4b83e25d1d0663759369a4576cb8035a61f2f77d2792688a35725")) return CoreVariant::WeChat411310;
    if (!std::strcmp(sha256,"1b599c66ada05ab12f9850cffe9d44314abb0440fde3d74623ada1c71dc94508")) return CoreVariant::WeChat41159;
    if (!std::strcmp(sha256,"10f8e995453e2da46d4f2b5080cd6da1f13cc5147746adc119ceae38cb039de5")) return CoreVariant::Original;
    if (!std::strcmp(sha256,"ff8fa05d5497c0497765439cc886436868610d10ca3b21131642482311821c4f")) return CoreVariant::AntiRecall;
    return CoreVariant::Unsupported;
}
inline bool core_install_layout(const std::wstring& executable, const std::wstring& module, CoreVariant variant=CoreVariant::Unsupported) {
    const auto last=executable.find_last_of(L"\\/");
    if(last==std::wstring::npos)return false;
    const auto root=executable.substr(0,last+1);
    if(module.size()<=root.size() || _wcsnicmp(module.c_str(),root.c_str(),root.size()))return false;
    const auto relative=module.substr(root.size());
    const auto slash=relative.find_first_of(L"\\/");
    if(slash==std::wstring::npos)return !_wcsicmp(relative.c_str(),L"Weixin.dll");
    const auto second=relative.find_first_of(L"\\/",slash+1);
    if(second==std::wstring::npos) {
        const auto folder=relative.substr(0,slash);
        return !folder.empty() && folder!=L"." && folder!=L".." &&
            !_wcsicmp(relative.c_str()+slash+1,L"Weixin.dll");
    }
    if(variant!=CoreVariant::WeChat41137 && variant!=CoreVariant::WeChat411310 && variant!=CoreVariant::WeChat41159)return false;
    const auto first=relative.substr(0,slash),middle=relative.substr(slash+1,second-slash-1);
    return !first.empty() && !middle.empty() && first!=L"." && first!=L".." && middle!=L"." && middle!=L".."
        && relative.find_first_of(L"\\/",second+1)==std::wstring::npos
        && !_wcsicmp(relative.c_str()+second+1,L"Weixin.dll");
}





