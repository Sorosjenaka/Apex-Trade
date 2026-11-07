//! Notifikasi operasional (`Notifier`) untuk kejadian kritis mesin live/demo.
//!
//! Secara bawaan memakai `NoopNotifier` (tanpa jaringan). Bila variabel environment
//! `APEX_NOTIFY_URL` diset, `HttpWebhookNotifier` mengirim POST JSON `{level, event, text, ts}`
//! secara fire-and-forget dengan batas waktu 3 detik dan deduplikasi 60 detik per nama `event`
//! agar badai error beruntun tidak membanjiri kanal peringatan operator.

use apex_core::types::Ms;
use serde::Serialize;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub const NOTIFY_URL_ENV: &str = "APEX_NOTIFY_URL";
pub const NOTIFY_DEDUP_WINDOW_MS: Ms = 60_000;
pub const NOTIFY_TIMEOUT_SECS: u64 = 3;

pub trait Notifier: Send + Sync {
    fn notify(&self, level: &str, event: &str, text: &str, ts: Ms);
}

#[derive(Clone, Copy, Debug, Default)]
pub struct NoopNotifier;

impl Notifier for NoopNotifier {
    fn notify(&self, _level: &str, _event: &str, _text: &str, _ts: Ms) {}
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct NotifyPayload {
    pub level: String,
    pub event: String,
    pub text: String,
    pub ts: Ms,
}

pub struct HttpWebhookNotifier {
    url: String,
    client: reqwest::Client,
    last_sent_ms: Mutex<BTreeMap<String, Ms>>,
}

impl HttpWebhookNotifier {
    pub fn new(url: String) -> Result<Self, String> {
        let trimmed = url.trim().to_string();
        if trimmed.is_empty() {
            return Err("APEX_NOTIFY_URL kosong".into());
        }
        apex_data::init_tls();
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(NOTIFY_TIMEOUT_SECS))
            .build()
            .map_err(|e| format!("gagal membangun HTTP client notifier: {e}"))?;
        Ok(Self {
            url: trimmed,
            client,
            last_sent_ms: Mutex::new(BTreeMap::new()),
        })
    }

    pub fn from_env() -> Option<Self> {
        let raw = std::env::var(NOTIFY_URL_ENV).ok()?;
        if raw.trim().is_empty() {
            return None;
        }
        Self::new(raw).ok()
    }

    pub fn from_env_or_noop() -> Arc<dyn Notifier> {
        match Self::from_env() {
            Some(w) => Arc::new(w),
            None => Arc::new(NoopNotifier),
        }
    }

    /// Periksa dan catat jendela deduplikasi 60 detik per `event`.
    /// Mengembalikan `true` bila notifikasi diizinkan dikirim pada timestamp `ts`.
    pub fn should_send(&self, event: &str, ts: Ms) -> bool {
        let mut g = self.last_sent_ms.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(&prev) = g.get(event) {
            if ts >= prev && ts - prev < NOTIFY_DEDUP_WINDOW_MS {
                return false;
            }
        }
        g.insert(event.to_string(), ts);
        true
    }

    /// Kirim pesan uji sinkron (dipakai khusus `apex preflight --notify-test`).
    pub async fn send_sync(&self, payload: &NotifyPayload) -> Result<u16, String> {
        let resp = self
            .client
            .post(&self.url)
            .json(payload)
            .send()
            .await
            .map_err(|e| e.without_url().to_string())?;
        Ok(resp.status().as_u16())
    }
}

impl Notifier for HttpWebhookNotifier {
    fn notify(&self, level: &str, event: &str, text: &str, ts: Ms) {
        if !self.should_send(event, ts) {
            return;
        }
        let payload = NotifyPayload {
            level: level.to_string(),
            event: event.to_string(),
            text: text.to_string(),
            ts,
        };
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            let client = self.client.clone();
            let url = self.url.clone();
            handle.spawn(async move {
                let _ = client.post(&url).json(&payload).send().await;
            });
        }
    }
}

/// Notifier tiruan in-memory dengan deduplikasi 60 detik identik untuk pengujian deterministik tanpa jaringan.
#[derive(Clone, Default)]
pub struct RecordingNotifier {
    records: Arc<Mutex<Vec<NotifyPayload>>>,
    last_sent_ms: Arc<Mutex<BTreeMap<String, Ms>>>,
}

impl RecordingNotifier {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn records(&self) -> Vec<NotifyPayload> {
        self.records
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    pub fn count_event(&self, event: &str) -> usize {
        self.records()
            .into_iter()
            .filter(|r| r.event == event)
            .count()
    }
}

impl Notifier for RecordingNotifier {
    fn notify(&self, level: &str, event: &str, text: &str, ts: Ms) {
        {
            let mut dedup = self.last_sent_ms.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(&prev) = dedup.get(event) {
                if ts >= prev && ts - prev < NOTIFY_DEDUP_WINDOW_MS {
                    return;
                }
            }
            dedup.insert(event.to_string(), ts);
        }
        self.records
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(NotifyPayload {
                level: level.to_string(),
                event: event.to_string(),
                text: text.to_string(),
                ts,
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dedup_suppresses_identical_event_within_60s_only() {
        let w = HttpWebhookNotifier::new("http://127.0.0.1:9/webhook".into()).unwrap();
        let t0 = 1_700_000_000_000;
        assert!(w.should_send("SL_MISSING", t0));
        assert!(!w.should_send("SL_MISSING", t0 + 15_000));
        assert!(!w.should_send("SL_MISSING", t0 + 59_999));
        // Event berbeda tidak terblokir dedup SL_MISSING
        assert!(w.should_send("TASK_DEAD", t0 + 1_000));
        // Setelah >= 60 detik, event yang sama diizinkan lagi
        assert!(w.should_send("SL_MISSING", t0 + 60_000));
    }

    #[test]
    fn recording_notifier_records_and_deduplicates() {
        let rec = RecordingNotifier::new();
        let t0 = 1_700_000_000_000;
        rec.notify("CRITICAL", "KILL_ENGAGED", "kill test", t0);
        rec.notify("CRITICAL", "KILL_ENGAGED", "kill duplicate", t0 + 5_000);
        rec.notify("INFO", "BOOT", "booting", t0 + 5_000);
        assert_eq!(rec.count_event("KILL_ENGAGED"), 1);
        assert_eq!(rec.count_event("BOOT"), 1);
    }
}
