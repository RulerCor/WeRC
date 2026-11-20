use werc_domain::{AppError, AppResult};
/// Resolve the explicitly supplied PID through normal installed-client discovery.
/// Never assume a drive letter, user name or installation directory.
pub fn executable(pid: u32) -> AppResult<String> {
    let mut candidates = werc_wechat_win::discovery::discover()?
        .installations
        .into_iter()
        .filter(|i| i.process_ids.contains(&pid));
    let selected = candidates.next().ok_or_else(|| {
        AppError::new(
            "CLIENT_SELECTION_REQUIRED",
            "指定进程不在当前客户端检测结果中",
        )
    })?;
    if candidates.next().is_some() {
        return Err(AppError::new(
            "CLIENT_SELECTION_REQUIRED",
            "指定进程匹配多个安装",
        ));
    }
    Ok(selected.executable_path)
}
