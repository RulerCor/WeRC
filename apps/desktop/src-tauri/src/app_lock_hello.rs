use super::*;

#[cfg(windows)]
struct Apartment;
#[cfg(windows)]
impl Apartment {
    fn initialize() -> windows::core::Result<Self> {
        unsafe {
            windows::Win32::System::WinRT::RoInitialize(
                windows::Win32::System::WinRT::RO_INIT_MULTITHREADED,
            )?;
        }
        Ok(Self)
    }
}
#[cfg(windows)]
impl Drop for Apartment {
    fn drop(&mut self) {
        unsafe {
            windows::Win32::System::WinRT::RoUninitialize();
        }
    }
}

pub(super) fn hello_availability() -> &'static str {
    #[cfg(windows)]
    {
        use windows::Security::Credentials::UI::{
            UserConsentVerifier as V, UserConsentVerifierAvailability as A,
        };
        let Ok(_apartment) = Apartment::initialize() else {
            return "unavailable";
        };
        match V::CheckAvailabilityAsync().and_then(|op| op.join()) {
            Ok(A::Available) => "available",
            Ok(A::NotConfiguredForUser) => "not_configured",
            Ok(A::DeviceNotPresent) => "no_device",
            Ok(A::DisabledByPolicy) => "policy_disabled",
            Ok(A::DeviceBusy) => "busy",
            _ => "unavailable",
        }
    }
    #[cfg(not(windows))]
    {
        "unavailable"
    }
}
#[tauri::command]
pub(crate) async fn app_lock_hello_available() -> Result<String, AppError> {
    tauri::async_runtime::spawn_blocking(|| hello_availability().to_owned())
        .await
        .map_err(|_| AppError::internal())
}
#[tauri::command]
pub(crate) async fn unlock_desktop_hello(handle: tauri::AppHandle) -> Result<LockStatus, AppError> {
    #[cfg(windows)]
    {
        use windows::{
            Security::Credentials::UI::{UserConsentVerificationResult, UserConsentVerifier},
            Win32::System::WinRT::IUserConsentVerifierInterop,
        };
        let generation = {
            let state = handle.state::<AppLock>();
            let inner = state.inner.lock().map_err(|_| AppError::internal())?;
            if !inner.configuration.as_ref().is_some_and(|c| c.hello) {
                return Err(failed("HELLO_UNAVAILABLE", "未启用 Windows Hello"));
            }
            inner.generation
        };
        let hwnd = handle
            .get_webview_window("main")
            .ok_or_else(AppError::internal)?
            .hwnd()
            .map_err(|_| AppError::internal())?
            .0 as usize;
        tauri::async_runtime::spawn_blocking(move || {
            let _apartment = Apartment::initialize()
                .map_err(|_| failed("HELLO_UNAVAILABLE", "Windows Hello 当前不可用"))?;
            let interop: IUserConsentVerifierInterop =
                windows::core::factory::<UserConsentVerifier, _>()
                    .map_err(|_| failed("HELLO_UNAVAILABLE", "此系统不支持窗口内 Windows Hello"))?;
            let op: windows_future::IAsyncOperation<UserConsentVerificationResult> = unsafe {
                interop.RequestVerificationForWindowAsync(
                    windows::Win32::Foundation::HWND(hwnd as *mut _),
                    &windows::core::HSTRING::from("解锁 WeRC 本机窗口"),
                )
            }
            .map_err(|_| failed("HELLO_UNAVAILABLE", "无法打开 Windows Hello"))?;
            let result = op
                .join()
                .map_err(|_| failed("HELLO_FAILED", "Windows Hello 验证未完成"))?;
            if result != UserConsentVerificationResult::Verified {
                return Err(failed(
                    "HELLO_NOT_VERIFIED",
                    "Windows Hello 未通过或已取消；可使用应用密码",
                ));
            }
            let state = handle.state::<AppLock>();
            let mut inner = state.inner.lock().map_err(|_| AppError::internal())?;
            if inner.generation != generation
                || !inner.configuration.as_ref().is_some_and(|c| c.hello)
            {
                return Err(failed("LOCK_CHANGED", "验证期间锁定状态已变化，请重新解锁"));
            }
            inner.locked = false;
            inner.activity = Instant::now();
            inner.generation = inner.generation.wrapping_add(1);
            Ok(AppLock::snapshot(&inner))
        })
        .await
        .map_err(|_| AppError::internal())?
    }
    #[cfg(not(windows))]
    {
        let _ = handle;
        Err(failed("HELLO_UNAVAILABLE", "Windows Hello 需要 Windows"))
    }
}
