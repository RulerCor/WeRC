//! Local desktop access only. Locking does not stop authorized Bot clients.
use super::*;
use serde::Deserialize;
use std::{
    io::Write,
    path::PathBuf,
    time::{Instant, SystemTime, UNIX_EPOCH},
};
use zeroize::Zeroizing;
const CONTEXT: &str = "WeRC/desktop-lock/v1";
const ROUNDS: u32 = 600_000;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Configuration {
    version: u32,
    salt: [u8; 16],
    password: [u8; 32],
    recovery: [u8; 32],
    hello: bool,
    idle_minutes: u32,
    failures: u32,
    blocked_until: u64,
}
struct Inner {
    configuration: Option<Configuration>,
    locked: bool,
    activity: Instant,
    generation: u64,
}
pub(super) struct AppLock {
    directory: PathBuf,
    inner: Mutex<Inner>,
}
#[derive(Serialize)]
pub(super) struct LockStatus {
    configured: bool,
    locked: bool,
    hello_enabled: bool,
    idle_minutes: u32,
    retry_after_seconds: u64,
}
#[derive(Serialize)]
pub(super) struct Configured {
    status: LockStatus,
    recovery_code: String,
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn failed(code: &str, message: &str) -> AppError {
    AppError::new(code, message)
}
fn derive(value: &str, salt: &[u8; 16]) -> Zeroizing<[u8; 32]> {
    let mut hash = Zeroizing::new([0; 32]);
    pbkdf2::pbkdf2_hmac::<sha2::Sha256>(value.as_bytes(), salt, ROUNDS, hash.as_mut());
    hash
}
fn equal(a: &[u8; 32], b: &[u8; 32]) -> bool {
    a.iter().zip(b).fold(0u8, |diff, (a, b)| diff | (a ^ b)) == 0
}
impl AppLock {
    pub fn open(directory: PathBuf) -> Result<Self, AppError> {
        let configuration = match std::fs::read(directory.join("desktop-lock.bin")) {
            Ok(bytes) => {
                let plain = Zeroizing::new(werc_wechat_win::keys::unprotect(CONTEXT, &bytes)?);
                let value: Configuration = serde_json::from_slice(&plain)
                    .map_err(|_| failed("LOCK_CONFIG_INVALID", "应用锁配置无法读取"))?;
                if value.version != 1 || value.idle_minutes > 1440 {
                    return Err(failed(
                        "LOCK_CONFIG_INVALID",
                        "应用锁配置版本或空闲时长无效",
                    ));
                }
                Some(value)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(_) => return Err(failed("LOCK_CONFIG_IO", "无法读取应用锁配置")),
        };
        let locked = configuration.is_some();
        Ok(Self {
            directory,
            inner: Mutex::new(Inner {
                configuration,
                locked,
                activity: Instant::now(),
                generation: 0,
            }),
        })
    }
    fn save(&self, value: &Configuration) -> Result<(), AppError> {
        let plain = Zeroizing::new(serde_json::to_vec(value).map_err(|_| AppError::internal())?);
        let bytes = werc_wechat_win::keys::protect(CONTEXT, &plain)?;
        let temporary = self
            .directory
            .join(format!("desktop-lock-{}.tmp", uuid::Uuid::new_v4()));
        let result = (|| {
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)
                .map_err(|_| failed("LOCK_CONFIG_IO", "无法保存应用锁"))?;
            file.write_all(&bytes)
                .and_then(|_| file.sync_all())
                .map_err(|_| failed("LOCK_CONFIG_IO", "应用锁写盘失败"))?;
            drop(file);
            preferences::replace(&temporary, &self.directory.join("desktop-lock.bin"))
        })();
        let _ = std::fs::remove_file(&temporary);
        result
    }
    fn expire(inner: &mut Inner) {
        if !inner.locked
            && inner.configuration.as_ref().is_some_and(|c| {
                c.idle_minutes > 0
                    && inner.activity.elapsed().as_secs() >= u64::from(c.idle_minutes) * 60
            })
        {
            inner.locked = true;
            inner.generation = inner.generation.wrapping_add(1);
        }
    }
    fn snapshot(inner: &Inner) -> LockStatus {
        LockStatus {
            configured: inner.configuration.is_some(),
            locked: inner.locked,
            hello_enabled: inner.configuration.as_ref().is_some_and(|c| c.hello),
            idle_minutes: inner.configuration.as_ref().map_or(15, |c| c.idle_minutes),
            retry_after_seconds: inner
                .configuration
                .as_ref()
                .map_or(0, |c| c.blocked_until.saturating_sub(now())),
        }
    }
    pub fn status(&self) -> Result<LockStatus, AppError> {
        let mut inner = self
            .inner
            .try_lock()
            .map_err(|_| failed("LOCK_BUSY", "正在验证应用密码"))?;
        Self::expire(&mut inner);
        Ok(Self::snapshot(&inner))
    }
    pub fn require_open(&self) -> Result<(), AppError> {
        if self.status()?.locked {
            Err(failed("APP_LOCKED", "应用已锁定，请先解锁"))
        } else {
            Ok(())
        }
    }
    fn verify(&self, inner: &mut Inner, secret: &str, recovery: bool) -> Result<(), AppError> {
        let c = inner
            .configuration
            .as_ref()
            .ok_or_else(|| failed("LOCK_NOT_CONFIGURED", "尚未设置应用锁"))?;
        if c.blocked_until > now() {
            return Err(failed("LOCK_WAIT", "尝试次数较多，请等待倒计时结束"));
        }
        if secret.len() > 1024 {
            return Err(failed("INVALID_ARGUMENT", "密码或恢复码过长"));
        }
        let actual = derive(secret, &c.salt);
        if !equal(&actual, if recovery { &c.recovery } else { &c.password }) {
            let mut updated = c.clone();
            updated.failures = updated.failures.saturating_add(1);
            updated.blocked_until = now()
                + if updated.failures < 3 {
                    0
                } else {
                    (5u64 << updated.failures.saturating_sub(3).min(6)).min(300)
                };
            self.save(&updated)?;
            inner.configuration = Some(updated);
            return Err(failed("LOCK_CREDENTIAL_INVALID", "密码或恢复码不正确"));
        }
        let mut updated = c.clone();
        updated.failures = 0;
        updated.blocked_until = 0;
        self.save(&updated)?;
        inner.configuration = Some(updated);
        Ok(())
    }
    fn unlock(&self, password: &str) -> Result<LockStatus, AppError> {
        let mut inner = self.inner.lock().map_err(|_| AppError::internal())?;
        self.verify(&mut inner, password, false)?;
        inner.locked = false;
        inner.activity = Instant::now();
        inner.generation = inner.generation.wrapping_add(1);
        Ok(Self::snapshot(&inner))
    }
    fn configure(
        &self,
        current: &str,
        password: &str,
        hello: bool,
        idle: u32,
    ) -> Result<Configured, AppError> {
        if password.chars().count() < 8 || password.len() > 1024 || idle > 1440 {
            return Err(failed(
                "INVALID_ARGUMENT",
                "密码至少8个字符，空闲锁定时间最多1440分钟",
            ));
        }
        let mut inner = self.inner.lock().map_err(|_| AppError::internal())?;
        if inner.configuration.is_some() {
            self.verify(&mut inner, current, false)?;
        }
        let mut salt = [0; 16];
        let mut recovery = [0; 24];
        getrandom::fill(&mut salt)
            .and_then(|_| getrandom::fill(&mut recovery))
            .map_err(|_| AppError::internal())?;
        let recovery_code = hex::encode(recovery);
        let configuration = Configuration {
            version: 1,
            salt,
            password: *derive(password, &salt),
            recovery: *derive(&recovery_code, &salt),
            hello,
            idle_minutes: idle,
            failures: 0,
            blocked_until: 0,
        };
        self.save(&configuration)?;
        inner.configuration = Some(configuration);
        inner.locked = false;
        inner.activity = Instant::now();
        inner.generation = inner.generation.wrapping_add(1);
        Ok(Configured {
            status: Self::snapshot(&inner),
            recovery_code,
        })
    }
    fn remove(&self, secret: &str, recovery: bool) -> Result<LockStatus, AppError> {
        let mut inner = self.inner.lock().map_err(|_| AppError::internal())?;
        self.verify(&mut inner, secret, recovery)?;
        std::fs::remove_file(self.directory.join("desktop-lock.bin"))
            .map_err(|_| failed("LOCK_CONFIG_IO", "无法移除应用锁配置"))?;
        inner.configuration = None;
        inner.locked = false;
        inner.generation = inner.generation.wrapping_add(1);
        Ok(Self::snapshot(&inner))
    }
}
#[tauri::command]
pub(super) fn app_lock_status(state: State<'_, AppLock>) -> Result<LockStatus, AppError> {
    state.status()
}
#[tauri::command]
pub(super) fn app_lock_activity(state: State<'_, AppLock>) -> Result<(), AppError> {
    let mut inner = state.inner.lock().map_err(|_| AppError::internal())?;
    AppLock::expire(&mut inner);
    if !inner.locked {
        inner.activity = Instant::now();
    }
    Ok(())
}
#[tauri::command]
pub(super) fn lock_desktop(
    handle: tauri::AppHandle,
    state: State<'_, AppLock>,
) -> Result<LockStatus, AppError> {
    use tauri::Emitter;
    let status = {
        let mut inner = state.inner.lock().map_err(|_| AppError::internal())?;
        if inner.configuration.is_none() {
            return Err(failed("LOCK_NOT_CONFIGURED", "先在设置中配置应用锁"));
        }
        inner.locked = true;
        inner.generation = inner.generation.wrapping_add(1);
        AppLock::snapshot(&inner)
    };
    let _ = handle.emit("werc-lock-changed", ());
    Ok(status)
}
#[tauri::command]
pub(super) async fn unlock_desktop(
    handle: tauri::AppHandle,
    password: String,
) -> Result<LockStatus, AppError> {
    let password = Zeroizing::new(password);
    tauri::async_runtime::spawn_blocking(move || handle.state::<AppLock>().unlock(&password))
        .await
        .map_err(|_| AppError::internal())?
}
#[tauri::command]
pub(super) async fn configure_app_lock(
    handle: tauri::AppHandle,
    current_password: String,
    password: String,
    hello: bool,
    idle_minutes: u32,
) -> Result<Configured, AppError> {
    let current = Zeroizing::new(current_password);
    let password = Zeroizing::new(password);
    tauri::async_runtime::spawn_blocking(move || {
        if hello && hello_availability() != "available" {
            return Err(failed(
                "HELLO_UNAVAILABLE",
                "此电脑尚未配置 Windows Hello；可先使用应用密码",
            ));
        }
        handle
            .state::<AppLock>()
            .configure(&current, &password, hello, idle_minutes)
    })
    .await
    .map_err(|_| AppError::internal())?
}
#[tauri::command]
pub(super) async fn remove_app_lock(
    handle: tauri::AppHandle,
    password: String,
) -> Result<LockStatus, AppError> {
    let password = Zeroizing::new(password);
    tauri::async_runtime::spawn_blocking(move || handle.state::<AppLock>().remove(&password, false))
        .await
        .map_err(|_| AppError::internal())?
}
#[tauri::command]
pub(super) async fn recover_app_lock(
    handle: tauri::AppHandle,
    recovery_code: String,
) -> Result<LockStatus, AppError> {
    let code = Zeroizing::new(recovery_code.trim().to_ascii_lowercase());
    tauri::async_runtime::spawn_blocking(move || handle.state::<AppLock>().remove(&code, true))
        .await
        .map_err(|_| AppError::internal())?
}
#[path = "app_lock_hello.rs"]
pub(super) mod hello;
use hello::hello_availability;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn password_restart_idle_delay_and_recovery_keep_user_data() {
        let directory =
            std::env::temp_dir().join(format!("werc-lock-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&directory).unwrap();
        std::fs::write(
            directory.join("state.sqlite3"),
            b"owned fixture, not a user store",
        )
        .unwrap();
        let lock = AppLock::open(directory.clone()).unwrap();
        assert!(!lock.status().unwrap().configured);
        let configured = lock
            .configure("", "owned-password-自动化测试", false, 1)
            .unwrap();
        assert!(!configured.status.locked);
        let raw = std::fs::read(directory.join("desktop-lock.bin")).unwrap();
        assert!(
            !raw.windows(configured.recovery_code.len())
                .any(|s| s == configured.recovery_code.as_bytes())
        );
        drop(lock);
        let lock = AppLock::open(directory.clone()).unwrap();
        assert!(lock.require_open().is_err());
        for _ in 0..3 {
            assert!(lock.unlock("wrong").is_err());
        }
        assert!(lock.status().unwrap().retry_after_seconds > 0);
        drop(lock);
        let lock = AppLock::open(directory.clone()).unwrap();
        assert!(lock.status().unwrap().retry_after_seconds > 0);
        {
            let mut inner = lock.inner.lock().unwrap();
            inner.configuration.as_mut().unwrap().blocked_until = 0;
        }
        assert!(!lock.unlock("owned-password-自动化测试").unwrap().locked);
        {
            let mut inner = lock.inner.lock().unwrap();
            inner.activity = Instant::now() - std::time::Duration::from_secs(61);
        }
        assert!(lock.require_open().is_err());
        assert!(
            !lock
                .remove(&configured.recovery_code, true)
                .unwrap()
                .configured
        );
        assert_eq!(
            std::fs::read(directory.join("state.sqlite3")).unwrap(),
            b"owned fixture, not a user store"
        );
        assert!(
            !AppLock::open(directory.clone())
                .unwrap()
                .status()
                .unwrap()
                .locked
        );
        std::fs::remove_dir_all(directory).unwrap();
    }
}
