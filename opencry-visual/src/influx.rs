//! InfluxDB, read-only, for the offline tools: the market tape's ticks (`tick_data`) and the
//! bridge's bars (`market_data`, measurement `trendbar`, 1m since 2025-09 for the spread-bet
//! symbols). Credentials come from INFLUX_URL / INFLUX_TOKEN / INFLUX_ORG, else INFLUXDB_URL /
//! INFLUXDB_TOKEN2 / INFLUXDB_ORG in the repo-root `.env` (or `OPENCRY_ENV_FILE`); the token is read
//! into memory and never printed. Native only (blocking HTTP).
//!
//! `jev_score` and `replay` carry older private copies of these helpers; new tools use this.

#![cfg(not(target_arch = "wasm32"))]

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use crate::candles::Candle;

pub struct Influx {
    pub url: String,
    pub token: String,
    pub org: String,
}

/// One KEY=value from the repo-root `.env` (or `OPENCRY_ENV_FILE`), without printing anything.
pub fn dotenv_value(key: &str) -> Option<String> {
    let path = std::env::var("OPENCRY_ENV_FILE").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.env"));
    let text = fs::read_to_string(path).ok()?;
    text.lines().find_map(|l| l.trim().strip_prefix(&format!("{key}=")).map(|v| v.trim().trim_matches(|c| c == '"' || c == '\'').to_string()))
}

pub fn config() -> Result<Influx, String> {
    let get = |a: &str, b: &str| std::env::var(a).ok().filter(|v| !v.is_empty()).or_else(|| dotenv_value(b));
    Ok(Influx {
        url: get("INFLUX_URL", "INFLUXDB_URL").unwrap_or_else(|| "http://localhost:8086".into()),
        token: get("INFLUX_TOKEN", "INFLUXDB_TOKEN2").ok_or("no InfluxDB token: set INFLUX_TOKEN or INFLUXDB_TOKEN2 in .env (OPENCRY_ENV_FILE)")?,
        org: get("INFLUX_ORG", "INFLUXDB_ORG").ok_or("no InfluxDB org: set INFLUX_ORG or INFLUXDB_ORG in .env (OPENCRY_ENV_FILE)")?,
    })
}

/// Run a Flux query, returning the annotated CSV.
pub fn flux(cfg: &Influx, query: &str) -> Result<String, String> {
    let mut req = ehttp::Request::post(format!("{}/api/v2/query?org={}", cfg.url, cfg.org), query.as_bytes().to_vec());
    req.headers = ehttp::Headers::new(&[
        ("Authorization", &format!("Token {}", cfg.token)),
        ("Content-Type", "application/vnd.flux"),
        ("Accept", "application/csv"),
    ]);
    let res = ehttp::fetch_blocking(&req).map_err(|e| format!("influx: {e}"))?;
    if !res.ok {
        return Err(format!("influx HTTP {}: {}", res.status, res.text().unwrap_or("").chars().take(200).collect::<String>()));
    }
    Ok(res.text().unwrap_or("").to_string())
}

/// Influx CSV (one or more tables, each with its own header row) into rows keyed by column.
pub fn parse_csv(text: &str) -> Vec<HashMap<String, String>> {
    let mut rows = Vec::new();
    let mut header: Vec<String> = Vec::new();
    for line in text.lines() {
        let line = line.trim_end_matches('\r');
        if line.is_empty() {
            header.clear();
            continue;
        }
        if line.starts_with('#') {
            continue;
        }
        let cells: Vec<&str> = line.split(',').collect();
        if header.is_empty() || cells.get(1) == Some(&"result") {
            header = cells.iter().map(|c| c.to_string()).collect();
            continue;
        }
        let row: HashMap<String, String> = header.iter().cloned().zip(cells.iter().map(|c| c.to_string())).collect();
        rows.push(row);
    }
    rows
}

/// RFC3339 with any fraction length (Influx writes nanoseconds) to Unix ms.
pub fn time_ms(s: &str) -> Option<i64> {
    let s = s.trim_end_matches('Z');
    let (whole, frac) = match s.split_once('.') {
        Some((w, f)) => (w, f),
        None => (s, ""),
    };
    let mut frac3: String = frac.chars().take(3).collect();
    while frac3.len() < 3 {
        frac3.push('0');
    }
    crate::candles::parse_iso_ms(&format!("{whole}.{frac3}Z"))
}

/// The bridge's bars for one `_SB` symbol and timeframe over [from_ms, to_ms), oldest first,
/// one bar per open time (the measurement carries a `host` tag, so a bar can be stored twice).
/// Fetched in week-long chunks so no single response is huge.
pub fn load_trendbars(cfg: &Influx, sb_symbol: &str, timeframe: &str, from_ms: i64, to_ms: i64, mut progress: impl FnMut(usize, usize)) -> Result<Vec<Candle>, String> {
    const CHUNK_MS: i64 = 7 * 86_400_000;
    let mut out: Vec<Candle> = Vec::new();
    let chunks = ((to_ms - from_ms) as f64 / CHUNK_MS as f64).ceil().max(0.0) as usize;
    let mut a = from_ms;
    let mut k = 0;
    while a < to_ms {
        let b = (a + CHUNK_MS).min(to_ms);
        let q = format!(
            r#"from(bucket:"market_data") |> range(start: time(v: {}), stop: time(v: {})) |> filter(fn:(r) => r._measurement == "trendbar" and r.symbol == "{sb_symbol}" and r.timeframe == "{timeframe}") |> drop(columns:["host","_start","_stop","_measurement","symbol","timeframe"]) |> pivot(rowKey:["_time"], columnKey:["_field"], valueColumn:"_value") |> group() |> sort(columns:["_time"])"#,
            a * 1_000_000,
            b * 1_000_000
        );
        for r in parse_csv(&flux(cfg, &q)?) {
            let f = |k: &str| r.get(k).and_then(|v| v.parse::<f32>().ok());
            let (Some(t), Some(o), Some(h), Some(l), Some(c)) = (r.get("_time").and_then(|v| time_ms(v)), f("open"), f("high"), f("low"), f("close")) else { continue };
            let bar = Candle { time_ms: t, open: o, high: h, low: l, close: c, volume: f("volume").unwrap_or(0.0), delta: 0.0, sweep: 0.0 };
            match out.last_mut() {
                Some(last) if last.time_ms == t => *last = bar,
                _ => out.push(bar),
            }
        }
        k += 1;
        progress(k, chunks);
        a = b;
    }
    out.sort_by_key(|c| c.time_ms);
    out.dedup_by_key(|c| c.time_ms);
    Ok(out)
}
