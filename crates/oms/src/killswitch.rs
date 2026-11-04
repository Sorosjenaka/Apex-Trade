//! Kill switch berbasis file (FR-023). `apex kill` membuat file; engine memeriksa tiap loop.
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct KillSwitch {
    path: PathBuf,
}

impl KillSwitch {
    pub fn new(path: &Path) -> Self {
        KillSwitch { path: path.to_path_buf() }
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn is_engaged(&self) -> bool {
        self.path.exists()
    }
    pub fn reason(&self) -> Option<String> {
        std::fs::read_to_string(&self.path).ok()
    }
    pub fn engage(&self, reason: &str) -> std::io::Result<()> {
        if let Some(p) = self.path.parent() {
            std::fs::create_dir_all(p)?;
        }
        std::fs::write(&self.path, reason)
    }
    pub fn release(&self) -> std::io::Result<()> {
        if self.path.exists() {
            std::fs::remove_file(&self.path)?;
        }
        Ok(())
    }
}
