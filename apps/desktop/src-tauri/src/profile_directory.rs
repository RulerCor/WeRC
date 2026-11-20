use std::{ffi::OsString, path::PathBuf};

/// An explicit profile is useful for portable profiles and isolated diagnostics.
/// Relative paths are resolved once against the caller's working directory.
pub fn selected(args: impl IntoIterator<Item = OsString>) -> Result<Option<PathBuf>, String> {
    let mut args = args.into_iter();
    let mut selected = None;
    while let Some(arg) = args.next() {
        if arg != "--data-directory" { continue; }
        if selected.is_some() { return Err("资料目录参数重复".into()); }
        let value = args.next().filter(|v| !v.is_empty())
            .ok_or("--data-directory 后需要填写资料目录")?;
        if value.to_string_lossy().starts_with("--") { return Err("资料目录参数无效".into()); }
        let path = PathBuf::from(value);
        selected = Some(if path.is_absolute() { path } else {
            std::env::current_dir().map_err(|_| "无法读取工作目录")?.join(path)
        });
    }
    Ok(selected)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn parse(values: &[&str]) -> Result<Option<PathBuf>, String> {
        selected(values.iter().map(OsString::from))
    }
    #[test]
    fn explicit_profile_requires_one_nonempty_path() {
        assert_eq!(parse(&[]).unwrap(), None);
        assert!(parse(&["--data-directory"]).is_err());
        assert!(parse(&["--data-directory", ""]).is_err());
        assert!(parse(&["--data-directory", "a", "--data-directory", "b"]).is_err());
        assert_eq!(parse(&["--data-directory", "资料 目录"]).unwrap().unwrap(),
            std::env::current_dir().unwrap().join("资料 目录"));
    }
}
