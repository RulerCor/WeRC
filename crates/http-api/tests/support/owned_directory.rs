//! Declared last in fixtures so repositories/providers are dropped before cleanup.
#[derive(Clone)]
pub struct OwnedDirectory(pub std::path::PathBuf, #[allow(dead_code)] std::sync::Arc<Cleanup>);
struct Cleanup(std::path::PathBuf);
impl OwnedDirectory {
    pub fn new(prefix: &str) -> Self {
        let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().parent().unwrap().join("target/tmp");
        std::fs::create_dir_all(&base).unwrap();
        let directory = base.join(format!("{prefix}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&directory).unwrap();
        Self(directory.clone(),std::sync::Arc::new(Cleanup(directory)))
    }
}
impl Drop for Cleanup {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
