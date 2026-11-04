//! Persist state OMS ke JSON (tulis atomik: tmp → rename).
use serde::{de::DeserializeOwned, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct StateStore {
    path: Option<PathBuf>,
}

impl StateStore {
    pub fn file(path: &Path) -> Self {
        StateStore { path: Some(path.to_path_buf()) }
    }
    pub fn memory() -> Self {
        StateStore { path: None }
    }
    pub fn load<T: DeserializeOwned + Default>(&self) -> anyhow::Result<T> {
        match &self.path {
            Some(p) if p.exists() => Ok(serde_json::from_str(&std::fs::read_to_string(p)?)?),
            _ => Ok(T::default()),
        }
    }
    pub fn save<T: Serialize>(&self, v: &T) -> anyhow::Result<()> {
        if let Some(p) = &self.path {
            if let Some(d) = p.parent() {
                std::fs::create_dir_all(d)?;
            }
            let tmp = p.with_extension("json.tmp");
            std::fs::write(&tmp, serde_json::to_vec_pretty(v)?)?;
            std::fs::rename(&tmp, p)?;
        }
        Ok(())
    }
}
