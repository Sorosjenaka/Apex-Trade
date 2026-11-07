//! apex — CLI Apex Scalper RS.
//! Default AMAN: mode=paper, arm=false. Tidak ada flag CLI untuk ARM (hanya config/engine.toml).
mod bt;
mod cfg;
mod forward_gate_cmd;
mod live;
mod ops_cmd;
mod review_cmd;
mod review_stats;
#[cfg(test)]
mod review_tests;

use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "apex",
    version,
    about = "Apex Scalper RS — Binance USDⓈ-M scalper (paper default, ARM=false)"
)]
struct Cli {
    /// folder config (engine.toml, risk.toml, screener.toml, strategies.toml, blacklist.json)
    #[arg(long, global = true, default_value = "config")]
    config: PathBuf,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(clap::Args, Clone)]
struct BtOpts {
    #[arg(long, default_value = "2025-02-01")]
    from: String,
    /// eksklusif
    #[arg(long, default_value = "2026-05-01")]
    to: String,
    /// mis. S1,S3 (default: enabled di strategies.toml)
    #[arg(long)]
    strategies: Option<String>,
    #[arg(long, default_value = "data")]
    data: PathBuf,
    #[arg(long, default_value = "reports")]
    out: PathBuf,
    /// reader (default) | screener
    #[arg(long, default_value = "reader")]
    regime: String,
    /// fee 2×, slip +0,05%, funding 2× (G-05)
    #[arg(long)]
    stress: bool,
    /// buka kunci holdout 2026-05-01..2026-10-31 (sekali per param_hash)
    #[arg(long)]
    unlock_holdout: bool,
    /// file exchangeInfo (default config/filters_snapshot.json)
    #[arg(long)]
    filters: Option<PathBuf>,
    #[arg(long)]
    no_volume_gate: bool,
    /// jangan ulang sweep CHOP; baca validation_summary.json
    #[arg(long)]
    gates_only: bool,
}

#[derive(Subcommand)]
enum Cmd {
    /// Unduh klines 15m/1h + funding PUBLIK ke CSV
    Fetch {
        #[arg(long, default_value = "2025-01-01")]
        from: String,
        #[arg(long, default_value = "2026-05-01")]
        to: String,
        #[arg(long, default_value = "data")]
        data: PathBuf,
        /// Hanya ambil candle 5m publik untuk riset; tidak menimpa 15m/1h/funding.
        #[arg(long)]
        only_5m: bool,
    },
    /// Market Reader (regime global) dari data publik
    Reader,
    /// Screener: allowlist + volume gate + regime per simbol
    Screen {
        #[arg(long, default_value_t = 15)]
        top: usize,
    },
    /// Backtest event-driven (bar tertutup, tanpa look-ahead)
    Backtest {
        #[command(flatten)]
        o: BtOpts,
        /// jalankan tiap strategi terpisah + gabungan
        #[arg(long)]
        per_strategy: bool,
    },
    /// Gate validasi G-02..G-11 per strategi (G-11 terkunci kecuali --unlock-holdout)
    Validate {
        #[command(flatten)]
        o: BtOpts,
    },
    /// Rekam bar tertutup dari WebSocket publik ke CSV
    Record {
        #[arg(long, default_value = "data")]
        data: PathBuf,
    },
    /// Jalankan engine. paper = simulasi lokal (tanpa order). testnet/live butuh arm=true di config.
    Run {
        #[arg(long, default_value = "paper")]
        mode: String,
        /// satu siklus 15m lalu keluar
        #[arg(long)]
        once: bool,
    },
    /// Kill switch: blok entry baru (file). --flatten = tutup posisi oleh engine yang berjalan.
    Kill {
        #[arg(long)]
        release: bool,
        #[arg(long)]
        flatten: bool,
        #[arg(long, default_value = "manual")]
        reason: String,
    },
    /// Status state OMS + kill switch
    Status,
    /// Laporan evaluasi berkala dari telemetri nyata (+ sidecar JSON)
    Review {
        /// Tanggal awal UTC inklusif (YYYY-MM-DD)
        #[arg(long)]
        since: String,
        /// Tanggal akhir UTC eksklusif (YYYY-MM-DD)
        #[arg(long)]
        until: Option<String>,
        /// Filter kode strategi (mis. S1)
        #[arg(long)]
        strategy: Option<String>,
        /// Path laporan Markdown keluaran (.json dibuat otomatis berdampingan)
        #[arg(long)]
        out: PathBuf,
        /// Opsional override path trades_telemetry.jsonl
        #[arg(long)]
        telemetry: Option<PathBuf>,
        /// Opsional override path audit.jsonl
        #[arg(long)]
        audit: Option<PathBuf>,
    },
    /// Evaluasi gerbang forward GR-9 (--stage demo|shadow --since YYYY-MM-DD)
    ForwardGate {
        /// demo | shadow
        #[arg(long)]
        stage: String,
        /// Tanggal awal pengamatan UTC (YYYY-MM-DD)
        #[arg(long)]
        since: String,
        /// Opsional override path forward_gate.toml
        #[arg(long)]
        gate_config: Option<PathBuf>,
        /// Opsional override path trades_telemetry.jsonl
        #[arg(long)]
        telemetry: Option<PathBuf>,
        /// Opsional override path audit.jsonl
        #[arg(long)]
        audit: Option<PathBuf>,
    },
    /// Pemeriksaan pra-terbang read-only (--target demo|live [--notify-test])
    Preflight {
        /// demo | live
        #[arg(long, default_value = "demo")]
        target: String,
        /// Uji kirim webhook notifikasi HTTP sinkron
        #[arg(long)]
        notify_test: bool,
    },
    /// Uji asap order limit GTX aman di Binance Demo Futures
    DemoSmoke,
}

fn to_bt(o: BtOpts) -> bt::BtArgs {
    bt::BtArgs {
        from: o.from,
        to: o.to,
        strategies: o.strategies,
        data: o.data,
        out: o.out,
        regime: o.regime,
        stress: o.stress,
        unlock_holdout: o.unlock_holdout,
        filters: o.filters,
        no_volume_gate: o.no_volume_gate,
        gates_only: o.gates_only,
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "warn".into()),
        )
        .init();
    apex_data::init_tls();
    let cli = Cli::parse();
    let mut cfg = cfg::AllConfig::load(&cli.config)?;
    match cli.cmd {
        Cmd::Fetch {
            from,
            to,
            data,
            only_5m,
        } => live::fetch(&cfg, &data, &from, &to, only_5m).await,
        Cmd::Reader => live::reader(&cfg).await,
        Cmd::Screen { top } => live::screen(&cfg, top).await,
        Cmd::Backtest { o, per_strategy } => bt::backtest(&cfg, &to_bt(o), per_strategy),
        Cmd::Validate { o } => bt::validate(&cfg, &to_bt(o)),
        Cmd::Record { data } => live::record(&cfg, &data).await,
        Cmd::Run { mode, once } => {
            let mode: apex_core::Mode = mode.parse().map_err(|e: String| anyhow::anyhow!(e))?;
            live::run(&mut cfg, live::RunOpts { mode, once }).await
        }
        Cmd::Kill {
            release,
            flatten,
            reason,
        } => {
            let k = apex_oms::killswitch::KillSwitch::new(&cfg.engine.kill_file);
            if release {
                k.release()?;
                println!("kill switch DILEPAS ({})", k.path().display());
            } else {
                k.engage(&format!(
                    "{}{}",
                    if flatten { "FLATTEN " } else { "" },
                    reason
                ))?;
                println!(
                    "kill switch AKTIF ({}) — entry baru diblokir{}",
                    k.path().display(),
                    if flatten {
                        ", engine akan flatten posisi"
                    } else {
                        ""
                    }
                );
            }
            Ok(())
        }
        Cmd::Status => {
            let k = apex_oms::killswitch::KillSwitch::new(&cfg.engine.kill_file);
            println!(
                "mode config={:?} arm={} | kill={}",
                cfg.engine.mode,
                cfg.engine.arm,
                if k.is_engaged() {
                    format!("AKTIF: {}", k.reason().unwrap_or_default())
                } else {
                    "off".into()
                }
            );
            for sub in ["paper", "testnet", "live"] {
                let p = cfg.engine.state_dir.join(sub).join("oms_state.json");
                if !p.exists() {
                    continue;
                }
                let b: apex_oms::manager::Book =
                    serde_json::from_str(&std::fs::read_to_string(&p)?)?;
                println!("[{sub}] aktif={} selesai={} SL hari ini={} pnl hari ini=${:.2} cooldown s/d {}", b.trades.len(), b.closed.len(), b.risk.daily_sl_count, b.risk.daily_realized_net, apex_core::time::fmt_utc(b.risk.cooldown_until));
                for t in b.trades.values() {
                    println!(
                        "   {} {} {:?} {:?} qty={} sl={} tp={} {}",
                        t.client_id,
                        t.symbol,
                        t.side,
                        t.state,
                        t.filled_qty,
                        t.sl,
                        t.tp,
                        t.last_error.clone().unwrap_or_default()
                    );
                }
            }
            Ok(())
        }
        Cmd::Review {
            since,
            until,
            strategy,
            out,
            telemetry,
            audit,
        } => {
            review_cmd::run_review(
                &cfg,
                &review_cmd::ReviewOpts {
                    since,
                    until,
                    strategy,
                    out,
                    telemetry,
                    audit,
                },
            )?;
            Ok(())
        }
        Cmd::ForwardGate {
            stage,
            since,
            gate_config,
            telemetry,
            audit,
        } => {
            let pass = forward_gate_cmd::run_forward_gate(
                &cfg,
                &forward_gate_cmd::ForwardGateOpts {
                    stage,
                    since,
                    gate_config,
                    telemetry,
                    audit,
                },
            )?;
            if !pass {
                std::process::exit(1);
            }
            Ok(())
        }
        Cmd::Preflight {
            target,
            notify_test,
        } => {
            let pass = ops_cmd::run_preflight_cli(&cfg, &target, notify_test).await?;
            if !pass {
                std::process::exit(1);
            }
            Ok(())
        }
        Cmd::DemoSmoke => ops_cmd::run_demo_smoke_cli(&cfg).await,
    }
}
