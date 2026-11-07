//! Kill switch berbasis file + kait memori (FR-023).
//! `apex kill` membuat file; engine memeriksa tiap loop. Bila disk penuh/gagal tulis saat
//! kondisi darurat (`STATE_SAVE_FAILED`), kait atomik di memori tetap mengunci engine.
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug)]
pub struct KillSwitch {
    path: PathBuf,
    latched: Arc<AtomicBool>,
    mem_reason: Arc<Mutex<Option<String>>>,
}

impl KillSwitch {
    pub fn new(path: &Path) -> Self {
        KillSwitch {
            path: path.to_path_buf(),
            latched: Arc::new(AtomicBool::new(false)),
            mem_reason: Arc::new(Mutex::new(None)),
        }
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn is_engaged(&self) -> bool {
        self.latched.load(Ordering::SeqCst) || self.path.exists()
    }
    pub fn reason(&self) -> Option<String> {
        std::fs::read_to_string(&self.path).ok().or_else(|| {
            self.mem_reason
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone()
        })
    }
    pub fn engage(&self, reason: &str) -> std::io::Result<()> {
        self.latched.store(true, Ordering::SeqCst);
        *self.mem_reason.lock().unwrap_or_else(|e| e.into_inner()) = Some(reason.to_string());
        if let Some(p) = self.path.parent() {
            std::fs::create_dir_all(p)?;
        }
        std::fs::write(&self.path, reason)
    }
    pub fn release(&self) -> std::io::Result<()> {
        self.latched.store(false, Ordering::SeqCst);
        *self.mem_reason.lock().unwrap_or_else(|e| e.into_inner()) = None;
        if self.path.exists() {
            std::fs::remove_file(&self.path)?;
        }
        Ok(())
    }
}
