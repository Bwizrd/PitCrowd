//! Headless replay: the floor's own feed, candle, level and paper-book code run over the ticks
//! and books the market tape saved to InfluxDB, as fast as the machine goes, with the
//! scoreboard printed at the end and every trade written out.
//!
//!   cargo run --release --bin replay -- --last 6h
//!   cargo run --release --bin replay -- --from 2026-10-21T07:00:00Z --to 2026-10-21T16:30:00Z \
//!        --symbols XAGUSD,EURUSD --jev recorded --out replay_out/london
//!   cargo run --release --bin replay -- --last 3d --sessions london,ny      (only those UTC hours)
//!   cargo run --release --bin replay -- --report-only replay_out/london     (rebuild the reports)
//!
//! Output directory: trades.jsonl (one closed trade per line), summary.json (config + per-trader
//! numbers), report.txt (what is printed: per-trader, edge, symbol, session, day tables and
//! equity sparklines) and report.html (the same with SVG equity curves, readable on a phone).
//! The statistics live in `opencry_visual::report`.
//!
//! --sessions keeps only the ticks, books and reads that fall inside the named windows (Asia
//! 00-07, London 07-12, Overlap 12-16:30, NY 16:30-21, Late 21-24 UTC) across the whole span.
//! A position still open when a window ends is carried into the next kept window, where the
//! first tick marks it; the paper book's own end-of-day close still applies.
//!
//! Jev modes:
//!   none      the three Jev traders stay flat; levels traders and the control run
//!   rule      the floor-rule fallback stands in for Jev every --jev-every seconds (free)
//!   recorded  reads from jev_relay/log/inferences.jsonl at the times they were made
//!   live      re-inference: calls Jev through the relay every --jev-every replay seconds per
//!             pit (costs tokens; one call per 2 s of real time)
//!
//! InfluxDB: INFLUX_URL / INFLUX_TOKEN / INFLUX_ORG, else INFLUXDB_URL / INFLUXDB_TOKEN2 /
//! INFLUXDB_ORG from the repo-root `.env` or `OPENCRY_ENV_FILE` (never printed).

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    if let Err(e) = replay::run() {
        eprintln!("replay: {e}");
        std::process::exit(1);
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod replay {
    use std::collections::HashMap;
    use std::fs;
    use std::io::Write;
    use std::path::PathBuf;
    use std::time::{Duration, Instant};

    use opencry_visual::candles::{Candle, CandleStore, Timeframe, parse_iso_ms, wall_clock_ms};
    use opencry_visual::jev::{
        DeterministicModel, JevDesk, JevResponse, MarketInterpretation, ModelError, ReadSource,
        interpret, relay_url, system_one_request, system_one_result,
    };
    use opencry_visual::levels::{LevelStore, tape_symbol};
    use opencry_visual::live_feed::{FeedMode, LiveFeed, PitFeedStatus, TapeDepth, TapeTick};
    use opencry_visual::market::Instrument;
    use opencry_visual::paper::{Archetype, PaperBook, READ_FRESH_SECS, Trade, VIEW_BARS, View};
    use opencry_visual::pit_engine::tick_size;
    use opencry_visual::report::{Report, ReportTrade, Session, parse_jsonl};
    use opencry_visual::tape_chart::BOARD_BARS;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum JevMode {
        None,
        Rule,
        Recorded,
        Live,
    }

    struct Args {
        from_ms: i64,
        to_ms: i64,
        symbols: Vec<Instrument>,
        jev: JevMode,
        jev_every: f64,
        out: PathBuf,
        inferences: PathBuf,
        seed: u64,
        no_depth: bool,
        /// Empty = the whole span.
        sessions: Vec<Session>,
    }

    struct Influx {
        url: String,
        token: String,
        org: String,
        bucket: String,
    }

    fn arg(name: &str) -> Option<String> {
        let args: Vec<String> = std::env::args().collect();
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1).cloned())
    }

    fn flag(name: &str) -> bool {
        std::env::args().any(|a| a == name)
    }

    fn parse_duration_secs(s: &str) -> Result<i64, String> {
        let (num, unit) = s.split_at(s.len() - 1);
        let n: i64 = num.parse().map_err(|_| format!("bad duration {s}"))?;
        Ok(match unit {
            "s" => n,
            "m" => n * 60,
            "h" => n * 3600,
            "d" => n * 86_400,
            _ => return Err(format!("bad duration unit in {s} (s, m, h, d)")),
        })
    }

    fn parse_args() -> Result<Args, String> {
        let now = wall_clock_ms();
        let (from_ms, to_ms) = if let Some(last) = arg("--last") {
            (now - parse_duration_secs(&last)? * 1000, now)
        } else {
            let from = arg("--from").ok_or("need --from <RFC3339> or --last <6h>")?;
            let to = arg("--to").unwrap_or_else(|| "now".into());
            let from_ms = parse_iso_ms(&from).ok_or(format!("bad --from {from}"))?;
            let to_ms = if to == "now" {
                now
            } else {
                parse_iso_ms(&to).ok_or(format!("bad --to {to}"))?
            };
            (from_ms, to_ms)
        };
        if to_ms <= from_ms {
            return Err("--to must be after --from".into());
        }
        let symbols = match arg("--symbols").as_deref() {
            None | Some("all") => Instrument::ALL.to_vec(),
            Some(list) => list
                .split(',')
                .map(|s| {
                    let want = s.trim().to_uppercase().replace('/', "");
                    Instrument::ALL
                        .iter()
                        .copied()
                        .find(|i| tape_symbol(*i) == want)
                        .ok_or(format!("unknown symbol {s}"))
                })
                .collect::<Result<Vec<_>, _>>()?,
        };
        let jev = match arg("--jev").as_deref().unwrap_or("none") {
            "none" => JevMode::None,
            "rule" => JevMode::Rule,
            "recorded" => JevMode::Recorded,
            "live" => JevMode::Live,
            other => return Err(format!("--jev {other}: none | rule | recorded | live")),
        };
        let jev_every = arg("--jev-every")
            .and_then(|v| v.parse().ok())
            .unwrap_or(60.0f64)
            .max(10.0);
        let stamp = format!(
            "{}_{}",
            fmt_ms(from_ms).replace([':', '-'], ""),
            fmt_ms(to_ms).replace([':', '-'], "")
        );
        let out = arg("--out").map(PathBuf::from).unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("replay_out")
                .join(stamp)
        });
        let inferences = arg("--inferences")
            .map(PathBuf::from)
            .or_else(|| find_up("jev_relay/log/inferences.jsonl"))
            .unwrap_or_else(|| {
                PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../jev_relay/log/inferences.jsonl")
            });
        let seed = arg("--seed").and_then(|v| v.parse().ok()).unwrap_or(42);
        let sessions = match arg("--sessions") {
            Some(list) => Session::parse_list(&list)?,
            None => Vec::new(),
        };
        Ok(Args {
            from_ms,
            to_ms,
            symbols,
            jev,
            jev_every,
            out,
            inferences,
            seed,
            no_depth: flag("--no-depth"),
            sessions,
        })
    }

    /// First ancestor of this crate (itself included) where `rel` exists. A git worktree under
    /// `.claude/worktrees/<x>/` finds the main checkout's untracked files (the relay log) and
    /// the repo-root `.env` this way, the same as the main checkout does.
    fn find_up(rel: &str) -> Option<PathBuf> {
        let mut dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        loop {
            let candidate = dir.join(rel);
            if candidate.exists() {
                return Some(candidate);
            }
            if !dir.pop() {
                return None;
            }
        }
    }

    /// Read one KEY=value from the repo-root `.env` (or `OPENCRY_ENV_FILE`) without printing anything.
    fn dotenv_value(key: &str) -> Option<String> {
        let path = std::env::var("OPENCRY_ENV_FILE")
            .map(PathBuf::from)
            .ok()
            .or_else(|| find_up(".env"))?;
        let text = fs::read_to_string(path).ok()?;
        text.lines().find_map(|l| {
            l.trim()
                .strip_prefix(&format!("{key}="))
                .map(|v| v.trim().trim_matches(|c| c == '"' || c == '\'').to_string())
        })
    }

    fn influx_config() -> Result<Influx, String> {
        let get = |a: &str, b: &str| {
            std::env::var(a)
                .ok()
                .filter(|v| !v.is_empty())
                .or_else(|| dotenv_value(b))
        };
        Ok(Influx {
            url: get("INFLUX_URL", "INFLUXDB_URL")
                .unwrap_or_else(|| "http://localhost:8086".into()),
            token: get("INFLUX_TOKEN", "INFLUXDB_TOKEN2").ok_or(
                "no InfluxDB token: set INFLUX_TOKEN or INFLUXDB_TOKEN2 in .env (OPENCRY_ENV_FILE)",
            )?,
            org: get("INFLUX_ORG", "INFLUXDB_ORG").ok_or(
                "no InfluxDB org: set INFLUX_ORG or INFLUXDB_ORG in .env (OPENCRY_ENV_FILE)",
            )?,
            bucket: std::env::var("INFLUX_TICK_BUCKET").unwrap_or_else(|_| "tick_data".into()),
        })
    }

    fn flux(cfg: &Influx, query: &str) -> Result<String, String> {
        let mut req = ehttp::Request::post(
            format!("{}/api/v2/query?org={}", cfg.url, cfg.org),
            query.as_bytes().to_vec(),
        );
        req.headers = ehttp::Headers::new(&[
            ("Authorization", &format!("Token {}", cfg.token)),
            ("Content-Type", "application/vnd.flux"),
            ("Accept", "application/csv"),
        ]);
        let res = ehttp::fetch_blocking(&req).map_err(|e| format!("influx: {e}"))?;
        if !res.ok {
            return Err(format!(
                "influx HTTP {}: {}",
                res.status,
                res.text()
                    .unwrap_or("")
                    .chars()
                    .take(200)
                    .collect::<String>()
            ));
        }
        Ok(res.text().unwrap_or("").to_string())
    }

    /// Influx CSV (one or more tables, each with its own header row) into rows keyed by column.
    fn parse_csv(text: &str) -> Vec<HashMap<String, String>> {
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
            let row: HashMap<String, String> = header
                .iter()
                .cloned()
                .zip(cells.iter().map(|c| c.to_string()))
                .collect();
            rows.push(row);
        }
        rows
    }

    /// RFC3339 with any fraction length (Influx writes nanoseconds) to Unix ms.
    fn time_ms(s: &str) -> Option<i64> {
        let s = s.trim_end_matches('Z');
        let (whole, frac) = match s.split_once('.') {
            Some((w, f)) => (w, f),
            None => (s, ""),
        };
        let mut frac3: String = frac.chars().take(3).collect();
        while frac3.len() < 3 {
            frac3.push('0');
        }
        parse_iso_ms(&format!("{whole}.{frac3}Z"))
    }

    fn fmt_ms(ms: i64) -> String {
        let secs = ms.div_euclid(1000);
        let days = secs.div_euclid(86_400);
        let day = secs.rem_euclid(86_400);
        // civil from days (Howard Hinnant)
        let z = days + 719_468;
        let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
        let doe = z - era * 146_097;
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let y = yoe + era * 400;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let d = doy - (153 * mp + 2) / 5 + 1;
        let m = if mp < 10 { mp + 3 } else { mp - 9 };
        let y = if m <= 2 { y + 1 } else { y };
        format!(
            "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
            day / 3600,
            (day % 3600) / 60,
            day % 60
        )
    }

    // Short-lived and sorted by time; boxing the big variant buys nothing.
    #[allow(clippy::large_enum_variant)]
    enum Event {
        Tick(Instrument, TapeTick),
        Depth(Instrument, TapeDepth),
        Read(Instrument, MarketInterpretation),
    }

    fn load_ticks(
        cfg: &Influx,
        inst: Instrument,
        from_ms: i64,
        to_ms: i64,
    ) -> Result<Vec<(i64, Event)>, String> {
        let sym = tape_symbol(inst);
        let q = format!(
            r#"from(bucket:"{}") |> range(start: time(v: {}), stop: time(v: {})) |> filter(fn:(r) => r._measurement == "tick" and r.symbol == "{sym}") |> pivot(rowKey:["_time"], columnKey:["_field"], valueColumn:"_value") |> keep(columns:["_time","bid","ask"]) |> sort(columns:["_time"])"#,
            cfg.bucket,
            from_ms * 1_000_000,
            to_ms * 1_000_000
        );
        let id = opencry_visual::live_feed::symbol_id_for(inst).unwrap_or(0);
        let rows = parse_csv(&flux(cfg, &q)?);
        let mut out = Vec::with_capacity(rows.len());
        for r in rows {
            let (Some(t), Some(bid), Some(ask)) = (
                r.get("_time").and_then(|v| time_ms(v)),
                r.get("bid").and_then(|v| v.parse::<f64>().ok()),
                r.get("ask").and_then(|v| v.parse::<f64>().ok()),
            ) else {
                continue;
            };
            out.push((
                t,
                Event::Tick(
                    inst,
                    TapeTick {
                        symbol_id: id,
                        symbol: sym.clone(),
                        t,
                        bid,
                        ask,
                        spread: ask - bid,
                    },
                ),
            ));
        }
        Ok(out)
    }

    fn load_depth(
        cfg: &Influx,
        inst: Instrument,
        from_ms: i64,
        to_ms: i64,
    ) -> Result<Vec<(i64, Event)>, String> {
        let sym = tape_symbol(inst);
        let q = format!(
            r#"from(bucket:"{}") |> range(start: time(v: {}), stop: time(v: {})) |> filter(fn:(r) => r._measurement == "depth" and r.symbol == "{sym}") |> pivot(rowKey:["_time"], columnKey:["_field"], valueColumn:"_value") |> sort(columns:["_time"])"#,
            cfg.bucket,
            from_ms * 1_000_000,
            to_ms * 1_000_000
        );
        let id = opencry_visual::live_feed::symbol_id_for(inst).unwrap_or(0);
        let rows = parse_csv(&flux(cfg, &q)?);
        let mut out = Vec::with_capacity(rows.len());
        for r in rows {
            let Some(t) = r.get("_time").and_then(|v| time_ms(v)) else {
                continue;
            };
            let side = |p: &str| -> Vec<(f32, f32)> {
                (1..=5)
                    .filter_map(|i| {
                        let px = r.get(&format!("{p}_px_{i}"))?.parse::<f32>().ok()?;
                        let sz = r.get(&format!("{p}_sz_{i}"))?.parse::<f32>().ok()?;
                        Some((px, sz))
                    })
                    .collect()
            };
            out.push((
                t,
                Event::Depth(
                    inst,
                    TapeDepth {
                        symbol_id: id,
                        t,
                        bids: side("bid"),
                        asks: side("ask"),
                    },
                ),
            ));
        }
        Ok(out)
    }

    /// Recorded inferences: the relay logs `{ts, request.state, response}`; the state's first
    /// line names the instrument.
    fn load_recorded(
        path: &PathBuf,
        symbols: &[Instrument],
        from_ms: i64,
        to_ms: i64,
    ) -> Result<Vec<(i64, Event)>, String> {
        let text =
            fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        let mut out = Vec::new();
        for line in text.lines() {
            let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
                continue;
            };
            let Some(ts) = v.get("ts").and_then(|t| t.as_i64()) else {
                continue;
            };
            if ts < from_ms || ts > to_ms {
                continue;
            }
            let Some(state) = v.pointer("/request/state").and_then(|s| s.as_str()) else {
                continue;
            };
            let Some(first) = state.lines().next() else {
                continue;
            };
            let Some(sym) = first.split('(').nth(1).and_then(|s| s.split(')').next()) else {
                continue;
            };
            let Some(inst) = symbols.iter().copied().find(|i| i.symbol() == sym) else {
                continue;
            };
            let Some(resp) = v.get("response") else {
                continue;
            };
            let Ok(parsed) = serde_json::from_value::<JevResponse>(resp.clone()) else {
                continue;
            };
            let hash = v
                .get("observation_hash")
                .and_then(|h| h.as_str())
                .unwrap_or("")
                .to_string();
            let latency = v.get("latency_ms").and_then(|l| l.as_u64()).unwrap_or(0);
            if let Ok(read) = interpret(&parsed, &hash, latency, ReadSource::Jev) {
                out.push((ts, Event::Read(inst, read)));
            }
        }
        Ok(out)
    }

    #[derive(serde::Serialize)]
    struct TraderSummary {
        trader: &'static str,
        archetype: Archetype,
        trades: usize,
        net_r: f32,
        net_ticks: f32,
        wins: u32,
        losses: u32,
        win_rate: f32,
        avg_win_r: f32,
        avg_loss_r: f32,
        profit_factor: f32,
        max_drawdown_r: f32,
        /// Spread paid at entry as a fraction of the stop, summed: the cost of doing business in R.
        spread_paid_r: f32,
        by_symbol_r: HashMap<String, f32>,
    }

    fn summarise(book: &PaperBook) -> Vec<TraderSummary> {
        let mut out: Vec<TraderSummary> = book
            .traders
            .iter()
            .map(|t| {
                let wins: Vec<f32> = t
                    .trades
                    .iter()
                    .map(|x| x.pnl_r)
                    .filter(|p| *p > 0.0)
                    .collect();
                let losses: Vec<f32> = t
                    .trades
                    .iter()
                    .map(|x| x.pnl_r)
                    .filter(|p| *p <= 0.0)
                    .collect();
                let gw: f32 = wins.iter().sum();
                let gl: f32 = losses.iter().sum::<f32>().abs();
                let mut by_symbol_r: HashMap<String, f32> = HashMap::new();
                for x in &t.trades {
                    *by_symbol_r.entry(x.symbol.to_string()).or_default() += x.pnl_r;
                }
                TraderSummary {
                    trader: t.name(),
                    archetype: t.archetype,
                    trades: t.trades.len(),
                    net_r: t.pnl_r,
                    net_ticks: t.pnl_ticks,
                    wins: t.wins,
                    losses: t.losses,
                    win_rate: t.win_rate(),
                    avg_win_r: if wins.is_empty() {
                        0.0
                    } else {
                        gw / wins.len() as f32
                    },
                    avg_loss_r: if losses.is_empty() {
                        0.0
                    } else {
                        -gl / losses.len() as f32
                    },
                    profit_factor: if gl > 0.0 {
                        gw / gl
                    } else if gw > 0.0 {
                        f32::INFINITY
                    } else {
                        0.0
                    },
                    max_drawdown_r: t.max_drawdown_r,
                    spread_paid_r: t
                        .trades
                        .iter()
                        .map(|x| x.spread_ticks_in / x.stop_ticks.max(1.0))
                        .sum(),
                    by_symbol_r,
                }
            })
            .collect();
        out.sort_by(|a, b| b.net_r.partial_cmp(&a.net_r).unwrap());
        out
    }

    /// `--report-only <dir>`: rebuild report.txt / report.html from an existing trades.jsonl.
    /// The window comes from summary.json when it is there, else from the trades themselves.
    fn report_only(dir: PathBuf) -> Result<(), String> {
        let path = dir.join("trades.jsonl");
        let text = fs::read_to_string(&path)
            .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        let (trades, bad) = parse_jsonl(&text);
        if bad > 0 {
            eprintln!("replay: {bad} unparseable lines in {}", path.display());
        }
        if trades.is_empty() {
            return Err(format!("no trades in {}", path.display()));
        }
        let config = fs::read_to_string(dir.join("summary.json"))
            .ok()
            .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
            .and_then(|v| v.get("config").cloned());
        let get_ms = |k: &str| {
            config
                .as_ref()
                .and_then(|c| c.get(k))
                .and_then(|v| v.as_str())
                .and_then(parse_iso_ms)
        };
        let from_ms =
            get_ms("from").unwrap_or_else(|| trades.iter().map(|t| t.opened_ms).min().unwrap());
        let to_ms =
            get_ms("to").unwrap_or_else(|| trades.iter().map(|t| t.closed_ms).max().unwrap());
        let seed = config
            .as_ref()
            .and_then(|c| c.get("seed"))
            .and_then(|v| v.as_u64())
            .unwrap_or(42);
        let jev = config
            .as_ref()
            .and_then(|c| c.get("jev"))
            .and_then(|v| v.as_str())
            .unwrap_or("?")
            .to_string();
        let title = format!(
            "OpenCry paper book replay  jev {jev}  (rebuilt from {}/trades.jsonl)",
            dir.file_name()
                .map(std::ffi::OsStr::to_string_lossy)
                .unwrap_or_default()
        );
        let report = Report::build(&title, trades, from_ms, to_ms, Archetype::ALL.len(), seed);
        write_reports(&dir, &report)?;
        print!("{}", report.text());
        println!("written: {}", dir.display());
        Ok(())
    }

    fn write_reports(dir: &std::path::Path, report: &Report) -> Result<(), String> {
        fs::write(dir.join("report.txt"), report.text()).map_err(|e| e.to_string())?;
        fs::write(dir.join("report.html"), report.html()).map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn run() -> Result<(), String> {
        if let Some(dir) = arg("--report-only") {
            return report_only(PathBuf::from(dir));
        }
        let args = parse_args()?;
        let cfg = influx_config()?;
        eprintln!(
            "replay: {} -> {}  symbols {}  jev {:?} every {:.0}s  out {}",
            fmt_ms(args.from_ms),
            fmt_ms(args.to_ms),
            args.symbols
                .iter()
                .map(|i| tape_symbol(*i))
                .collect::<Vec<_>>()
                .join(","),
            args.jev,
            args.jev_every,
            args.out.display()
        );

        // Load.
        let started = Instant::now();
        let mut events: Vec<(i64, Event)> = Vec::new();
        for inst in &args.symbols {
            let ticks = load_ticks(&cfg, *inst, args.from_ms, args.to_ms)?;
            let depth = if args.no_depth {
                Vec::new()
            } else {
                load_depth(&cfg, *inst, args.from_ms, args.to_ms)?
            };
            eprintln!(
                "replay: {} ticks {} books {}",
                tape_symbol(*inst),
                ticks.len(),
                depth.len()
            );
            events.extend(ticks);
            events.extend(depth);
        }
        if args.jev == JevMode::Recorded {
            let reads = load_recorded(&args.inferences, &args.symbols, args.from_ms, args.to_ms)?;
            eprintln!(
                "replay: {} recorded Jev reads from {}",
                reads.len(),
                args.inferences.display()
            );
            events.extend(reads);
        }
        if !args.sessions.is_empty() {
            let before = events.len();
            events.retain(|(t, _)| args.sessions.contains(&Session::of_ms(*t)));
            eprintln!(
                "replay: sessions {} keep {} of {} events",
                args.sessions
                    .iter()
                    .map(|s| s.label())
                    .collect::<Vec<_>>()
                    .join(","),
                events.len(),
                before
            );
        }
        if events.is_empty() {
            return Err(
                "no ticks in that range (is the market tape running and writing to tick_data?)"
                    .into(),
            );
        }
        // Stable by time; ticks before books before reads at the same instant.
        events.sort_by_key(|(t, e)| {
            (
                *t,
                match e {
                    Event::Tick(..) => 0,
                    Event::Depth(..) => 1,
                    Event::Read(..) => 2,
                },
            )
        });
        eprintln!(
            "replay: {} events loaded in {:.1}s",
            events.len(),
            started.elapsed().as_secs_f32()
        );

        // State: exactly what the app holds.
        let mut feed = LiveFeed::default();
        let mut store = CandleStore::default();
        let mut levels = LevelStore::default();
        let mut desk = JevDesk::with_model(Box::new(DeterministicModel));
        desk.set_session_start_ms(args.from_ms);
        let mut book = PaperBook::default();
        book.log_to_relay = false;
        book.show = false;
        book.reseed(args.seed);
        let t0 = args.from_ms;
        let clock = |t: i64| (t - t0) as f64 / 1000.0 + 1.0;
        feed.note_message(0.5);

        let questions = desk.questions();
        let relay = relay_url();
        let mut next_read: HashMap<Instrument, f64> = HashMap::new();
        let mut last_live_call = Instant::now() - Duration::from_secs(5);
        let mut prev_bid: HashMap<Instrument, f32> = HashMap::new();
        let (mut n_ticks, mut n_depth, mut n_reads, mut n_calls, mut n_call_errors) =
            (0u64, 0u64, 0u64, 0u64, 0u64);
        let total = events.len();
        let mut next_progress = total / 10;
        let fresh_secs = if args.jev == JevMode::None {
            0.0
        } else {
            READ_FRESH_SECS.max(args.jev_every + 5.0)
        };

        for (idx, (t, ev)) in events.into_iter().enumerate() {
            let now = clock(t);
            match ev {
                Event::Depth(inst, d) => {
                    n_depth += 1;
                    feed.ingest_depth(inst, &d);
                }
                Event::Read(inst, mut r) => {
                    n_reads += 1;
                    r.at = now;
                    desk.set_read(inst, r);
                }
                Event::Tick(inst, tick) => {
                    n_ticks += 1;
                    let prev = feed.quote(inst).map(|q| q.price);
                    feed.ingest_tick(inst, &tick, now);
                    let tick_sz = tick_size(inst);
                    let d_ticks = prev
                        .map(|p| ((tick.bid - p) / tick_sz as f64) as f32)
                        .unwrap_or(0.0);
                    store.apply_live_tick(inst, tick.t, tick.bid as f32, d_ticks);

                    // Levels from the 1m bars, like the app.
                    let series = store.series(inst, Timeframe::M1, FeedMode::Live);
                    let n = series.bars.len();
                    let bars60: Vec<Candle> = series
                        .bars
                        .iter()
                        .skip(n.saturating_sub(BOARD_BARS))
                        .copied()
                        .collect();
                    let rev = series.revision;
                    levels.refresh_trading(inst, &bars60, rev, now);

                    // Jev, on the chosen schedule.
                    if matches!(args.jev, JevMode::Rule | JevMode::Live)
                        && now >= *next_read.get(&inst).unwrap_or(&0.0)
                    {
                        next_read.insert(inst, now + args.jev_every);
                        if let Some(obs) = desk.observe(
                            inst,
                            &store,
                            Timeframe::M1,
                            &feed,
                            levels.for_trading(inst),
                            now,
                            t,
                        ) {
                            match args.jev {
                                JevMode::Rule => {
                                    let mut r = DeterministicModel::read(&obs);
                                    r.at = now;
                                    desk.set_read(inst, r);
                                    n_reads += 1;
                                }
                                JevMode::Live => {
                                    // The relay allows one call per 2 s; pace real time to it.
                                    let since = last_live_call.elapsed();
                                    if since < Duration::from_millis(2100) {
                                        std::thread::sleep(Duration::from_millis(2100) - since);
                                    }
                                    let hash = obs.hash();
                                    let req = system_one_request(&relay, &obs, &questions)
                                        .map_err(|e| e.short())?;
                                    let sent = Instant::now();
                                    let res = ehttp::fetch_blocking(&req);
                                    last_live_call = Instant::now();
                                    n_calls += 1;
                                    match system_one_result(
                                        res,
                                        &hash,
                                        sent.elapsed().as_millis() as u64,
                                    ) {
                                        Ok(mut r) => {
                                            r.at = now;
                                            desk.set_read(inst, r);
                                            n_reads += 1;
                                        }
                                        Err(ModelError::RateLimited(ms)) => {
                                            n_call_errors += 1;
                                            std::thread::sleep(Duration::from_millis(ms + 100));
                                            next_read.insert(inst, now); // try again on the next tick
                                        }
                                        Err(e) => {
                                            n_call_errors += 1;
                                            eprintln!("replay: Jev call failed: {}", e.short());
                                        }
                                    }
                                }
                                _ => {}
                            }
                        }
                    }

                    // The book.
                    let PitFeedStatus::Live { rate, .. } = feed.pit_status(inst, now) else {
                        continue;
                    };
                    let q = feed.quote(inst).expect("quote after ingest");
                    // The book sees the day's bars (session VWAP, extremes, opening ranges), like the app.
                    let bars_view: Vec<Candle> = series
                        .bars
                        .iter()
                        .skip(n.saturating_sub(VIEW_BARS))
                        .copied()
                        .collect();
                    let read = desk.read(inst).filter(|r| match args.jev {
                        JevMode::None => false,
                        JevMode::Rule => now - r.at <= fresh_secs,
                        JevMode::Recorded | JevMode::Live => {
                            r.source == ReadSource::Jev && now - r.at <= fresh_secs
                        }
                    });
                    let bid = q.price as f32;
                    let view = View {
                        inst,
                        bid,
                        ask: q.ask as f32,
                        tick: tick_sz,
                        spread_ticks: (q.spread / tick_sz as f64) as f32,
                        now,
                        wall_ms: t,
                        bars: &bars_view,
                        flow: q.flow(now),
                        rate,
                        levels: levels.for_trading(inst),
                        read,
                        prev_bid: *prev_bid.get(&inst).unwrap_or(&bid),
                    };
                    book.step_pit(&view, false);
                    prev_bid.insert(inst, bid);
                }
            }
            if idx + 1 >= next_progress && total >= 10 {
                eprintln!(
                    "replay: {:>3}%  {}  trades so far {}",
                    (idx + 1) * 100 / total,
                    fmt_ms(t),
                    book.traders.iter().map(|x| x.trades.len()).sum::<usize>()
                );
                next_progress += total / 10;
            }
        }

        // Anything still open is marked at the last touch as a session close.
        for inst in &args.symbols {
            if let Some(q) = feed.quote(*inst) {
                let bid = q.price as f32;
                let mut view = View {
                    inst: *inst,
                    bid,
                    ask: q.ask as f32,
                    tick: tick_size(*inst),
                    spread_ticks: (q.spread / tick_size(*inst) as f64) as f32,
                    now: clock(args.to_ms) + 1.0,
                    wall_ms: args.to_ms,
                    bars: &[],
                    flow: Default::default(),
                    rate: 0.0,
                    levels: &[],
                    read: None,
                    prev_bid: bid,
                };
                // Force the session-close path regardless of the clock.
                view.wall_ms = (args.to_ms / 86_400_000) * 86_400_000 + (20 * 60 + 56) * 60_000;
                book.enabled = false;
                book.step_pit(&view, false);
            }
        }

        // Report.
        let summary = summarise(&book);
        fs::create_dir_all(&args.out).map_err(|e| e.to_string())?;
        let mut trades: Vec<&Trade> = book.traders.iter().flat_map(|t| t.trades.iter()).collect();
        trades.sort_by_key(|t| t.opened_ms);
        let mut f = fs::File::create(args.out.join("trades.jsonl")).map_err(|e| e.to_string())?;
        for t in &trades {
            writeln!(f, "{}", serde_json::to_string(t).unwrap()).map_err(|e| e.to_string())?;
        }
        let sessions_label = if args.sessions.is_empty() {
            "all hours".to_string()
        } else {
            args.sessions
                .iter()
                .map(|s| s.label())
                .collect::<Vec<_>>()
                .join("+")
        };
        let config = serde_json::json!({
            "from": fmt_ms(args.from_ms), "to": fmt_ms(args.to_ms), "sessions": sessions_label,
            "symbols": args.symbols.iter().map(|i| tape_symbol(*i)).collect::<Vec<_>>(),
            "jev": format!("{:?}", args.jev), "jev_every_secs": args.jev_every, "seed": args.seed,
            "ticks": n_ticks, "books": n_depth, "reads": n_reads, "jev_calls": n_calls, "jev_call_errors": n_call_errors,
            "elapsed_secs": started.elapsed().as_secs_f32(),
        });

        // The report: statistics, breakdowns, curves and verdicts, as text and HTML.
        let title = format!(
            "OpenCry paper book replay  jev {:?}  {}  {}",
            args.jev,
            sessions_label,
            args.symbols
                .iter()
                .map(|i| tape_symbol(*i))
                .collect::<Vec<_>>()
                .join(",")
        );
        let report = Report::build(
            &title,
            trades.iter().map(|t| ReportTrade::from(*t)).collect(),
            args.from_ms,
            args.to_ms,
            Archetype::ALL.len(),
            args.seed,
        );
        write_reports(&args.out, &report)?;
        fs::write(
            args.out.join("summary.json"),
            serde_json::to_string_pretty(&serde_json::json!({"config": config, "traders": summary, "report": report.traders})).unwrap(),
        )
        .map_err(|e| e.to_string())?;

        println!();
        println!(
            "REPLAY {} -> {}   ticks {}  books {}  reads {}  ({:.1}s)",
            fmt_ms(args.from_ms),
            fmt_ms(args.to_ms),
            n_ticks,
            n_depth,
            n_reads,
            started.elapsed().as_secs_f32()
        );
        print!("{}", report.text());
        println!("written: {}", args.out.display());
        Ok(())
    }
}
