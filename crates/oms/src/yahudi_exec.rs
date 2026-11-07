//! 4-Layer Cooldown & Symbol Probation Guard, serta Mistake Classifier & Playbook Engine
//! dari `Yahudi-Bot` (`SCALP` Mode).

use std::collections::BTreeMap;

use apex_core::types::Ms;
use serde::{Deserialize, Serialize};

// ============================================================================
// 1. 4-Layer Cooldown, Anti-Churn & Symbol Probation Guard (Yahudi-Bot RiskGuard)
// ============================================================================

pub const GLOBAL_LOSS_COOLDOWN_MS: Ms = 15 * 60 * 1000; // 15 menit setelah loss apapun
pub const GLOBAL_SL_COOLDOWN_MS: Ms = 60 * 60 * 1000; // 60 menit setelah terkena SL penuh
pub const SYMBOL_REENTRY_COOLDOWN_MS: Ms = 10 * 60 * 1000; // 10 menit per simbol setelah exit
pub const SYMBOL_PROBATION_MS: Ms = 45 * 60 * 1000; // 45 menit probation setelah 3x loss beruntun di simbol yang sama
pub const SYMBOL_PROBATION_LOSS_STREAK: u32 = 3;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct YahudiCooldownGuard {
    pub global_cooldown_until_ms: Ms,
    pub global_cooldown_reason: String,
    pub symbol_cooldown_until_ms: BTreeMap<String, Ms>,
    pub symbol_probation_until_ms: BTreeMap<String, Ms>,
    pub symbol_loss_streak: BTreeMap<String, u32>,
    pub tripwire_triggered: bool,
    pub tripwire_reason: Option<String>,
}

impl YahudiCooldownGuard {
    /// Cek apakah simbol diizinkan melakukan entry pada `now_ms`.
    pub fn can_enter(&self, symbol: &str, now_ms: Ms) -> Result<(), String> {
        if self.tripwire_triggered {
            return Err(format!(
                "TRIPWIRE_HALT:{}",
                self.tripwire_reason
                    .as_deref()
                    .unwrap_or("2X_MAX_RISK_LOSS")
            ));
        }
        if now_ms < self.global_cooldown_until_ms {
            let rem_s = (self.global_cooldown_until_ms - now_ms) / 1000;
            return Err(format!(
                "GLOBAL_COOLDOWN({}s: {})",
                rem_s, self.global_cooldown_reason
            ));
        }
        if let Some(&prob_until) = self.symbol_probation_until_ms.get(symbol) {
            if now_ms < prob_until {
                let rem_s = (prob_until - now_ms) / 1000;
                return Err(format!("SYMBOL_PROBATION_3_LOSSES({}s)", rem_s));
            }
        }
        if let Some(&cd_until) = self.symbol_cooldown_until_ms.get(symbol) {
            if now_ms < cd_until {
                let rem_s = (cd_until - now_ms) / 1000;
                return Err(format!("SYMBOL_REENTRY_COOLDOWN({}s)", rem_s));
            }
        }
        Ok(())
    }

    /// Catat penutupan posisi (exit) dan perbarui seluruh timer cooldown & probation.
    pub fn on_trade_closed(
        &mut self,
        symbol: &str,
        now_ms: Ms,
        net_pnl_usd: f64,
        close_reason: &str,
        max_risk_usd: f64,
    ) {
        // 1. Per-symbol re-entry cooldown (10 menit setelah exit apapun)
        self.symbol_cooldown_until_ms
            .insert(symbol.to_string(), now_ms + SYMBOL_REENTRY_COOLDOWN_MS);

        if net_pnl_usd < 0.0 {
            // 2. 2x Max Risk Loss Tripwire check
            if max_risk_usd > 0.0 && net_pnl_usd.abs() > 2.0 * max_risk_usd {
                self.tripwire_triggered = true;
                self.tripwire_reason = Some(format!(
                    "LOSS_${:.2}_EXCEEDS_2X_RISK_${:.2}",
                    net_pnl_usd.abs(),
                    max_risk_usd
                ));
            }

            // 3. Global SL Cooldown (60m) vs Global Loss Cooldown (15m)
            let is_sl = close_reason.to_ascii_uppercase().contains("SL");
            let cd_ms = if is_sl {
                GLOBAL_SL_COOLDOWN_MS
            } else {
                GLOBAL_LOSS_COOLDOWN_MS
            };
            let target_until = now_ms + cd_ms;
            if target_until > self.global_cooldown_until_ms {
                self.global_cooldown_until_ms = target_until;
                self.global_cooldown_reason = if is_sl {
                    format!("POST_SL_60M({})", symbol)
                } else {
                    format!("POST_LOSS_15M({})", symbol)
                };
            }

            // 4. Per-Symbol Loss Streak & 45m Probation setelah 3 loss beruntun
            let streak = self
                .symbol_loss_streak
                .entry(symbol.to_string())
                .and_modify(|c| *c += 1)
                .or_insert(1);
            if *streak >= SYMBOL_PROBATION_LOSS_STREAK {
                self.symbol_probation_until_ms
                    .insert(symbol.to_string(), now_ms + SYMBOL_PROBATION_MS);
            }
        } else if net_pnl_usd > 0.0 {
            // Reset loss streak simbol jika profit
            self.symbol_loss_streak.insert(symbol.to_string(), 0);
        }
    }
}

// ============================================================================
// 3. Automated Mistake Classifier & If-Then Playbook Engine (Package D)
// ============================================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MistakeTag {
    None,
    ImpulseChase,
    SpreadBlowout,
    CounterCvd,
    ChopWhipsaw,
    StopTooTight,
    NormalVariance,
}

impl MistakeTag {
    pub fn as_str(&self) -> &'static str {
        match self {
            MistakeTag::None => "NONE",
            MistakeTag::ImpulseChase => "IMPULSE_CHASE",
            MistakeTag::SpreadBlowout => "SPREAD_BLOWOUT",
            MistakeTag::CounterCvd => "COUNTER_CVD",
            MistakeTag::ChopWhipsaw => "CHOP_WHIPSAW",
            MistakeTag::StopTooTight => "STOP_TOO_TIGHT",
            MistakeTag::NormalVariance => "NORMAL_VARIANCE",
        }
    }
}

/// Klasifikasi otomatis penyebab loss (`mistakeClassifier.ts` di `Yahudi-Bot`).
pub fn classify_trade_mistake(
    net_pnl_usd: f64,
    hold_duration_ms: Ms,
    spread_pct_at_entry: f64,
    slippage_bps: f64,
    cvd_aligned_at_entry: bool,
    chop_14_at_entry: f64,
    sl_pct_at_entry: f64,
) -> MistakeTag {
    if net_pnl_usd >= 0.0 {
        return MistakeTag::None;
    }
    if spread_pct_at_entry > 0.0012 {
        return MistakeTag::SpreadBlowout;
    }
    if slippage_bps > 8.0 {
        return MistakeTag::ImpulseChase;
    }
    if !cvd_aligned_at_entry {
        return MistakeTag::CounterCvd;
    }
    if hold_duration_ms < 5 * 60 * 1000 && sl_pct_at_entry < 0.021 {
        return MistakeTag::StopTooTight;
    }
    if hold_duration_ms < 10 * 60 * 1000 && chop_14_at_entry > 58.0 {
        return MistakeTag::ChopWhipsaw;
    }
    MistakeTag::NormalVariance
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlaybookRule {
    pub id: String,
    pub mistake_tag: String,
    pub condition_desc: String,
    pub action: String, // "HARD_BLOCK" | "WARN"
    pub triggered_count: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PlaybookEngine {
    pub rules: Vec<PlaybookRule>,
    pub mistake_counts: BTreeMap<String, u32>,
}

impl Default for PlaybookEngine {
    fn default() -> Self {
        Self {
            rules: vec![
                PlaybookRule {
                    id: "PB-01".to_string(),
                    mistake_tag: "CHOP_WHIPSAW".to_string(),
                    condition_desc: "IF chop_14 > 61.8 AND adx_14 < 20 THEN HARD_BLOCK".to_string(),
                    action: "HARD_BLOCK".to_string(),
                    triggered_count: 0,
                },
                PlaybookRule {
                    id: "PB-02".to_string(),
                    mistake_tag: "SPREAD_BLOWOUT".to_string(),
                    condition_desc: "IF spread_pct > 0.15% THEN HARD_BLOCK".to_string(),
                    action: "HARD_BLOCK".to_string(),
                    triggered_count: 0,
                },
                PlaybookRule {
                    id: "PB-03".to_string(),
                    mistake_tag: "COUNTER_CVD".to_string(),
                    condition_desc: "IF cvd_aligned == false THEN HARD_BLOCK".to_string(),
                    action: "HARD_BLOCK".to_string(),
                    triggered_count: 0,
                },
            ],
            mistake_counts: BTreeMap::new(),
        }
    }
}

impl PlaybookEngine {
    pub fn record_mistake(&mut self, tag: MistakeTag) {
        if tag == MistakeTag::None {
            return;
        }
        *self
            .mistake_counts
            .entry(tag.as_str().to_string())
            .or_insert(0) += 1;
    }

    /// Evaluasi Playbook If-Then di hot-path sebelum mengeksekusi sinyal entry.
    pub fn evaluate_veto(
        &mut self,
        chop_14: f64,
        adx_14: f64,
        spread_pct: f64,
        cvd_aligned: bool,
    ) -> Result<(), String> {
        if chop_14 > 61.8 && adx_14 < 20.0 {
            if let Some(r) = self.rules.iter_mut().find(|r| r.id == "PB-01") {
                r.triggered_count += 1;
            }
            return Err("PLAYBOOK_VETO:PB-01(CHOP_WHIPSAW)".to_string());
        }
        if spread_pct > 0.0015 {
            if let Some(r) = self.rules.iter_mut().find(|r| r.id == "PB-02") {
                r.triggered_count += 1;
            }
            return Err("PLAYBOOK_VETO:PB-02(SPREAD_BLOWOUT)".to_string());
        }
        if !cvd_aligned {
            if let Some(r) = self.rules.iter_mut().find(|r| r.id == "PB-03") {
                r.triggered_count += 1;
            }
            return Err("PLAYBOOK_VETO:PB-03(COUNTER_CVD)".to_string());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cooldown_guard_enforces_sl_and_probation() {
        let mut g = YahudiCooldownGuard::default();
        let t0 = 1_000_000;
        g.on_trade_closed("SOLUSDT", t0, -1.0, "SL", 1.0);
        // Global SL cooldown 60m active
        assert!(g.can_enter("BTCUSDT", t0 + 60_000).is_err());
        assert!(g
            .can_enter("BTCUSDT", t0 + GLOBAL_SL_COOLDOWN_MS + 1)
            .is_ok());

        // 3 consecutive losses on SOLUSDT triggers 45m probation
        let t1 = t0 + GLOBAL_SL_COOLDOWN_MS + 10_000;
        g.on_trade_closed("SOLUSDT", t1, -0.5, "TREND_FLIP", 1.0);
        let t2 = t1 + GLOBAL_LOSS_COOLDOWN_MS + 10_000;
        g.on_trade_closed("SOLUSDT", t2, -0.5, "TREND_FLIP", 1.0);
        // After global loss cooldown expires (15m), SOLUSDT is still in 45m probation!
        assert!(g
            .can_enter("BTCUSDT", t2 + GLOBAL_LOSS_COOLDOWN_MS + 1)
            .is_ok());
        assert!(g
            .can_enter("SOLUSDT", t2 + GLOBAL_LOSS_COOLDOWN_MS + 1)
            .is_err());
    }
}
