//! CSV klines/funding (format sama dengan fetcher riset): header
//! open_time,open,high,low,close,volume,close_time,quote_volume,trades,taker_buy_base,taker_buy_quote
use apex_core::types::{Bar, Ms, Tf};
use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::future::Future;
use std::io::Write;
use std::path::{Path, PathBuf};

pub fn load_klines(path: &Path) -> anyhow::Result<Vec<Bar>> {
    let mut rdr = csv::ReaderBuilder::new()
        .has_headers(true)
        .from_path(path)?;
    let mut out: Vec<Bar> = Vec::new();
    for rec in rdr.records() {
        let r = rec?;
        let f = |i: usize| -> anyhow::Result<f64> {
            Ok(r.get(i)
                .ok_or_else(|| anyhow::anyhow!("kolom {i}"))?
                .trim()
                .parse::<f64>()?)
        };
        let b = Bar {
            open_time: r.get(0).unwrap_or("").trim().parse::<i64>()?,
            open: f(1)?,
            high: f(2)?,
            low: f(3)?,
            close: f(4)?,
            volume: f(5)?,
            quote_volume: f(7)?,
            trades: f(8)? as u64,
            taker_buy_base: f(9)?,
        };
        if let Some(last) = out.last() {
            if b.open_time <= last.open_time {
                continue; // dedup / urutkan monoton
            }
        }
        out.push(b);
    }
    Ok(out)
}

pub fn save_klines(path: &Path, tf: Tf, bars: &[Bar]) -> anyhow::Result<()> {
    if let Some(p) = path.parent() {
        fs::create_dir_all(p)?;
    }
    let tmp = path.with_extension("csv.tmp");
    {
        let mut w = csv::Writer::from_path(&tmp)?;
        w.write_record([
            "open_time",
            "open",
            "high",
            "low",
            "close",
            "volume",
            "close_time",
            "quote_volume",
            "trades",
            "taker_buy_base",
            "taker_buy_quote",
        ])?;
        for b in bars {
            w.write_record(row(tf, b))?;
        }
        w.flush()?;
    }
    fs::rename(&tmp, path)?;
    Ok(())
}

fn row(tf: Tf, b: &Bar) -> Vec<String> {
    vec![
        b.open_time.to_string(),
        b.open.to_string(),
        b.high.to_string(),
        b.low.to_string(),
        b.close.to_string(),
        b.volume.to_string(),
        (b.open_time + tf.ms() - 1).to_string(),
        b.quote_volume.to_string(),
        b.trades.to_string(),
        b.taker_buy_base.to_string(),
        "".into(),
    ]
}

#[derive(Clone, Debug)]
pub struct FundingRow {
    pub time: Ms,
    pub rate: f64,
}

pub fn load_funding(path: &Path) -> anyhow::Result<Vec<FundingRow>> {
    let mut rdr = csv::ReaderBuilder::new()
        .has_headers(true)
        .from_path(path)?;
    let mut out = Vec::new();
    for rec in rdr.records() {
        let r = rec?;
        out.push(FundingRow {
            time: r.get(0).unwrap_or("0").parse()?,
            rate: r.get(1).unwrap_or("0").parse()?,
        });
    }
    out.sort_by_key(|x| x.time);
    Ok(out)
}

pub fn kline_path(data_dir: &Path, sym: &str, tf: Tf) -> PathBuf {
    data_dir
        .join("klines")
        .join(format!("{sym}_{}.csv", tf.binance()))
}

pub fn funding_path(data_dir: &Path, sym: &str) -> PathBuf {
    data_dir.join("funding").join(format!("{sym}.csv"))
}

/// Recorder: append bar TERTUTUP ke CSV (buat header bila file baru). Bar duplikat/lebih lama diabaikan.
pub struct Recorder {
    dir: PathBuf,
    last: HashMap<(String, Tf), Ms>,
}

impl Recorder {
    pub fn new(data_dir: &Path) -> Recorder {
        Recorder {
            dir: data_dir.to_path_buf(),
            last: Default::default(),
        }
    }

    /// Kembalikan `open_time` bar tertutup terakhir untuk `(sym, tf)`, termasuk membaca CSV lama
    /// saat proses baru dinyalakan agar reconnect/restart tidak menulis duplikat atau kehilangan titik awal backfill.
    pub fn last_open_time(&mut self, sym: &str, tf: Tf) -> Option<Ms> {
        let key = (sym.to_string(), tf);
        if let Some(&t) = self.last.get(&key) {
            return Some(t);
        }
        let p = kline_path(&self.dir, sym, tf);
        if p.exists() {
            if let Ok(rows) = load_klines(&p) {
                if let Some(last_bar) = rows.last() {
                    self.last.insert(key, last_bar.open_time);
                    return Some(last_bar.open_time);
                }
            }
        }
        None
    }

    pub fn record(&mut self, sym: &str, tf: Tf, b: &Bar) -> anyhow::Result<bool> {
        if self
            .last_open_time(sym, tf)
            .is_some_and(|t| b.open_time <= t)
        {
            return Ok(false);
        }
        let p = kline_path(&self.dir, sym, tf);
        if let Some(d) = p.parent() {
            fs::create_dir_all(d)?;
        }
        let new = !p.exists();
        let mut f = OpenOptions::new().create(true).append(true).open(&p)?;
        if new {
            writeln!(
                f,
                "open_time,open,high,low,close,volume,close_time,quote_volume,trades,taker_buy_base,taker_buy_quote"
            )?;
        }
        writeln!(f, "{}", row(tf, b).join(","))?;
        self.last.insert((sym.to_string(), tf), b.open_time);
        Ok(true)
    }

    /// Isi celah bar tertutup sejak `last_open_time` saat `WsEvent::Reconnected` diterima.
    /// Memakai closure pengambil bar agar dapat diuji deterministik tanpa jaringan.
    pub async fn backfill_gaps_with<F, Fut>(
        &mut self,
        targets: &[(String, Tf)],
        now: Ms,
        mut fetch: F,
    ) -> anyhow::Result<usize>
    where
        F: FnMut(String, Tf, Ms, Ms, Ms) -> Fut,
        Fut: Future<Output = anyhow::Result<Vec<Bar>>>,
    {
        let mut added = 0usize;
        for (sym, tf) in targets {
            let Some(last_open) = self.last_open_time(sym, *tf) else {
                continue;
            };
            // Bar berikutnya baru tertutup penuh pada `last_open + 2 * tf.ms()`.
            if now < last_open + 2 * tf.ms() {
                continue;
            }
            let mut bars = fetch(sym.clone(), *tf, last_open + tf.ms(), now, now).await?;
            bars.sort_by_key(|b| b.open_time);
            for b in &bars {
                if b.available_at(*tf) <= now && self.record(sym, *tf, b)? {
                    added += 1;
                }
            }
        }
        Ok(added)
    }

    /// Backfill celah bar tertutup memakai REST publik Binance (`PublicClient::klines`).
    pub async fn backfill_gaps(
        &mut self,
        pc: &crate::PublicClient,
        targets: &[(String, Tf)],
        now: Ms,
    ) -> anyhow::Result<usize> {
        self.backfill_gaps_with(targets, now, |sym, tf, start, end, as_of| async move {
            pc.klines(&sym, tf, start, end, as_of).await
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_recorder() {
        let dir = std::env::temp_dir().join(format!("apex_csv_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let b = Bar {
            open_time: 0,
            open: 1.0,
            high: 2.0,
            low: 0.5,
            close: 1.5,
            volume: 10.0,
            quote_volume: 15.0,
            trades: 7,
            taker_buy_base: 6.0,
        };
        let p = kline_path(&dir, "XUSDT", Tf::M15);
        save_klines(&p, Tf::M15, &[b]).unwrap();
        let l = load_klines(&p).unwrap();
        assert_eq!(l, vec![b]);
        let mut r = Recorder::new(&dir.join("rec"));
        assert!(r.record("XUSDT", Tf::M15, &b).unwrap());
        assert!(!r.record("XUSDT", Tf::M15, &b).unwrap());
        let mut b2 = b;
        b2.open_time = 900_000;
        assert!(r.record("XUSDT", Tf::M15, &b2).unwrap());
        assert_eq!(
            load_klines(&kline_path(&dir.join("rec"), "XUSDT", Tf::M15))
                .unwrap()
                .len(),
            2
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn backfill_mengisi_tepat_n_bar_tanpa_duplikat() {
        let dir = std::env::temp_dir().join(format!("apex_backfill_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let step = Tf::M15.ms();
        let mk_bar = |idx: i64| Bar {
            open_time: idx * step,
            open: 100.0 + idx as f64,
            high: 101.0 + idx as f64,
            low: 99.0 + idx as f64,
            close: 100.5 + idx as f64,
            volume: 50.0,
            quote_volume: 5_000.0,
            trades: 10,
            taker_buy_base: 25.0,
        };

        let mut rec = Recorder::new(&dir);
        // Bar awal t=0 tercatat sebelum putus koneksi.
        assert!(rec.record("BTCUSDT", Tf::M15, &mk_bar(0)).unwrap());

        // Putus koneksi melewati N=4 bar tertutup (idx 1, 2, 3, 4); bar idx 5 masih berjalan (belum tutup).
        let now = 5 * step + 10_000;
        let targets = vec![("BTCUSDT".to_string(), Tf::M15)];
        let filled = rec
            .backfill_gaps_with(&targets, now, |_sym, _tf, _s, _e, _now| async move {
                // Sertakan duplikat bar 0 dan bar 5 yang belum tutup untuk menguji dedup & kausalitas.
                Ok((0..=5).map(mk_bar).collect())
            })
            .await
            .unwrap();
        assert_eq!(filled, 4);

        // Bila Reconnected berikutnya datang tanpa bar tertinggal atau WS mengirim ulang idx 4, tidak ada duplikat.
        assert!(!rec.record("BTCUSDT", Tf::M15, &mk_bar(4)).unwrap());
        let filled_again = rec
            .backfill_gaps_with(&targets, now, |_sym, _tf, _s, _e, _now| async move {
                Ok((0..=5).map(mk_bar).collect())
            })
            .await
            .unwrap();
        assert_eq!(filled_again, 0);

        let saved = load_klines(&kline_path(&dir, "BTCUSDT", Tf::M15)).unwrap();
        let times: Vec<Ms> = saved.iter().map(|b| b.open_time).collect();
        assert_eq!(times, vec![0, step, 2 * step, 3 * step, 4 * step]);
        let _ = fs::remove_dir_all(dir);
    }
}
