//! Shared local device selection for setup and the isolated recognition process.
use std::process::Command;
use werc_domain::{AppError, AppResult};

#[derive(serde::Serialize)]
pub struct Device {
    pub name: String,
    pub driver: String,
    pub compute_capability: String,
    pub compatible: bool,
    #[serde(skip)]
    uuid: String,
}

fn version(value: &str) -> Option<(u32, u32)> {
    let (major, minor) = value.trim().split_once('.')?;
    Some((major.parse().ok()?, minor.parse().ok()?))
}

pub fn compatible(driver: &str, compute: &str, minimum: &str) -> bool {
    matches!((version(driver), version(minimum), version(compute)),
        (Some(d), Some(m), Some(c)) if d >= m && c >= (7, 5))
}

fn parse(output: &str, minimum: &str) -> Vec<Device> {
    output.lines().take(16).filter_map(|line| {
        let fields: Vec<_> = line.split(',').map(str::trim).collect();
        if fields.len() != 4 { return None; }
        let valid_uuid = fields[3].strip_prefix("GPU-").is_some_and(|s|
            s.len() == 36 && s.bytes().enumerate().all(|(i, c)|
                if matches!(i, 8 | 13 | 18 | 23) { c == b'-' } else { c.is_ascii_hexdigit() }));
        Some(Device {
            name: fields[0].into(), driver: fields[1].into(),
            compute_capability: fields[2].into(), uuid: fields[3].into(),
            compatible: valid_uuid && compatible(fields[1], fields[2], minimum),
        })
    }).collect()
}

pub fn detect(minimum: &str) -> AppResult<Vec<Device>> {
    if !cfg!(all(windows, target_arch = "x86_64")) { return Ok(vec![]); }
    #[cfg(windows)]
    {
        use std::{os::windows::process::CommandExt, process::Stdio, time::{Duration, Instant}};
        let system = std::env::var_os("SystemRoot").ok_or_else(AppError::internal)?;
        let executable = std::path::PathBuf::from(system).join("System32/nvidia-smi.exe");
        if !executable.is_file() { return Ok(vec![]); }
        let mut child = Command::new(executable)
            .args(["--query-gpu=name,driver_version,compute_cap,uuid", "--format=csv,noheader,nounits"])
            .stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null())
            .creation_flags(0x08000000).spawn()
            .map_err(|_| AppError::new("GPU_DETECT_FAILED", "无法启动本机显卡检测"))?;
        let start = Instant::now();
        loop {
            match child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) if start.elapsed() < Duration::from_secs(15) => std::thread::sleep(Duration::from_millis(50)),
                _ => {
                    let _ = child.kill(); let _ = child.wait();
                    return Err(AppError::new("GPU_DETECT_FAILED", "显卡检测超时或驱动不可用；可继续使用 CPU"));
                }
            }
        }
        let output = child.wait_with_output().map_err(|_| AppError::internal())?;
        if !output.status.success() {
            return Err(AppError::new("GPU_DETECT_FAILED", "NVIDIA 驱动未正常响应；请检查驱动，或继续使用 CPU"));
        }
        return Ok(parse(&String::from_utf8_lossy(&output.stdout), minimum));
    }
    #[cfg(not(windows))]
    Ok(vec![])
}

pub fn configure_command(command: &mut Command) -> AppResult<()> {
    let plan: serde_json::Value = serde_json::from_str(include_str!("../../../native/asr/CUDA.json"))
        .map_err(|_| AppError::internal())?;
    let minimum = plan["minimum_windows_driver"].as_str().ok_or_else(AppError::internal)?;
    let device = detect(minimum)?.into_iter().find(|d| d.compatible)
        .ok_or_else(|| AppError::new("GPU_UNSUPPORTED", "当前电脑没有符合此 CUDA 方案的设备；可在设置中选择 CPU"))?;
    // Only the child sees the selected UUID, as its logical CUDA device 0.
    command.env("CUDA_VISIBLE_DEVICES", device.uuid);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn multi_gpu_selection_skips_unsupported_first_device() {
        let devices = parse("Old, 581.15, 6.1, GPU-00000000-0000-0000-0000-000000000001\nNew, 581.15, 12.0, GPU-00000000-0000-0000-0000-000000000002", "570.65");
        assert!(!devices[0].compatible);
        assert_eq!(devices.iter().find(|d| d.compatible).unwrap().name, "New");
        assert!(!compatible("560.94", "8.6", "570.65"));
        assert!(!compatible("N/A", "N/A", "570.65"));
        assert!(!parse("Broken, 581.15, 12.0, unknown", "570.65")[0].compatible);
    }
}
