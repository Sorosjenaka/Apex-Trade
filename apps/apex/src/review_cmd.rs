//! Subcommand `apex review`: laporan berkala Markdown + sidecar JSON (`WP-7a`).
#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]

use std::fs;
use std::path::{Path, PathBuf};

use apex_core::time::parse_ymd;
use apex_oms::{evaluate_forward_gate_files, read_telemetry_jsonl, ForwardStage, TradeTelemetry};

use crate::cfg::AllConfig;
use crate::review_stats::{
    build_conclusion, compute_attribution, compute_concentration, compute_excursion_stats,
    compute_execution_quality, compute_forensics_pareto, compute_summary, parse_skip_funnel,
    AttributionCell, ReviewReportData,
};

pub struct ReviewOpts {
    pub since: String,
    pub until: Option<String>,
    pub strategy: Option<String>,
    pub out: PathBuf,
    pub telemetry: Option<PathBuf>,
    pub audit: Option<PathBuf>,
}

pub fn sidecar_json_path(md_path: &Path) -> PathBuf {
    if md_path.extension().and_then(|e| e.to_str()) == Some("md") {
        md_path.with_extension("json")
    } else {
        let mut s = md_path.as_os_str().to_os_string();
        s.push(".json");
        PathBuf::from(s)
    }
}

fn fmt_opt_f(v: Option<f64>, dp: usize, suffix: &str) -> String {
    match v {
        Some(x) => format!("{x:.dp$}{suffix}"),
        None => "INSUFFICIENT".into(),
    }
}

fn render_summary_md(out: &mut String, d: &ReviewReportData) {
    let s = &d.summary;
    out.push_str("## 1. Ringkasan Statistik\n\n");
    out.push_str(&format!(
        "- Status sampel: **{}** (`n = {}`, minggu ISO = `{}`)\n",
        s.sample_status, s.n, s.iso_weeks
    ));
    let er_str = match (s.mean_r, s.ci95_er) {
        (Some(m), Some((lo, hi))) => {
            format!("{m:+.4}R (CI 95% block-bootstrap ISO-week: [{lo:+.4}R, {hi:+.4}R])")
        }
        (Some(m), None) => format!("{m:+.4}R (CI 95% INSUFFICIENT: butuh >= 2 minggu ISO)"),
        _ => "INSUFFICIENT (n=0)".into(),
    };
    out.push_str(&format!("- Ekspektasi `E[R]`: `{er_str}`\n"));
    let wr_str = match (s.win_rate_pct, &s.wilson95_pct) {
        (Some(wr), Some(w)) => format!(
            "{wr:.2}% ({}/{} menang; Wilson 95%: [{:.2}%, {:.2}%])",
            s.wins, s.n, w.lower_pct, w.upper_pct
        ),
        _ => "INSUFFICIENT (n=0)".into(),
    };
    out.push_str(&format!("- Win Rate: `{wr_str}`\n"));
    let pf_str = match s.profit_factor {
        Some(pf) => format!("{pf:.3}"),
        None => "INSUFFICIENT (PF hanya dihitung bila n >= 100)".into(),
    };
    out.push_str(&format!("- Profit Factor: `{pf_str}`\n\n"));
}

fn render_cells_md(out: &mut String, title: &str, cells: &[AttributionCell]) {
    out.push_str(&format!("### {title}\n\n"));
    if cells.is_empty() {
        out.push_str("_Tidak ada sampel (`INSUFFICIENT`)._\n\n");
        return;
    }
    out.push_str("| Kategori | n | Total R | Mean R | Catatan |\n|---|---|---|---|---|\n");
    for c in cells {
        let note = if c.flagged_small_n {
            "INSUFFICIENT (n<10)"
        } else {
            "OK"
        };
        out.push_str(&format!(
            "| {} | {} | {:+.4}R | {:+.4}R | {} |\n",
            c.key, c.n, c.sum_r, c.mean_r, note
        ));
    }
    out.push('\n');
}

fn render_exec_and_excursion_md(out: &mut String, d: &ReviewReportData) {
    let e = &d.execution;
    out.push_str("## 4. Kualitas Eksekusi\n\n");
    out.push_str(&format!(
        "- Slippage realisasi vs model (`n_non_paper={}`): mean `{}`, p95 `{}`\n",
        e.non_paper_n,
        fmt_opt_f(e.slip_diff_mean_r, 4, "R"),
        fmt_opt_f(e.slip_diff_p95_r, 4, "R")
    ));
    out.push_str(&format!(
        "- Rasio fill maker: `{}` | Waktu-ke-fill p50/p95: `{}` / `{}`\n",
        fmt_opt_f(e.maker_fill_ratio.map(|x| x * 100.0), 2, "%"),
        fmt_opt_f(e.time_to_fill_p50_ms, 0, " ms"),
        fmt_opt_f(e.time_to_fill_p95_ms, 0, " ms")
    ));
    out.push_str(&format!(
        "- Durasi tanpa proteksi SL p50/p95/maks: `{}` / `{}` / `{}`\n\n",
        fmt_opt_f(e.unprotected_p50_ms, 0, " ms"),
        fmt_opt_f(e.unprotected_p95_ms, 0, " ms"),
        e.unprotected_max_ms
            .map(|m| format!("{m} ms"))
            .unwrap_or_else(|| "INSUFFICIENT".into())
    ));

    let x = &d.excursion;
    out.push_str("## 5. Excursion & Gave-Back\n\n");
    out.push_str(&format!(
        "- Sampel ber-excursion 15m: `{}` | MFE p50/p95: `{}` / `{}` | MAE p50/p95: `{}` / `{}`\n",
        x.n_with_excursion,
        fmt_opt_f(x.mfe_p50_r, 3, "R"),
        fmt_opt_f(x.mfe_p95_r, 3, "R"),
        fmt_opt_f(x.mae_p50_r, 3, "R"),
        fmt_opt_f(x.mae_p95_r, 3, "R")
    ));
    out.push_str(&format!(
        "- Tingkat *gave-back* (`MFE >= 1.0R` lalu `SlHit`): `{}` trade (`{}`)\n\n",
        x.gave_back_count,
        fmt_opt_f(x.gave_back_rate_pct, 2, "%")
    ));
}

fn render_tail_sections_md(out: &mut String, d: &ReviewReportData) {
    let c = &d.concentration;
    out.push_str("## 6. Konsentrasi Hasil\n\n");
    out.push_str(&format!("- Total PnL neto: `${:.4}`\n", c.net_pnl_usd));
    let fmt_top = |t: &Option<(String, f64, Option<f64>)>| match t {
        Some((k, v, Some(sh))) => format!("{k}: ${v:.4} ({sh:.2}% dari net positif)"),
        Some((k, v, None)) => format!("{k}: ${v:.4} (porsi % N/A karena total net <= 0)"),
        None => "INSUFFICIENT".into(),
    };
    out.push_str(&format!("- Simbol teratas: `{}`\n", fmt_top(&c.top_symbol)));
    out.push_str(&format!("- Bulan teratas: `{}`\n\n", fmt_top(&c.top_month)));

    out.push_str("## 7. Forensik Teragregasi (Pareto)\n\n");
    out.push_str("> Catatan: RCA satu trade bukan bukti kausal; hanya pola berulang lintas sampel yang layak diuji pra-registrasi.\n\n");
    if d.forensics.mistake_tags.is_empty() {
        out.push_str("- Tag kesalahan: `INSUFFICIENT / 0 trade rugi`\n");
    } else {
        for (tag, cnt) in &d.forensics.mistake_tags {
            out.push_str(&format!("- Tag `{tag}`: `{cnt}`\n"));
        }
    }
    for (lay, cnt) in &d.forensics.supported_layers {
        out.push_str(&format!("- Layer 5-Whys Supported `{lay}`: `{cnt}`\n"));
    }
    out.push('\n');

    out.push_str("## 8. Status Gerbang Forward (`WP-8`)\n\n");
    for (lbl, rep) in [("demo", &d.forward_demo), ("shadow", &d.forward_shadow)] {
        match rep {
            Some(r) => out.push_str(&format!(
                "- Stage `{lbl}`: **{}** (durasi `{:.2}` hari, trade `{}`)\n",
                if r.overall_pass { "PASS" } else { "NOT PASS" },
                r.elapsed_days,
                r.trade_count
            )),
            None => out.push_str(&format!("- Stage `{lbl}`: `INSUFFICIENT`\n")),
        }
    }
    out.push_str("\n## 9. Kesimpulan Otomatis\n\n");
    out.push_str(&format!("**{}** — {}\n", d.conclusion, d.conclusion_detail));
}

pub fn render_review_markdown(d: &ReviewReportData) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "# Laporan Review Berkala Apex-Trade (`{}` s/d `{}`)\n\n",
        d.since,
        d.until.as_deref().unwrap_or("sekarang")
    ));
    if let Some(st) = &d.strategy_filter {
        out.push_str(&format!("Filter strategi: `{st}`\n\n"));
    }
    render_summary_md(&mut out, d);

    out.push_str("## 2. Funnel Alasan Skip\n\n");
    if d.skip_funnel.is_empty() {
        out.push_str("_Tidak ada event `SKIP` pada rentang ini (`INSUFFICIENT`)._\n\n");
    } else {
        out.push_str("| Alasan Skip | Jumlah |\n|---|---|\n");
        for (k, v) in &d.skip_funnel {
            out.push_str(&format!("| {k} | {v} |\n"));
        }
        out.push('\n');
    }

    out.push_str("## 3. Atribusi R\n\n");
    render_cells_md(&mut out, "3.1 Per Strategi", &d.attribution.by_strategy);
    render_cells_md(&mut out, "3.2 Per Simbol", &d.attribution.by_symbol);
    render_cells_md(&mut out, "3.3 Per Sisi", &d.attribution.by_side);
    render_cells_md(&mut out, "3.4 Per Jam UTC", &d.attribution.by_utc_hour);
    render_cells_md(&mut out, "3.5 Per Rezim", &d.attribution.by_regime);
    render_cells_md(&mut out, "3.6 Per Bias Makro", &d.attribution.by_macro_bias);
    render_exec_and_excursion_md(&mut out, d);
    render_tail_sections_md(&mut out, d);
    out
}

pub fn build_review_report_data(
    since: &str,
    until: Option<&str>,
    strategy: Option<&str>,
    all_trades: &[TradeTelemetry],
    audit_paths: &[&Path],
    gate_cfg_path: Option<&Path>,
) -> anyhow::Result<ReviewReportData> {
    let since_ms = parse_ymd(since)
        .ok_or_else(|| anyhow::anyhow!("tanggal --since tidak valid '{since}' (YYYY-MM-DD)"))?;
    let until_ms = match until {
        Some(u) => Some(
            parse_ymd(u)
                .ok_or_else(|| anyhow::anyhow!("tanggal --until tidak valid '{u}' (YYYY-MM-DD)"))?,
        ),
        None => None,
    };
    let filtered: Vec<TradeTelemetry> = all_trades
        .iter()
        .filter(|t| t.closed_ms >= since_ms && until_ms.is_none_or(|u| t.closed_ms < u))
        .filter(|t| strategy.is_none_or(|s| t.strategy.eq_ignore_ascii_case(s)))
        .cloned()
        .collect();

    let summary = compute_summary(&filtered);
    let (conclusion, conclusion_detail) = build_conclusion(&summary);
    let skip_funnel = parse_skip_funnel(audit_paths, since_ms, until_ms);
    let attribution = compute_attribution(&filtered);
    let execution = compute_execution_quality(&filtered);
    let excursion = compute_excursion_stats(&filtered);
    let concentration = compute_concentration(&filtered);
    let forensics = compute_forensics_pareto(&filtered);

    let (forward_demo, forward_shadow) = if let Some(gcp) = gate_cfg_path.filter(|p| p.exists()) {
        let empty_tel = Path::new("__nonexistent_tel__.jsonl");
        let aud0 = audit_paths.first().copied().unwrap_or(empty_tel);
        let d = apex_oms::ForwardGateFile::load(gcp).ok().map(|gf| {
            let aud =
                apex_oms::parse_audit_forward_stats(aud0, since_ms, 1_800_000).unwrap_or_default();
            let dm = apex_oms::evaluate_forward_gate_data(
                ForwardStage::Demo,
                gf.thresholds_for(ForwardStage::Demo),
                since_ms,
                &filtered,
                aud.clone(),
            );
            let sh = apex_oms::evaluate_forward_gate_data(
                ForwardStage::Shadow,
                gf.thresholds_for(ForwardStage::Shadow),
                since_ms,
                &filtered,
                aud,
            );
            (dm, sh)
        });
        match d {
            Some((dm, sh)) => (Some(dm), Some(sh)),
            None => (None, None),
        }
    } else {
        (None, None)
    };

    Ok(ReviewReportData {
        since: since.into(),
        until: until.map(str::to_string),
        strategy_filter: strategy.map(str::to_string),
        summary,
        skip_funnel,
        attribution,
        execution,
        excursion,
        concentration,
        forensics,
        forward_demo,
        forward_shadow,
        conclusion,
        conclusion_detail,
    })
}

pub fn run_review(cfg: &AllConfig, o: &ReviewOpts) -> anyhow::Result<(PathBuf, PathBuf)> {
    let tel_path = o
        .telemetry
        .clone()
        .unwrap_or_else(|| Path::new("audit/trades_telemetry.jsonl").to_path_buf());
    let trades = read_telemetry_jsonl(&tel_path)?;
    let default_auds = [
        cfg.engine.state_dir.join("paper").join("audit.jsonl"),
        cfg.engine.state_dir.join("testnet").join("audit.jsonl"),
        cfg.engine.state_dir.join("live").join("audit.jsonl"),
    ];
    let custom_aud: Vec<PathBuf> = o.audit.iter().cloned().collect();
    let aud_refs: Vec<&Path> = if custom_aud.is_empty() {
        default_auds.iter().map(PathBuf::as_path).collect()
    } else {
        custom_aud.iter().map(PathBuf::as_path).collect()
    };
    let gate_cfg = cfg.dir.join("forward_gate.toml");
    let _ = evaluate_forward_gate_files; // re-used via ForwardGateFile + evaluate_forward_gate_data
    let data = build_review_report_data(
        &o.since,
        o.until.as_deref(),
        o.strategy.as_deref(),
        &trades,
        &aud_refs,
        Some(&gate_cfg),
    )?;
    let md = render_review_markdown(&data);
    let json_str = serde_json::to_string_pretty(&data)?;
    if let Some(parent) = o.out.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }
    let json_out = sidecar_json_path(&o.out);
    fs::write(&o.out, &md)?;
    fs::write(&json_out, json_str)?;
    println!(
        "Laporan review ditulis ke {} dan {}",
        o.out.display(),
        json_out.display()
    );
    Ok((o.out.clone(), json_out))
}
