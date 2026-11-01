//! Backtest genomes (`src/genome.rs`) on edge-lab's 15m history, for the evolving pit.
//!
//! Reads genomes as JSON lines on stdin and writes one JSON line per genome on stdout: its id
//! and every trade as `[open ms, R]`, R after the typical spread. The judging (in-sample,
//! holdout, the deflated Sharpe over every genome ever tried) is `evolve/evolve.py`'s job;
//! this is the fast part. Features are cached per symbol across the batch, so a thousand
//! genomes cost seconds.
//!
//!   genome_backtest < genomes.jsonl > results.jsonl
//!   genome_backtest --cache /path/to/edge-lab/data_cache < ...
//!   genome_backtest --edge-dir evolve/edge_lab/signals < ...   (edge-lab genomes' entries)
//!   genome_backtest --detail < ...      (each trade as [open ms, R, symbol, side, exit])
//!   genome_backtest --slip-ticks 1 < ...  (every fill, in and out, that many ticks worse)
//!   genome_backtest --spreads DIR < ...  (each bar's own spread, DIR/spreads_<SYM>.json {ms: px})
//!
//! The eight desk symbols edge-lab caches at 15m: Bitcoin has no 15m history, so genomes do
//! not trade it.

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    if let Err(e) = bt::run() {
        eprintln!("genome_backtest: {e}");
        std::process::exit(1);
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod bt {
    use std::collections::HashMap;
    use std::io::{BufRead, Write};

    use opencry_visual::genome::{backtest_slipped, Bar, Features, Genome};

    pub const SYMBOLS: [&str; 8] = ["XAUUSD", "XAGUSD", "GBPUSD", "EURUSD", "UK100", "US500", "NAS100", "US30"];
    /// `$OPENCRY_EDGE_CACHE`, else `$EDGE_LAB_DIR/data_cache`, else `../edge-lab/data_cache` beside this repo.
    fn default_cache() -> String {
        opencry_visual::edge_cache_dir()
    }

    fn load(cache: &str, sym: &str) -> Result<Vec<Bar>, String> {
        let path = format!("{cache}/ctrader_{sym}_SB_15m.json");
        let text = std::fs::read_to_string(&path).map_err(|e| format!("{path}: {e}"))?;
        let rows: Vec<Vec<f64>> = serde_json::from_str(&text).map_err(|e| format!("{path}: {e}"))?;
        let mut bars: Vec<Bar> = rows.iter().filter(|r| r.len() >= 5).map(|r| Bar { t: r[0] as i64, o: r[1], h: r[2], l: r[3], c: r[4] }).collect();
        bars.sort_by_key(|b| b.t);
        bars.dedup_by_key(|b| b.t);
        Ok(bars)
    }

    /// Load an edge-lab signal's entries (`<key>@<tf>.json`, `{symbol: [[t, d], ...]}`) onto
    /// each symbol's bars, once per batch. One with no file trades nothing.
    fn load_entries(dir: Option<&str>, key: &str, data: &mut HashMap<String, Features>) {
        if data.values().next().is_some_and(|f| f.has_entries(key)) {
            return;
        }
        let parsed: HashMap<String, Vec<(i64, i8)>> = dir
            .and_then(|d| std::fs::read_to_string(format!("{d}/{key}.json")).ok())
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        for (sym, f) in data.iter_mut() {
            let mut at = HashMap::new();
            for &(t, d) in parsed.get(sym).map(|v| v.as_slice()).unwrap_or(&[]) {
                if let Ok(i) = f.bars.binary_search_by_key(&t, |b| b.t) {
                    at.insert(i, d);
                }
            }
            f.set_entries(key, at);
        }
    }

    pub fn run() -> Result<(), String> {
        let args: Vec<String> = std::env::args().collect();
        let cache = args.iter().position(|a| a == "--cache").and_then(|i| args.get(i + 1)).cloned().unwrap_or_else(default_cache);
        let edge_dir = args.iter().position(|a| a == "--edge-dir").and_then(|i| args.get(i + 1)).cloned();
        let detail = args.iter().any(|a| a == "--detail");
        let slip: f64 = args.iter().position(|a| a == "--slip-ticks").and_then(|i| args.get(i + 1)).and_then(|v| v.parse().ok()).unwrap_or(0.0);
        let mut data: HashMap<String, Features> = HashMap::new();
        let spreads = args.iter().position(|a| a == "--spreads").and_then(|i| args.get(i + 1)).cloned();
        for s in SYMBOLS {
            let mut f = Features::new(load(&cache, s)?);
            if let Some(dir) = &spreads {
                if let Ok(text) = std::fs::read_to_string(format!("{dir}/spreads_{s}.json")) {
                    let by: HashMap<String, f64> = serde_json::from_str(&text).map_err(|e| format!("spreads_{s}: {e}"))?;
                    f.spreads = Some(f.bars.iter().map(|b| by.get(&b.t.to_string()).copied().unwrap_or(f64::NAN)).collect());
                }
            }
            data.insert(s.to_string(), f);
        }
        let stdin = std::io::stdin();
        let stdout = std::io::stdout();
        let mut out = stdout.lock();
        for line in stdin.lock().lines() {
            let line = line.map_err(|e| e.to_string())?;
            if line.trim().is_empty() {
                continue;
            }
            let g: Genome = match serde_json::from_str(&line) {
                Ok(g) => g,
                Err(e) => {
                    writeln!(out, "{}", serde_json::json!({"error": e.to_string(), "line": line})).map_err(|e| e.to_string())?;
                    continue;
                }
            };
            if let Some(id) = g.signal.edge_lab_id() {
                load_entries(edge_dir.as_deref(), &id, &mut data);
            }
            let trades = backtest_slipped(&g, &mut data, slip);
            let rows: Vec<serde_json::Value> = trades
                .iter()
                .map(|t| {
                    let r = (t.r * 10000.0).round() / 10000.0;
                    if detail { serde_json::json!([t.t, r, t.symbol, t.side, t.exit]) } else { serde_json::json!([t.t, r]) }
                })
                .collect();
            writeln!(out, "{}", serde_json::json!({"id": g.id, "trades": rows})).map_err(|e| e.to_string())?;
        }
        Ok(())
    }
}
