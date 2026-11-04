//! Audit log JSONL (FR-025). Field yang namanya mengandung key/secret/signature/token diredaksi.
use crate::types::Ms;
use serde_json::{Map, Value};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub struct AuditLog {
    path: Option<PathBuf>,
    mem: Mutex<Vec<Value>>,
}

pub fn is_sensitive(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    ["key", "secret", "signature", "token", "password"].iter().any(|s| n.contains(s))
}

fn redact(v: &mut Value) {
    match v {
        Value::Object(m) => {
            for (k, val) in m.iter_mut() {
                if is_sensitive(k) {
                    *val = Value::String("***".into());
                } else {
                    redact(val);
                }
            }
        }
        Value::Array(a) => a.iter_mut().for_each(redact),
        _ => {}
    }
}

impl AuditLog {
    pub fn to_file(path: &Path) -> std::io::Result<Self> {
        if let Some(p) = path.parent() {
            std::fs::create_dir_all(p)?;
        }
        Ok(AuditLog { path: Some(path.to_path_buf()), mem: Mutex::new(Vec::new()) })
    }
    pub fn in_memory() -> Self {
        AuditLog { path: None, mem: Mutex::new(Vec::new()) }
    }

    pub fn log(&self, ts: Ms, event: &str, fields: Value) {
        let mut obj = Map::new();
        obj.insert("ts".into(), Value::from(ts));
        obj.insert("event".into(), Value::from(event));
        let mut f = fields;
        redact(&mut f);
        obj.insert("data".into(), f);
        let line = Value::Object(obj);
        if let Some(p) = &self.path {
            if let Ok(mut fh) = OpenOptions::new().create(true).append(true).open(p) {
                let _ = writeln!(fh, "{line}");
            }
        }
        if let Ok(mut m) = self.mem.lock() {
            m.push(line);
            if m.len() > 10_000 {
                m.drain(0..5_000);
            }
        }
    }

    pub fn events(&self) -> Vec<Value> {
        self.mem.lock().map(|m| m.clone()).unwrap_or_default()
    }
    pub fn count(&self, event: &str) -> usize {
        self.events().iter().filter(|e| e["event"] == event).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn redacts_secrets() {
        let a = AuditLog::in_memory();
        a.log(1, "X", serde_json::json!({"apiKey":"abc","nested":{"signature":"s"},"qty":"1"}));
        let e = &a.events()[0];
        assert_eq!(e["data"]["apiKey"], "***");
        assert_eq!(e["data"]["nested"]["signature"], "***");
        assert_eq!(e["data"]["qty"], "1");
    }
}
