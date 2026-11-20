//! Automatic exact-build selection. A known upstream address table is not a
//! validated WeRC adapter: additional WeRC calls and layouts must also match.
use werc_domain::{AppError, AppResult};

#[derive(Clone, Copy, Debug)]
pub enum NativeFeature {
    Messaging,
    Quotes,
    Mentions,
    MomentsPublish,
    MomentsActions,
    MomentsReply,
    ArticleHistory,
    FavoriteDownload,
}
impl NativeFeature {
    pub fn id(self) -> &'static str {
        match self {
            Self::Messaging => "messages.native",
            Self::Quotes => "messages.send_quote",
            Self::Mentions => "messages.send_mentions",
            Self::MomentsPublish => "moments.publish",
            Self::MomentsActions => "moments.interact",
            Self::MomentsReply => "moments.reply",
            Self::ArticleHistory => "articles.history",
            Self::FavoriteDownload => "favorites.download",
        }
    }
}
pub struct NativeProfile {
    pub version: &'static str,
    pub sha256: &'static str,
    pub layout: u32,
    pub enabled: bool,
}
impl NativeProfile {
    pub fn supports(&self, feature: NativeFeature) -> bool {
        self.enabled
            && match self.layout {
                1 => true,
                5 => matches!(
                    feature,
                    NativeFeature::Messaging
                        | NativeFeature::Mentions
                        | NativeFeature::Quotes
                        | NativeFeature::MomentsReply
                        | NativeFeature::FavoriteDownload
                        | NativeFeature::MomentsPublish
                        | NativeFeature::MomentsActions
                        | NativeFeature::ArticleHistory
                ),
                11 => true,
                _ => false,
            }
    }
    pub fn require(&self, feature: NativeFeature) -> AppResult<()> {
        if self.supports(feature) {
            Ok(())
        } else {
            Err(AppError::new(
                "NATIVE_FEATURE_UNAVAILABLE",
                &format!(
                    "微信 {} 的当前精确构建尚未开放 {}",
                    self.version,
                    feature.id()
                ),
            ))
        }
    }
    pub fn capabilities(&self) -> Vec<werc_domain::capability::Capability> {
        use werc_domain::capability::{Availability, Capability, Evidence};
        [
            NativeFeature::Messaging,
            NativeFeature::Quotes,
            NativeFeature::Mentions,
            NativeFeature::MomentsPublish,
            NativeFeature::MomentsActions,
            NativeFeature::MomentsReply,
            NativeFeature::ArticleHistory,
            NativeFeature::FavoriteDownload,
        ]
        .into_iter()
        .map(|feature| {
            let ready = self.supports(feature);
            Capability {
                id: feature.id().into(),
                evidence: Evidence::Implemented,
                availability: if ready {
                    Availability::Ready
                } else {
                    Availability::Blocked
                },
                reason: format!(
                    "微信 {} · 精确构建档案 {} · {}",
                    self.version,
                    self.layout,
                    if ready {
                        "已启用后台原生适配"
                    } else {
                        "尚未通过此构建验证，已关闭"
                    }
                ),
                execution_mode: Some("native".into()),
                requires_foreground: false,
                requires_user_action: false,
            }
        })
        .collect()
    }
}
pub const PROFILES: &[NativeProfile] = &[
    NativeProfile {
        version: "4.1.15.13",
        sha256: "10f8e995453e2da46d4f2b5080cd6da1f13cc5147746adc119ceae38cb039de5",
        layout: 1,
        enabled: true,
    },
    NativeProfile {
        version: "4.1.15.13",
        sha256: "ff8fa05d5497c0497765439cc886436868610d10ca3b21131642482311821c4f",
        layout: 1,
        enabled: true,
    },
    NativeProfile {
        version: "4.1.15.56",
        sha256: "e0428a7b691853de496532523c2c09aa095f90941c37c9f4cfb3f137f3c66556",
        layout: 2,
        enabled: false,
    },
    NativeProfile {
        version: "4.2.0.11",
        sha256: "745cdf2955fa5128cdb00c056c97571156a5f107406a9bb15ac0754f3246b201",
        layout: 3,
        enabled: false,
    },
    NativeProfile {
        version: "4.2.0.12",
        sha256: "2b3677c3f932daaa4ef84c8b2acd0aee12cbe5a779ff9e740c370761e5c291ce",
        layout: 4,
        enabled: false,
    },
    NativeProfile {
        version: "4.2.0.52",
        sha256: "d733489eea0b62af77e1aa61ba645cd941583d2a64fb2d7572da818d285f8d84",
        layout: 5,
        enabled: true,
    },
    NativeProfile {
        version: "4.1.13.7",
        sha256: "f4c4118d85ee743a0832c301288edf77b041e02f84b73d7905ffe10090206d01",
        layout: 11,
        enabled: true,
    },
    NativeProfile {
        version: "4.1.15.9",
        sha256: "1b599c66ada05ab12f9850cffe9d44314abb0440fde3d74623ada1c71dc94508",
        layout: 13,
        enabled: true,
    },
];

/// Each feature is enabled independently for an exact verified core image.
pub fn settings_mask(version: &str, architecture: &str, sha256: &str) -> u32 {
    if architecture == "x86_64"
        && PROFILES.iter().any(|p| {
            p.version == version
                && p.sha256 == sha256
                && (p.layout == 5 || p.sha256 == PROFILES[0].sha256)
        })
    {
        7
    } else {
        0
    }
}

pub fn version_status(version: &str, architecture: &str) -> (&'static str, String) {
    if architecture == "x86_64" && PROFILES.iter().any(|p| p.version == version && p.enabled) {
        return (
            "native_check_required",
            format!(
                "已找到微信 {version} 的适配档案；连接时自动核验实际加载的 DLL 指纹、入口和登录身份。"
            ),
        );
    }
    if architecture == "x86_64" && PROFILES.iter().any(|p| p.version == version) {
        return (
            "adapter_pending",
            format!(
                "已自动识别微信 {version}；此版本的完整原生适配尚未通过验证，当前不能原生收发。"
            ),
        );
    }
    (
        "unsupported",
        format!("已读取微信 {version} / {architecture}；尚无此精确构建的原生适配档案。"),
    )
}

pub fn select(
    version: &str,
    architecture: &str,
    sha256: &str,
) -> AppResult<&'static NativeProfile> {
    if architecture == "x86_64" {
        if let Some(profile) = PROFILES
            .iter()
            .find(|p| p.version == version && p.sha256 == sha256)
        {
            if profile.enabled {
                return Ok(profile);
            }
            return Err(AppError::new(
                "NATIVE_ADAPTER_PENDING",
                &format!(
                    "已自动识别微信 {version}（适配档案 {}）；完整原生地址和数据结构尚未验证，暂不能连接收发。现有微信可继续保持登录。",
                    profile.layout
                ),
            ));
        }
    }
    Err(AppError::new(
        "NATIVE_BUILD_UNSUPPORTED",
        &format!(
            "微信 {version} / {architecture} 的实际 DLL 指纹没有匹配到可用适配；未连接原生收发。"
        ),
    ))
}

#[cfg(windows)]
pub(crate) fn check_process(pid: u32) -> AppResult<&'static NativeProfile> {
    use sha2::{Digest, Sha256};
    use std::{
        fs::File,
        io::Read,
        mem::{size_of, zeroed},
        os::windows::ffi::OsStringExt,
        path::PathBuf,
    };
    use windows_sys::Win32::{
        Foundation::{CloseHandle, INVALID_HANDLE_VALUE},
        System::Diagnostics::ToolHelp::*,
    };
    // Inspect the module actually loaded in this process, rather than assuming
    // that the EXE version or the newest installed subdirectory owns its layout.
    let snapshot =
        unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPMODULE | TH32CS_SNAPMODULE32, pid) };
    if snapshot == INVALID_HANDLE_VALUE {
        return Err(AppError::new(
            "NATIVE_CORE_UNAVAILABLE",
            "无法检查微信进程实际加载的模块",
        ));
    }
    let path = unsafe {
        let mut entry: MODULEENTRY32W = zeroed();
        entry.dwSize = size_of::<MODULEENTRY32W>() as u32;
        let mut more = Module32FirstW(snapshot, &mut entry);
        let mut found = None;
        while more != 0 {
            let n = entry
                .szModule
                .iter()
                .position(|c| *c == 0)
                .unwrap_or(entry.szModule.len());
            if String::from_utf16_lossy(&entry.szModule[..n]).eq_ignore_ascii_case("Weixin.dll") {
                let n = entry
                    .szExePath
                    .iter()
                    .position(|c| *c == 0)
                    .unwrap_or(entry.szExePath.len());
                found = Some(PathBuf::from(std::ffi::OsString::from_wide(
                    &entry.szExePath[..n],
                )));
                break;
            }
            more = Module32NextW(snapshot, &mut entry);
        }
        CloseHandle(snapshot);
        found
    }
    .ok_or_else(|| AppError::new("NATIVE_CORE_UNAVAILABLE", "所选进程尚未加载微信核心模块"))?;
    let architecture = crate::discovery::architecture(&path)?;
    let (version, _) = crate::discovery::file_version(&path)?;
    let mut file = File::open(&path)
        .map_err(|_| AppError::new("NATIVE_CORE_UNAVAILABLE", "无法读取当前微信核心模块"))?;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 65536];
    let mut total = 0;
    loop {
        let n = file.read(&mut buffer).map_err(|_| {
            AppError::new("NATIVE_CORE_UNAVAILABLE", "无法完成微信核心模块指纹核验")
        })?;
        if n == 0 {
            break;
        }
        total += n;
        if total > 256 * 1024 * 1024 {
            return Err(AppError::new(
                "NATIVE_BUILD_UNSUPPORTED",
                "微信核心模块超出适配档案范围",
            ));
        }
        hash.update(&buffer[..n]);
    }
    let profile = select(&version, architecture, &hex::encode(hash.finalize()))?;
    // The host rechecks the held disk image and live entries before attaching.
    Ok(profile)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_builds_expose_only_their_verified_features() {
        let current = &PROFILES[5];
        for feature in [
            NativeFeature::Messaging,
            NativeFeature::Quotes,
            NativeFeature::Mentions,
            NativeFeature::MomentsPublish,
            NativeFeature::MomentsActions,
            NativeFeature::MomentsReply,
            NativeFeature::ArticleHistory,
            NativeFeature::FavoriteDownload,
        ] {
            assert!(current.require(feature).is_ok());
            assert!(PROFILES[0].require(feature).is_ok());
            assert!(PROFILES[1].require(feature).is_ok());
        }
        for pending in PROFILES.iter().filter(|p| !p.enabled) {
            assert!(pending.capabilities().iter().all(|c| matches!(
                c.availability,
                werc_domain::capability::Availability::Blocked
            )));
        }
        let caps = current.capabilities();
        assert!(caps.iter().all(|c| !c.requires_foreground));
        assert_eq!(caps.len(), 8);
        assert!(caps.iter().all(|c| matches!(
            c.availability,
            werc_domain::capability::Availability::Ready
        )));
    }
    #[test]
    fn settings_are_selected_per_exact_build_independently_of_message_support() {
        for p in PROFILES {
            assert_eq!(
                settings_mask(p.version, "x86_64", p.sha256),
                if p.layout == 5 || p.sha256 == PROFILES[0].sha256 {
                    7
                } else {
                    0
                }
            );
        }
        assert_eq!(
            settings_mask(PROFILES[0].version, "x86", PROFILES[0].sha256),
            0
        );
        assert_eq!(settings_mask("4.2.0.52", "x86_64", PROFILES[0].sha256), 0);
        assert_eq!(settings_mask(PROFILES[0].version, "x86_64", "unknown"), 0);
    }
    #[test]
    fn selects_both_existing_variants_without_a_manual_version_switch() {
        for profile in PROFILES.iter().filter(|p| p.enabled) {
            assert_eq!(
                select(profile.version, "x86_64", profile.sha256)
                    .unwrap()
                    .layout,
                profile.layout
            );
            assert!(
                include_str!("../../../native/message-bridge/compatibility.h")
                    .contains(profile.sha256)
            );
        }
    }
    #[test]
    fn known_tables_and_current_server_are_not_falsely_enabled() {
        for profile in PROFILES.iter().filter(|p| !p.enabled) {
            assert_eq!(
                select(profile.version, "x86_64", profile.sha256)
                    .err()
                    .unwrap()
                    .code,
                "NATIVE_ADAPTER_PENDING"
            );
            assert_eq!(
                version_status(profile.version, "x86_64").0,
                "adapter_pending"
            );
        }
    }
    #[test]
    fn version_architecture_and_fingerprint_must_all_match() {
        let profile = &PROFILES[0];
        for (version, arch, hash) in [
            (profile.version, "x86", profile.sha256),
            ("4.2.0.52", "x86_64", profile.sha256),
            ("4.1.15.130", "x86_64", profile.sha256),
            (profile.version, "x86_64", "modified"),
        ] {
            assert_eq!(
                select(version, arch, hash).err().unwrap().code,
                "NATIVE_BUILD_UNSUPPORTED"
            );
        }
        assert_eq!(
            version_status("4.1.15.13", "x86_64").0,
            "native_check_required"
        );
        assert_eq!(version_status("0.0.0.1", "x86_64").0, "unsupported");
    }
}



