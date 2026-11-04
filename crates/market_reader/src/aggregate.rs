//! Aggregator regime (MARKET_READER.md §4).
use crate::readers::{CrowdFlags, Dir3, Trend};
use apex_core::types::{Ms, Side};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RegimeState {
    DataInvalid,
    ShockHalt,
    ChopNoTrade,
    RiskOff,
    RiskOnAlts,
    BtcLed,
}

impl RegimeState {
    pub fn as_str(&self) -> &'static str {
        match self {
            RegimeState::DataInvalid => "DATA_INVALID",
            RegimeState::ShockHalt => "SHOCK_HALT",
            RegimeState::ChopNoTrade => "CHOP_NO_TRADE",
            RegimeState::RiskOff => "RISK_OFF",
            RegimeState::RiskOnAlts => "RISK_ON_ALTS",
            RegimeState::BtcLed => "BTC_LED",
        }
    }
    pub fn is_trade_state(&self) -> bool {
        matches!(self, RegimeState::RiskOff | RegimeState::RiskOnAlts | RegimeState::BtcLed)
    }
    fn strictness(&self) -> u8 {
        match self {
            RegimeState::DataInvalid => 0,
            RegimeState::ShockHalt => 1,
            RegimeState::ChopNoTrade => 2,
            RegimeState::BtcLed => 3,
            RegimeState::RiskOff | RegimeState::RiskOnAlts => 4,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum UniverseScope {
    None,
    Majors,
    All,
}

/// Input aggregator pada satu `as_of` (semua sudah dihitung dari bar tertutup).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReaderSnapshot {
    pub as_of: Ms,
    /// MR-08 + status reader wajib (MR-01, MR-03, MR-04, MR-07).
    pub data_ok: bool,
    pub data_reasons: Vec<String>,
    /// MR-10 mentah (atau LIQ_CASCADE).
    pub shock: bool,
    pub btc_trend: Trend,
    pub adx_1h: f64,
    pub er_15m: f64,
    pub atr_pctile_1h: f64,
    pub dom_dir: Dir3,
    pub total_dir: Dir3,
    pub alt_dir: Dir3,
    pub breadth_up: f64,
    pub trend_share: f64,
    /// Sumber MR-03/04 adalah cadangan (TV-06) → max_pos=1.
    pub dom_fallback: bool,
    /// Reader opsional (MR-05/MR-06) gagal → max_pos=1.
    pub optional_missing: bool,
    pub crowd: CrowdFlags,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RegimeDecision {
    pub as_of: Ms,
    pub state: RegimeState,
    pub raw: RegimeState,
    pub long_ok: bool,
    pub short_ok: bool,
    pub universe: UniverseScope,
    pub max_positions: u32,
    pub reasons: Vec<String>,
}

impl RegimeDecision {
    pub fn side_ok(&self, s: Side) -> bool {
        match s {
            Side::Long => self.long_ok,
            Side::Short => self.short_ok,
        }
    }
}

/// Aturan pertama yang cocok menang. Ambang CHOP = default.
pub fn raw_state(s: &ReaderSnapshot) -> (RegimeState, Vec<String>) {
    raw_state_with(s, &ChopThresholds::default())
}

/// Sama dengan `raw_state`, dengan ambang CHOP eksplisit (sweep plateau, bukan live default).
pub fn raw_state_with(s: &ReaderSnapshot, th: &ChopThresholds) -> (RegimeState, Vec<String>) {
    if !s.data_ok {
        return (RegimeState::DataInvalid, s.data_reasons.clone());
    }
    if s.shock {
        return (RegimeState::ShockHalt, vec!["SHOCK".into()]);
    }
    let chop = th.is_chop(s.adx_1h, s.er_15m, s.trend_share);
    let expanding = s.atr_pctile_1h >= 40.0;
    if chop || !expanding || s.btc_trend == Trend::None {
        let mut r = Vec::new();
        if chop {
            r.push(format!("CHOP adx1h={:.1} er15={:.2} trend_share={:.0}%", s.adx_1h, s.er_15m, s.trend_share));
        }
        if !expanding {
            r.push(format!("EXPANDING=false atr_pctile={:.0}", s.atr_pctile_1h));
        }
        if s.btc_trend == Trend::None {
            r.push("btc_trend=NONE".into());
        }
        return (RegimeState::ChopNoTrade, r);
    }
    if s.btc_trend == Trend::Down && s.total_dir != Dir3::Rising && s.breadth_up <= 40.0 {
        return (RegimeState::RiskOff, vec![]);
    }
    if s.btc_trend == Trend::Up && s.dom_dir == Dir3::Falling && s.alt_dir == Dir3::Rising && s.breadth_up >= 60.0 {
        return (RegimeState::RiskOnAlts, vec![]);
    }
    (RegimeState::BtcLed, vec![format!("btc_trend={:?} dom={:?}", s.btc_trend, s.dom_dir)])
}

/// Ambang klausa CHOP di RG-2. Default = hipotesis MARKET_READER.md (ADX 18, ER 0,25, trend share 20%).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChopThresholds {
    pub adx_max: f64,
    pub er_max: f64,
    pub trend_share_min: f64,
}

impl Default for ChopThresholds {
    fn default() -> Self {
        Self { adx_max: 18.0, er_max: 0.25, trend_share_min: 20.0 }
    }
}

impl ChopThresholds {
    /// `CHOP` ⇔ (ADX 1h di bawah adx_max DAN ER 15m di bawah er_max) ATAU trend share di bawah trend_share_min.
    pub fn is_chop(self, adx_1h: f64, er_15m: f64, trend_share: f64) -> bool {
        (adx_1h < self.adx_max && er_15m < self.er_max) || trend_share < self.trend_share_min
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct RegimeMachine {
    pub state: Option<RegimeState>,
    pub entered_at: Ms,
    pub pending: Option<RegimeState>,
    pub pending_n: u32,
    pub last_pending_15m: Ms,
    pub shock_until: Ms,
    /// Arah BTC saat BTC_LED (searah btc_trend).
    pub btc_dir: i8,
    pub chop: ChopThresholds,
}

pub const DWELL_MS: Ms = 30 * 60_000;
pub const SHOCK_HOLD_MS: Ms = 30 * 60_000;

impl RegimeMachine {
    /// Dipanggil tiap close 5m (live) atau 15m (backtest). Histeresis loosening dihitung per close 15m.
    pub fn step(&mut self, s: &ReaderSnapshot) -> RegimeDecision {
        let now = s.as_of;
        if s.shock {
            self.shock_until = now + SHOCK_HOLD_MS;
        }
        let (mut raw, mut reasons) = raw_state_with(s, &self.chop);
        if raw != RegimeState::DataInvalid && now < self.shock_until {
            raw = RegimeState::ShockHalt;
            reasons = vec![format!("SHOCK hold sampai {}", apex_core::time::fmt_utc(self.shock_until))];
        }
        self.btc_dir = match s.btc_trend {
            Trend::Up => 1,
            Trend::Down => -1,
            Trend::None => 0,
        };
        let cur = self.state.unwrap_or(RegimeState::ChopNoTrade);
        let next = if raw == cur {
            self.pending = None;
            self.pending_n = 0;
            cur
        } else if raw.strictness() <= RegimeState::ChopNoTrade.strictness() {
            // lebih ketat (RG-0/1/2) → segera
            self.pending = None;
            self.pending_n = 0;
            raw
        } else {
            // longgar/lateral → butuh 2 close 15m beruntun (RG-6) + dwell (RG-7) bila antar trend-state
            let on_15m = now % 900_000 == 0;
            if self.pending == Some(raw) {
                if on_15m && now != self.last_pending_15m {
                    self.pending_n += 1;
                    self.last_pending_15m = now;
                }
            } else {
                self.pending = Some(raw);
                self.pending_n = if on_15m { 1 } else { 0 };
                self.last_pending_15m = if on_15m { now } else { 0 };
            }
            let tighter = raw.strictness() < cur.strictness();
            let dwell_ok = !cur.is_trade_state() || now - self.entered_at >= DWELL_MS;
            if (self.pending_n >= 2 || (tighter && cur.is_trade_state())) && dwell_ok {
                self.pending = None;
                self.pending_n = 0;
                raw
            } else if cur.is_trade_state() && !cur_conditions_hold(cur, raw) {
                // kondisi state saat ini sudah tidak berlaku dan belum boleh pindah → turun ke BTC_LED bila dwell belum,
                // tetapi BTC_LED itu sendiri lebih longgar dari CHOP; tetap di cur sampai dwell (RG-7).
                cur
            } else {
                cur
            }
        };
        if Some(next) != self.state {
            self.entered_at = now;
        }
        self.state = Some(next);
        self.decision(now, next, raw, reasons, s)
    }

    fn decision(&self, now: Ms, st: RegimeState, raw: RegimeState, reasons: Vec<String>, s: &ReaderSnapshot) -> RegimeDecision {
        let (long_ok, short_ok, universe, mut maxp) = match st {
            RegimeState::RiskOff => (false, true, UniverseScope::All, 2),
            RegimeState::RiskOnAlts => (true, false, UniverseScope::All, 2),
            RegimeState::BtcLed => (self.btc_dir > 0, self.btc_dir < 0, UniverseScope::Majors, 1),
            _ => (false, false, UniverseScope::None, 0),
        };
        if maxp > 0 && (s.dom_fallback || s.optional_missing) {
            maxp = 1;
        }
        if (st == RegimeState::RiskOnAlts && s.crowd.crowded_long) || (st == RegimeState::RiskOff && s.crowd.crowded_short) {
            maxp = maxp.min(1);
        }
        RegimeDecision { as_of: now, state: st, raw, long_ok: long_ok && maxp > 0, short_ok: short_ok && maxp > 0, universe, max_positions: maxp, reasons }
    }
}

fn cur_conditions_hold(cur: RegimeState, raw: RegimeState) -> bool {
    cur == raw
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(t: Ms) -> ReaderSnapshot {
        ReaderSnapshot {
            as_of: t,
            data_ok: true,
            data_reasons: vec![],
            shock: false,
            btc_trend: Trend::Up,
            adx_1h: 28.0,
            er_15m: 0.4,
            atr_pctile_1h: 60.0,
            dom_dir: Dir3::Falling,
            total_dir: Dir3::Rising,
            alt_dir: Dir3::Rising,
            breadth_up: 70.0,
            trend_share: 40.0,
            dom_fallback: false,
            optional_missing: false,
            crowd: CrowdFlags::default(),
        }
    }
    const Q: Ms = 900_000;

    #[test]
    fn raw_rules_order() {
        let mut s = snap(0);
        assert_eq!(raw_state(&s).0, RegimeState::RiskOnAlts);
        s.dom_dir = Dir3::Rising;
        assert_eq!(raw_state(&s).0, RegimeState::BtcLed);
        s.btc_trend = Trend::Down;
        s.total_dir = Dir3::Flat;
        s.breadth_up = 30.0;
        assert_eq!(raw_state(&s).0, RegimeState::RiskOff);
        s.atr_pctile_1h = 30.0;
        assert_eq!(raw_state(&s).0, RegimeState::ChopNoTrade);
        s.shock = true;
        assert_eq!(raw_state(&s).0, RegimeState::ShockHalt);
        s.data_ok = false;
        assert_eq!(raw_state(&s).0, RegimeState::DataInvalid);
        let mut s = snap(0);
        s.trend_share = 10.0;
        assert_eq!(raw_state(&s).0, RegimeState::ChopNoTrade);
    }

    #[test]
    fn chop_threshold_overrides_default() {
        let mut s = snap(0);
        s.trend_share = 12.0;
        assert_eq!(raw_state(&s).0, RegimeState::ChopNoTrade);
        let loose = ChopThresholds { trend_share_min: 10.0, ..ChopThresholds::default() };
        assert_eq!(raw_state_with(&s, &loose).0, RegimeState::RiskOnAlts);
        s.trend_share = 40.0;
        s.adx_1h = 30.0;
        s.er_15m = 0.5;
        let tight = ChopThresholds { trend_share_min: 50.0, ..ChopThresholds::default() };
        assert_eq!(raw_state_with(&s, &tight).0, RegimeState::ChopNoTrade);
        assert!(ChopThresholds::default().is_chop(10.0, 0.10, 40.0));
        assert!(!ChopThresholds { adx_max: 8.0, er_max: 0.05, trend_share_min: 5.0 }.is_chop(10.0, 0.10, 40.0));
    }

    #[test]
    fn loosen_needs_two_15m_closes_tighten_immediate() {
        let mut m = RegimeMachine::default();
        let d = m.step(&snap(Q));
        assert_eq!(d.state, RegimeState::ChopNoTrade);
        assert_eq!(d.max_positions, 0);
        let d = m.step(&snap(2 * Q));
        assert_eq!(d.state, RegimeState::RiskOnAlts);
        assert!(d.long_ok && !d.short_ok && d.max_positions == 2);
        let mut s = snap(3 * Q);
        s.btc_trend = Trend::None;
        assert_eq!(m.step(&s).state, RegimeState::ChopNoTrade); // segera
    }

    #[test]
    fn shock_hold_30m() {
        let mut m = RegimeMachine::default();
        m.step(&snap(Q));
        m.step(&snap(2 * Q));
        let mut s = snap(3 * Q);
        s.shock = true;
        assert_eq!(m.step(&s).state, RegimeState::ShockHalt);
        assert_eq!(m.step(&snap(4 * Q)).state, RegimeState::ShockHalt); // +15m masih hold
        assert_eq!(m.step(&snap(5 * Q)).state, RegimeState::ShockHalt); // hold habis tepat 30m: raw OK, butuh 2 close
        assert_eq!(m.step(&snap(6 * Q)).state, RegimeState::RiskOnAlts);
    }

    #[test]
    fn dwell_between_trend_states() {
        let mut m = RegimeMachine::default();
        m.step(&snap(Q));
        assert_eq!(m.step(&snap(2 * Q)).state, RegimeState::RiskOnAlts);
        let mut s = snap(3 * Q);
        s.dom_dir = Dir3::Rising; // → BTC_LED (lebih ketat, antar trend-state) tapi dwell 30m belum
        assert_eq!(m.step(&s).state, RegimeState::RiskOnAlts);
        s.as_of = 4 * Q;
        let d = m.step(&s);
        assert_eq!(d.state, RegimeState::BtcLed);
        assert_eq!(d.universe, UniverseScope::Majors);
        assert_eq!(d.max_positions, 1);
        assert!(d.long_ok);
    }

    #[test]
    fn fallback_and_crowding_cap() {
        let mut m = RegimeMachine::default();
        let mut s = snap(Q);
        s.dom_fallback = true;
        m.step(&s);
        s.as_of = 2 * Q;
        assert_eq!(m.step(&s).max_positions, 1);
        let mut s = snap(3 * Q);
        s.crowd.crowded_long = true;
        assert_eq!(m.step(&s).max_positions, 1);
    }
}
