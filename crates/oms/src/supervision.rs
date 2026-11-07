//! Supervisi task latar (`user_stream`, `public_stream`) di jalur engine live/demo (WP-3d).
//!
//! Memantau `JoinHandle` dan timestamp detak jantung (`last_heartbeat`) setiap tick tanpa mengubah
//! strategi `panic` global:
//! - Bila `user_stream` mati (`is_finished()`) atau senyap melebihi `max_silence_ms` saat ada posisi
//!   terbuka pada mode non-paper → blokir entry baru, kirim notifikasi `TASK_DEAD`, tetap jalankan poll REST.
//! - Bila `public_stream` mati → blokir entry baru, kirim notifikasi `TASK_DEAD`, dan jadwalkan
//!   restart ber-backoff eksponensial (`1s → 60s`).

use crate::notifier::Notifier;
use apex_core::audit::AuditLog;
use apex_core::types::Ms;
use serde_json::json;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use tokio::task::JoinHandle;

pub const DEFAULT_USER_STREAM_SILENCE_MS: i64 = 90_000;
pub const RESTART_BACKOFF_MAX_MS: i64 = 60_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskHealth {
    Healthy,
    Silent { silence_ms: i64 },
    Dead,
}

pub struct Supervised {
    pub name: &'static str,
    pub handle: JoinHandle<()>,
    pub last_heartbeat: Arc<AtomicI64>,
    alerted: bool,
}

impl Supervised {
    pub fn new(name: &'static str, handle: JoinHandle<()>, last_heartbeat: Arc<AtomicI64>) -> Self {
        Self {
            name,
            handle,
            last_heartbeat,
            alerted: false,
        }
    }

    pub fn heartbeat_ms(&self) -> Ms {
        self.last_heartbeat.load(Ordering::Relaxed)
    }

    pub fn inspect(&self, now: Ms, max_silence_ms: Option<i64>) -> TaskHealth {
        if self.handle.is_finished() {
            return TaskHealth::Dead;
        }
        if let Some(limit) = max_silence_ms {
            let hb = self.heartbeat_ms();
            if hb > 0 && now - hb > limit {
                return TaskHealth::Silent {
                    silence_ms: now - hb,
                };
            }
        }
        TaskHealth::Healthy
    }

    /// Evaluasi kesehatan task untuk user-stream ketika ada posisi terbuka (non-paper).
    /// Mengembalikan `true` bila entry baru harus diblokir.
    pub fn check_user_stream(
        &mut self,
        now: Ms,
        has_open_position: bool,
        non_paper: bool,
        max_silence_ms: i64,
        audit: &AuditLog,
        notifier: &dyn Notifier,
    ) -> bool {
        if !non_paper {
            return false;
        }
        let limit = has_open_position.then_some(max_silence_ms);
        match self.inspect(now, limit) {
            TaskHealth::Healthy => {
                self.alerted = false;
                false
            }
            TaskHealth::Dead => {
                if !self.alerted {
                    self.alerted = true;
                    let msg = format!("task {} berhenti tak terduga; entry diblokir, fallback poll REST tetap aktif", self.name);
                    audit.log(now, "TASK_DEAD", json!({"task": self.name, "reason": "finished", "has_open_position": has_open_position}));
                    notifier.notify("CRITICAL", "TASK_DEAD", &msg, now);
                }
                has_open_position || self.handle.is_finished()
            }
            TaskHealth::Silent { silence_ms } => {
                if !self.alerted {
                    self.alerted = true;
                    let msg = format!(
                        "task {} senyap {} ms saat ada posisi terbuka; entry diblokir, fallback poll REST aktif",
                        self.name, silence_ms
                    );
                    audit.log(
                        now,
                        "TASK_DEAD",
                        json!({"task": self.name, "reason": "silent", "silence_ms": silence_ms}),
                    );
                    notifier.notify("WARN", "TASK_DEAD", &msg, now);
                }
                true
            }
        }
    }
}

/// Supervisor dengan kemampuan restart ber-backoff untuk task `public_stream`.
pub struct RestartablePublicSupervisor {
    pub task: Option<Supervised>,
    pub restart_attempts: u32,
    pub next_restart_ms: Ms,
}

impl Default for RestartablePublicSupervisor {
    fn default() -> Self {
        Self::new()
    }
}

impl RestartablePublicSupervisor {
    pub fn new() -> Self {
        Self {
            task: None,
            restart_attempts: 0,
            next_restart_ms: 0,
        }
    }

    pub fn attach(&mut self, sup: Supervised) {
        self.task = Some(sup);
    }

    pub fn backoff_ms(attempt: u32) -> i64 {
        let shift = attempt.min(6);
        (1_000_i64.saturating_mul(1_i64 << shift)).min(RESTART_BACKOFF_MAX_MS)
    }

    /// Periksa `public_stream`; bila mati, catat `TASK_DEAD`, blokir entry, dan jalankan `spawner`
    /// saat jadwal backoff tercapai. Mengembalikan `true` bila entry baru harus diblokir.
    pub fn poll_or_restart<F>(
        &mut self,
        now: Ms,
        audit: &AuditLog,
        notifier: &dyn Notifier,
        spawner: F,
    ) -> bool
    where
        F: FnOnce() -> Supervised,
    {
        let Some(ref cur) = self.task else {
            return false;
        };
        if !cur.handle.is_finished() {
            if self.restart_attempts > 0 && now - cur.heartbeat_ms() < 30_000 {
                self.restart_attempts = 0;
            }
            return false;
        }
        if self.next_restart_ms == 0 {
            let wait = Self::backoff_ms(self.restart_attempts);
            self.restart_attempts = self.restart_attempts.saturating_add(1);
            self.next_restart_ms = now + wait;
            let msg = format!(
                "task {} mati; entry diblokir dan dijadwalkan restart dalam {} ms",
                cur.name, wait
            );
            audit.log(now, "TASK_DEAD", json!({"task": cur.name, "reason": "finished", "restart_in_ms": wait, "attempt": self.restart_attempts}));
            notifier.notify("WARN", "TASK_DEAD", &msg, now);
            return true;
        }
        if now >= self.next_restart_ms {
            self.next_restart_ms = 0;
            let next = spawner();
            audit.log(
                now,
                "TASK_RESTARTED",
                json!({"task": next.name, "attempt": self.restart_attempts}),
            );
            self.task = Some(next);
            return false;
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notifier::RecordingNotifier;

    #[tokio::test]
    async fn dead_or_silent_user_stream_blocks_entries_and_notifies() {
        let hb = Arc::new(AtomicI64::new(1_000));
        let handle = tokio::spawn(async {
            tokio::time::sleep(std::time::Duration::from_secs(30)).await;
        });
        let mut sup = Supervised::new("user_stream", handle, hb.clone());
        let audit = AuditLog::in_memory();
        let notif = RecordingNotifier::new();

        // Sehat saat baru jalan
        assert!(!sup.check_user_stream(5_000, true, true, 10_000, &audit, &notif));
        assert_eq!(notif.count_event("TASK_DEAD"), 0);

        // Senyap > 10s saat posisi terbuka -> blok entry + notifikasi TASK_DEAD
        assert!(sup.check_user_stream(15_000, true, true, 10_000, &audit, &notif));
        assert_eq!(notif.count_event("TASK_DEAD"), 1);
        assert_eq!(audit.count("TASK_DEAD"), 1);

        // Detak jantung segar memulihkan status sehat
        hb.store(16_000, Ordering::Relaxed);
        assert!(!sup.check_user_stream(20_000, true, true, 10_000, &audit, &notif));
        sup.handle.abort();
    }

    #[tokio::test]
    async fn dead_public_stream_blocks_entries_and_restarts_with_backoff() {
        let hb = Arc::new(AtomicI64::new(1_000));
        let dead_handle = tokio::spawn(async {});
        tokio::task::yield_now().await;

        let mut pub_sup = RestartablePublicSupervisor::new();
        pub_sup.attach(Supervised::new("public_stream", dead_handle, hb));
        let audit = AuditLog::in_memory();
        let notif = RecordingNotifier::new();

        // Deteksi mati pertama -> blokir entry + jadwalkan backoff 1000 ms
        let blocked = pub_sup.poll_or_restart(2_000, &audit, &notif, || {
            panic!("tidak boleh langsung restart sebelum backoff selesai")
        });
        assert!(blocked);
        assert_eq!(notif.count_event("TASK_DEAD"), 1);

        // Sebelum 3_000 ms masih diblokir
        let blocked_mid =
            pub_sup.poll_or_restart(2_500, &audit, &notif, || panic!("belum waktunya restart"));
        assert!(blocked_mid);

        // Pada 3_000 ms restart dieksekusi dan blokir dibuka
        let blocked_after = pub_sup.poll_or_restart(3_000, &audit, &notif, || {
            let hb2 = Arc::new(AtomicI64::new(3_000));
            let h2 = tokio::spawn(async {
                tokio::time::sleep(std::time::Duration::from_secs(30)).await;
            });
            Supervised::new("public_stream", h2, hb2)
        });
        assert!(!blocked_after);
        assert_eq!(audit.count("TASK_RESTARTED"), 1);
        if let Some(t) = pub_sup.task {
            t.handle.abort();
        }
    }
}
