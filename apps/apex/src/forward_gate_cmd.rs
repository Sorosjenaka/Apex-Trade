//! Subcommand CLI `apex forward-gate --stage demo|shadow --since <YYYY-MM-DD>` (`WP-8`).
#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]

use std::path::{Path, PathBuf};

use apex_core::time::parse_ymd;
use apex_oms::{evaluate_forward_gate_files, ForwardGateReport, ForwardStage};

use crate::cfg::AllConfig;

pub struct ForwardGateOpts {
    pub stage: String,
    pub since: String,
    pub gate_config: Option<PathBuf>,
    pub telemetry: Option<PathBuf>,
    pub audit: Option<PathBuf>,
}

pub fn format_forward_gate_markdown(rep: &ForwardGateReport, since_str: &str) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "# Evaluasi Gerbang Forward — stage `{}` (sejak `{}`)\n\n",
        rep.stage.as_str(),
        since_str
    ));
    out.push_str(&format!(
        "Status Keseluruhan: **{}**\n\n",
        if rep.overall_pass { "PASS" } else { "NOT PASS" }
    ));
    out.push_str("| ID | Pemeriksaan | Nilai Terukur | Ambang (`Rules.md` / `forward_gate.toml`) | Status |\n");
    out.push_str("|---|---|---|---|---|\n");
    for r in &rep.rows {
        out.push_str(&format!(
            "| {} | {} | {} | {} | **{}** |\n",
            r.id,
            r.metric,
            r.observed,
            r.threshold,
            r.verdict.as_str()
        ));
    }
    out
}

pub fn run_forward_gate(cfg: &AllConfig, o: &ForwardGateOpts) -> anyhow::Result<bool> {
    let stage = ForwardStage::parse_stage(&o.stage).map_err(anyhow::Error::msg)?;
    let since_ms = parse_ymd(&o.since).ok_or_else(|| {
        anyhow::anyhow!(
            "format tanggal --since tidak valid '{}' (gunakan YYYY-MM-DD)",
            o.since
        )
    })?;
    let gate_cfg = o
        .gate_config
        .clone()
        .unwrap_or_else(|| cfg.dir.join("forward_gate.toml"));
    let tel_path = o
        .telemetry
        .clone()
        .unwrap_or_else(|| Path::new("audit/trades_telemetry.jsonl").to_path_buf());
    let sub = match stage {
        ForwardStage::Demo => "testnet",
        ForwardStage::Shadow => "paper",
    };
    let aud_path = o
        .audit
        .clone()
        .unwrap_or_else(|| cfg.engine.state_dir.join(sub).join("audit.jsonl"));

    let rep = evaluate_forward_gate_files(stage, &gate_cfg, &tel_path, &aud_path, since_ms)
        .map_err(anyhow::Error::msg)?;
    print!("{}", format_forward_gate_markdown(&rep, &o.since));
    Ok(rep.overall_pass)
}
