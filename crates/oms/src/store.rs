//! Persist state OMS ke JSON secara atomik dan durable:
//! serialisasi → tulis file `.json.tmp` di direktori yang sama → `File::sync_all()`
//! → cadangkan file utama lama ke `.json.bak` → `rename` → `fsync` direktori induk (Unix).
//! Saat load, bila file utama hilang atau korup namun `.json.bak` valid, state dipulihkan
//! dari `.json.bak` agar pemadaman mendadak di tengah rotasi tidak menghilangkan posisi aktif.

use serde::{de::DeserializeOwned, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("kegagalan I/O state store ({op}): {source}")]
    Io {
        op: &'static str,
        #[source]
        source: std::io::Error,
    },
    #[error("kegagalan serialisasi JSON state store: {0}")]
    Serialize(#[source] serde_json::Error),
    #[error("kegagalan deserialisasi JSON state store ({path}): {source}")]
    Deserialize {
        path: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("injeksi kegagalan simpan state (simulasi test)")]
    SimulatedFailure,
}

#[derive(Clone, Debug)]
pub struct LoadOutcome<T> {
    pub value: T,
    pub recovered_from_bak: bool,
}

#[derive(Clone, Debug)]
pub struct StateStore {
    path: Option<PathBuf>,
    fail_saves_remaining: Arc<AtomicU32>,
}

impl StateStore {
    pub fn file(path: &Path) -> Self {
        StateStore {
            path: Some(path.to_path_buf()),
            fail_saves_remaining: Arc::new(AtomicU32::new(0)),
        }
    }

    pub fn memory() -> Self {
        StateStore {
            path: None,
            fail_saves_remaining: Arc::new(AtomicU32::new(0)),
        }
    }

    /// Aktifkan injeksi kegagalan `save()` sebanyak `n` kali berturut-turut untuk pengujian ketahanan.
    pub fn set_fail_saves(&self, n: u32) {
        self.fail_saves_remaining.store(n, Ordering::SeqCst);
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    pub fn tmp_path(primary: &Path) -> PathBuf {
        primary.with_extension("json.tmp")
    }

    pub fn bak_path(primary: &Path) -> PathBuf {
        primary.with_extension("json.bak")
    }

    pub fn load<T: DeserializeOwned + Default>(&self) -> Result<T, StoreError> {
        Ok(self.load_with_recovery()?.value)
    }

    /// Muat state dari file utama; bila file utama hilang atau korup tetapi `.json.bak` tersedia
    /// dan valid, pulihkan dari `.json.bak` serta tandai `recovered_from_bak = true`.
    pub fn load_with_recovery<T: DeserializeOwned + Default>(
        &self,
    ) -> Result<LoadOutcome<T>, StoreError> {
        let Some(p) = &self.path else {
            return Ok(LoadOutcome {
                value: T::default(),
                recovered_from_bak: false,
            });
        };
        let bak = Self::bak_path(p);
        if p.exists() {
            match Self::read_and_parse::<T>(p) {
                Ok(value) => {
                    return Ok(LoadOutcome {
                        value,
                        recovered_from_bak: false,
                    })
                }
                Err(primary_err) => {
                    if bak.exists() {
                        if let Ok(value) = Self::read_and_parse::<T>(&bak) {
                            return Ok(LoadOutcome {
                                value,
                                recovered_from_bak: true,
                            });
                        }
                    }
                    return Err(primary_err);
                }
            }
        }
        if bak.exists() {
            let value = Self::read_and_parse::<T>(&bak)?;
            return Ok(LoadOutcome {
                value,
                recovered_from_bak: true,
            });
        }
        Ok(LoadOutcome {
            value: T::default(),
            recovered_from_bak: false,
        })
    }

    fn read_and_parse<T: DeserializeOwned>(path: &Path) -> Result<T, StoreError> {
        let raw = std::fs::read_to_string(path).map_err(|source| StoreError::Io {
            op: "read_to_string",
            source,
        })?;
        serde_json::from_str(&raw).map_err(|source| StoreError::Deserialize {
            path: path.display().to_string(),
            source,
        })
    }

    fn take_fail_token(&self) -> bool {
        let mut cur = self.fail_saves_remaining.load(Ordering::SeqCst);
        while cur > 0 {
            match self.fail_saves_remaining.compare_exchange_weak(
                cur,
                cur - 1,
                Ordering::SeqCst,
                Ordering::SeqCst,
            ) {
                Ok(_) => return true,
                Err(next) => cur = next,
            }
        }
        false
    }

    pub fn save<T: Serialize>(&self, v: &T) -> Result<(), StoreError> {
        if self.take_fail_token() {
            return Err(StoreError::SimulatedFailure);
        }
        let Some(p) = &self.path else {
            return Ok(());
        };
        if let Some(d) = p.parent() {
            std::fs::create_dir_all(d).map_err(|source| StoreError::Io {
                op: "create_dir_all",
                source,
            })?;
        }
        let bytes = serde_json::to_vec_pretty(v).map_err(StoreError::Serialize)?;
        let tmp = Self::tmp_path(p);
        {
            let mut f = std::fs::OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(&tmp)
                .map_err(|source| StoreError::Io {
                    op: "open_tmp",
                    source,
                })?;
            f.write_all(&bytes).map_err(|source| StoreError::Io {
                op: "write_tmp",
                source,
            })?;
            f.sync_all().map_err(|source| StoreError::Io {
                op: "sync_tmp",
                source,
            })?;
        }
        if p.exists() {
            let bak = Self::bak_path(p);
            std::fs::copy(p, &bak).map_err(|source| StoreError::Io {
                op: "copy_bak",
                source,
            })?;
        }
        std::fs::rename(&tmp, p).map_err(|source| StoreError::Io {
            op: "rename_primary",
            source,
        })?;
        Self::sync_parent_dir(p)?;
        Ok(())
    }

    #[cfg(unix)]
    fn sync_parent_dir(p: &Path) -> Result<(), StoreError> {
        if let Some(parent) = p.parent() {
            let dir = std::fs::File::open(parent).map_err(|source| StoreError::Io {
                op: "open_parent_dir",
                source,
            })?;
            dir.sync_all().map_err(|source| StoreError::Io {
                op: "sync_parent_dir",
                source,
            })?;
        }
        Ok(())
    }

    #[cfg(not(unix))]
    fn sync_parent_dir(_p: &Path) -> Result<(), StoreError> {
        Ok(())
    }
}
