//! floor_tape: the market tape the OpenCry floor reads, with no broker and no account.
//!
//! It speaks the tape protocol (docs/DATA_SOURCES.md) on one port, 8083 by default:
//!
//!   ws://host:8083/          hello, then every `tick` and `depth` as it happens
//!   ws://host:8083/l2        Level 2 for BTC/USD in Kraken's websocket v2 messages
//!   GET  /health             what is being served, per symbol
//!   GET  /bars?symbol=XAUUSD&timeframe=1m&range=-6h[&delta=1]
//!   GET  /book?symbol=XAUUSD the five-level ladder now
//!   GET  /orderbook?symbol=  a retail order book (synthetic symbols only)
//!   POST /ingest             your own ticks and books (with --ingest)
//!   POST /ingest/bars        your own one-minute history (with --ingest)
//!
//! Where prices come from, per symbol:
//!   synthetic (default)  a seeded random market, the same on every machine (src/sim.rs)
//!   --csv DIR            your recordings, one <SYMBOL>.csv each, replayed on today's clock
//!   --ingest SYMS|all    whatever you POST: a live feed from any language (adapters/)
//!
//!   floor_tape                                   # all nine symbols synthetic
//!   floor_tape --csv data/recorded --speed 10    # replay CSVs ten times faster
//!   floor_tape --ingest BTCUSD                   # BTC from an adapter, the rest synthetic
//!   floor_tape --write-cache data/cache          # 15m history for the backtests, then exit
//!   floor_tape --write-csv data/sample --days 2  # sample CSVs in the replay format, then exit

mod csv_source;
mod l2;
mod rng;
mod sim;
mod store;
mod symbols;

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

use rng::Rng;
use sim::{MINUTE_MS, Sim, T0_MS, Tick, minute_of};
use store::{Bar, Series, parse_range, parse_timeframe};
use symbols::{SYMBOLS, by_name, round_to};

const BTC: usize = 6;
const LEVELS: usize = 5;
const USAGE: &str = "usage: floor_tape [--port 8083] [--bind 0.0.0.0] [--seed 42] [--history 30d]
                  [--csv DIR [--speed 1] [--warmup 1d] [--no-loop]]
                  [--ingest SYM,SYM|all] [--only SYM,SYM]
       floor_tape --write-cache DIR [--from 2025-01-01]
       floor_tape --write-csv DIR [--days 2]";

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Arguments
// ---------------------------------------------------------------------------

struct Args {
    port: u16,
    bind: String,
    seed: u64,
    history_ms: i64,
    csv: Option<PathBuf>,
    speed: f64,
    warmup_ms: i64,
    looping: bool,
    ingest: Vec<usize>,
    only: Option<Vec<usize>>,
    write_cache: Option<PathBuf>,
    from_ms: i64,
    write_csv: Option<PathBuf>,
    days: i64,
}

fn symbol_list(s: &str) -> Result<Vec<usize>, String> {
    if s.eq_ignore_ascii_case("all") {
        return Ok((0..SYMBOLS.len()).collect());
    }
    s.split(',')
        .filter(|x| !x.trim().is_empty())
        .map(|x| {
            by_name(x.trim()).ok_or(format!(
                "unknown symbol {x} (names: {})",
                SYMBOLS.map(|s| s.name).join(", ")
            ))
        })
        .collect()
}

fn parse_args() -> Result<Args, String> {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let mut a = Args {
        port: std::env::var("FLOOR_TAPE_PORT")
            .ok()
            .and_then(|p| p.parse().ok())
            .unwrap_or(8083),
        bind: "0.0.0.0".into(),
        seed: 42,
        history_ms: 30 * 1440 * MINUTE_MS,
        csv: None,
        speed: 1.0,
        warmup_ms: 1440 * MINUTE_MS,
        looping: true,
        ingest: Vec::new(),
        only: None,
        write_cache: None,
        from_ms: T0_MS,
        write_csv: None,
        days: 2,
    };
    let mut i = 0;
    let val = |i: &mut usize| -> Result<String, String> {
        *i += 1;
        argv.get(*i)
            .cloned()
            .ok_or(format!("{} needs a value", argv[*i - 1]))
    };
    let dur = |s: String| {
        parse_range(&format!("-{s}")).ok_or(format!("bad duration {s} (like 6h, 2d, 1w)"))
    };
    while i < argv.len() {
        match argv[i].as_str() {
            "--port" => a.port = val(&mut i)?.parse().map_err(|_| "bad --port")?,
            "--bind" => a.bind = val(&mut i)?,
            "--seed" => a.seed = val(&mut i)?.parse().map_err(|_| "bad --seed")?,
            "--history" => a.history_ms = dur(val(&mut i)?)?,
            "--csv" => a.csv = Some(PathBuf::from(val(&mut i)?)),
            "--speed" => a.speed = val(&mut i)?.parse().map_err(|_| "bad --speed")?,
            "--warmup" => a.warmup_ms = dur(val(&mut i)?)?,
            "--no-loop" => a.looping = false,
            "--ingest" => a.ingest = symbol_list(&val(&mut i)?)?,
            "--only" => a.only = Some(symbol_list(&val(&mut i)?)?),
            "--write-cache" => a.write_cache = Some(PathBuf::from(val(&mut i)?)),
            "--from" => {
                a.from_ms = csv_source::parse_time(&format!("{}T00:00:00Z", val(&mut i)?))
                    .ok_or("bad --from (YYYY-MM-DD)")?
            }
            "--write-csv" => a.write_csv = Some(PathBuf::from(val(&mut i)?)),
            "--days" => a.days = val(&mut i)?.parse().map_err(|_| "bad --days")?,
            "-h" | "--help" => return Err(USAGE.into()),
            other => return Err(format!("unknown argument {other}\n{USAGE}")),
        }
        i += 1;
    }
    Ok(a)
}

// ---------------------------------------------------------------------------
// Shared state
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Owner {
    Synthetic,
    Csv,
    Ingest,
    Off,
}

impl Owner {
    fn label(self) -> &'static str {
        match self {
            Owner::Synthetic => "synthetic",
            Owner::Csv => "csv",
            Owner::Ingest => "ingest",
            Owner::Off => "off",
        }
    }
}

type Ladder = (Vec<(f64, f64)>, Vec<(f64, f64)>);

struct Tape {
    owner: Vec<Owner>,
    series: Vec<Series>,
    quote: Vec<Option<Tick>>,
    ticks: Vec<u64>,
    ladder: Vec<Option<Ladder>>,
    l2: Option<l2::L2>,
    history_ms: i64,
    seed: u64,
    started: i64,
    ingested: u64,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Channel {
    Tape,
    L2,
}

struct Client {
    channel: Channel,
    tx: mpsc::Sender<Arc<String>>,
}

#[derive(Clone)]
struct Hub {
    tape: Arc<Mutex<Tape>>,
    clients: Arc<Mutex<Vec<Client>>>,
}

impl Hub {
    fn broadcast(&self, channel: Channel, msg: String) {
        let msg = Arc::new(msg);
        self.clients
            .lock()
            .unwrap()
            .retain(|c| c.channel != channel || c.tx.send(msg.clone()).is_ok());
    }

    /// One tick in: into the bars, out to every client, and on to the Level 2 book for BTC.
    fn publish(&self, sym: usize, tick: Tick, depth: Option<Ladder>, rng: &mut Rng) {
        let spec = &SYMBOLS[sym];
        let (depth, l2_msgs) = {
            let mut t = self.tape.lock().unwrap();
            if !t.series[sym].add_tick(&tick) {
                return;
            }
            t.quote[sym] = Some(tick);
            t.ticks[sym] += 1;
            let depth = depth.or_else(|| {
                (t.owner[sym] != Owner::Ingest).then(|| synthetic_ladder(sym, &tick, rng))
            });
            if let Some(d) = &depth {
                t.ladder[sym] = Some(d.clone());
            }
            let l2_msgs = if sym == BTC {
                t.l2.as_mut()
                    .map(|b| b.on_quote(tick.bid, tick.ask, tick.t))
                    .unwrap_or_default()
            } else {
                Vec::new()
            };
            (depth, l2_msgs)
        };
        let spread = round_to(tick.ask - tick.bid, spec.digits + 1);
        self.broadcast(Channel::Tape, json!({"type": "tick", "data": {"symbolId": spec.id, "symbol": spec.name, "t": tick.t, "bid": tick.bid, "ask": tick.ask, "spread": spread}}).to_string());
        if let Some((bids, asks)) = depth {
            let row = |v: &[(f64, f64)]| v.iter().map(|(p, s)| json!([p, s])).collect::<Vec<_>>();
            self.broadcast(Channel::Tape, json!({"type": "depth", "data": {"symbolId": spec.id, "symbol": spec.name, "t": tick.t, "bids": row(&bids), "asks": row(&asks)}}).to_string());
        }
        for m in l2_msgs {
            self.broadcast(Channel::L2, m);
        }
    }
}

/// A five-level broker ladder around the quote, sizes invented.
fn synthetic_ladder(sym: usize, tick: &Tick, rng: &mut Rng) -> Ladder {
    let spec = &SYMBOLS[sym];
    let unit = 10f64.powi(-(spec.digits as i32));
    let step = (spec.spread * 0.5).max(unit);
    let lot = match spec.name {
        "EURUSD" | "GBPUSD" => 500_000.0,
        "XAUUSD" => 50.0,
        "XAGUSD" => 2_000.0,
        "BTCUSD" => 2.0,
        _ => 20.0,
    };
    let mut side = |px: f64, dir: f64| -> Vec<(f64, f64)> {
        (0..LEVELS)
            .map(|i| {
                (
                    round_to(px + dir * step * i as f64, spec.digits),
                    (lot * (0.4 + 1.6 * rng.unit()) * (1.0 + i as f64 * 0.5)).round(),
                )
            })
            .collect()
    };
    let bids = side(tick.bid, -1.0);
    let asks = side(tick.ask, 1.0);
    (bids, asks)
}

/// A retail order book in OANDA's shape (`[price, long %, short %]` buckets within 4% of the
/// price) for the FX pairs and metals, refreshed every twenty minutes like OANDA's. Orders
/// cluster at round numbers. Synthetic symbols only: it is invented.
fn synthetic_orderbook(seed: u64, sym: usize, price: f64, now: i64) -> Option<Value> {
    let spec = &SYMBOLS[sym];
    let (width, round) = match spec.name {
        "EURUSD" | "GBPUSD" => (0.0005, 0.0050),
        "XAUUSD" => (1.0, 25.0),
        "XAGUSD" => (0.05, 0.5),
        _ => return None,
    };
    let window = now.div_euclid(20 * MINUTE_MS);
    let mut rng = Rng::keyed(seed, 100 + sym as u64, window as u64);
    let lo = ((price * 0.96) / width).ceil() as i64;
    let hi = ((price * 1.04) / width).floor() as i64;
    let buckets: Vec<Value> = (lo..=hi)
        .map(|k| {
            let px = round_to(k as f64 * width, spec.digits);
            let near_round = ((px / round).round() * round - px).abs() < width * 0.5;
            let boost = if near_round {
                3.0 + 3.0 * rng.unit()
            } else {
                1.0
            };
            let below = px < price;
            let (l, s) = (0.05 + 0.15 * rng.unit(), 0.05 + 0.15 * rng.unit());
            // Buy limits rest below the price and sell limits above, so the round numbers there swell.
            let (l, s) = if below {
                (l * boost, s)
            } else {
                (l, s * boost)
            };
            json!([
                px,
                (l * 1000.0f64).round() / 1000.0,
                (s * 1000.0f64).round() / 1000.0
            ])
        })
        .collect();
    let at = window * 20 * MINUTE_MS;
    Some(
        json!({"symbol": spec.name, "instrument": spec.name, "time": format!("{}", at), "price": price, "bucketWidth": width, "buckets": buckets, "positions": [], "fetchedAt": at, "source": "synthetic"}),
    )
}

// ---------------------------------------------------------------------------
// HTTP and websockets
// ---------------------------------------------------------------------------

struct Request {
    method: String,
    path: String,
    query: HashMap<String, String>,
    upgrade: bool,
    head_len: usize,
    content_length: usize,
}

fn decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < b.len() => {
                // From the bytes, not `&s[..]`: a `%` before a multi-byte character must not panic.
                let hex = std::str::from_utf8(&b[i + 1..i + 3]).ok();
                if let Some(v) = hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                    out.push(v);
                    i += 2;
                } else {
                    out.push(b'%');
                }
            }
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Look at the request head without consuming it, so a websocket upgrade can be handed to
/// tungstenite whole.
fn peek_request(stream: &TcpStream) -> Option<Request> {
    let mut buf = vec![0u8; 16 * 1024];
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    let n = loop {
        let n = stream.peek(&mut buf).ok()?;
        if n == 0 {
            return None;
        }
        if buf[..n].windows(4).any(|w| w == b"\r\n\r\n")
            || n == buf.len()
            || std::time::Instant::now() > deadline
        {
            break n;
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    let head_len = buf[..n].windows(4).position(|w| w == b"\r\n\r\n")? + 4;
    let head = String::from_utf8_lossy(&buf[..head_len]);
    let mut lines = head.split("\r\n");
    let mut first = lines.next()?.split(' ');
    let method = first.next()?.to_string();
    let target = first.next()?.to_string();
    let (path, qs) = target.split_once('?').unwrap_or((&target, ""));
    let query = qs
        .split('&')
        .filter_map(|kv| kv.split_once('=').map(|(k, v)| (decode(k), decode(v))))
        .collect();
    let mut upgrade = false;
    let mut content_length = 0;
    for l in lines {
        if let Some((k, v)) = l.split_once(':') {
            let (k, v) = (k.trim().to_ascii_lowercase(), v.trim());
            if k == "upgrade" && v.eq_ignore_ascii_case("websocket") {
                upgrade = true;
            }
            if k == "content-length" {
                content_length = v.parse().unwrap_or(0);
            }
        }
    }
    Some(Request {
        method,
        path: path.to_string(),
        query,
        upgrade,
        head_len,
        content_length,
    })
}

fn respond(stream: &mut TcpStream, status: u16, body: &str) {
    let reason = match status {
        200 => "OK",
        202 => "Accepted",
        204 => "No Content",
        400 => "Bad Request",
        404 => "Not Found",
        409 => "Conflict",
        413 => "Payload Too Large",
        _ => "Service Unavailable",
    };
    let _ = write!(
        stream,
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: GET, POST, OPTIONS\r\nAccess-Control-Allow-Headers: Content-Type\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
}

fn serve_ws(hub: Hub, stream: TcpStream, channel: Channel) {
    let Ok(mut ws) = tungstenite::accept(stream) else {
        return;
    };
    let (tx, rx) = mpsc::channel::<Arc<String>>();
    let first = {
        let t = hub.tape.lock().unwrap();
        match channel {
            Channel::Tape => Some(json!({"type": "hello", "source": "floor_tape", "symbols": SYMBOLS.iter().enumerate().filter(|(i, _)| t.owner[*i] != Owner::Off).map(|(i, s)| json!({"symbol": s.name, "id": s.id, "source": t.owner[i].label()})).collect::<Vec<_>>(), "levels": LEVELS}).to_string()),
            Channel::L2 => t.l2.as_ref().map(|b| b.snapshot(now_ms())),
        }
    };
    // Registered under the lock that publishes, so nothing falls between the snapshot and the stream.
    hub.clients.lock().unwrap().push(Client { channel, tx });
    if let Some(m) = first
        && ws.send(tungstenite::Message::text(m)).is_err()
    {
        return;
    }
    let _ = ws
        .get_ref()
        .set_read_timeout(Some(Duration::from_millis(20)));
    loop {
        let mut wrote = false;
        while let Ok(m) = rx.try_recv() {
            if ws.write(tungstenite::Message::text(m.as_str())).is_err() {
                return;
            }
            wrote = true;
        }
        if wrote && ws.flush().is_err() {
            return;
        }
        match ws.read() {
            Ok(tungstenite::Message::Close(_)) => return,
            Ok(_) => {}
            Err(tungstenite::Error::Io(e))
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(_) => return,
        }
    }
}

fn handle(hub: Hub, mut stream: TcpStream) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let Some(req) = peek_request(&stream) else {
        return;
    };
    if req.upgrade {
        let channel = if req.path.trim_end_matches('/') == "/l2" {
            Channel::L2
        } else {
            Channel::Tape
        };
        let _ = stream.set_read_timeout(None);
        return serve_ws(hub, stream, channel);
    }
    let mut head = vec![0u8; req.head_len];
    if stream.read_exact(&mut head).is_err() {
        return;
    }
    if req.method == "OPTIONS" {
        return respond(&mut stream, 204, "");
    }
    if req.content_length > 64 * 1024 * 1024 {
        return respond(&mut stream, 413, r#"{"error":"body over 64 MB"}"#);
    }
    let mut body = vec![0u8; req.content_length];
    if stream.read_exact(&mut body).is_err() {
        return;
    }
    let (status, out) = route(&hub, &req, &body);
    respond(&mut stream, status, &out);
}

fn route(hub: &Hub, req: &Request, body: &[u8]) -> (u16, String) {
    let now = now_ms();
    let symbol = || req.query.get("symbol").and_then(|s| by_name(s));
    match (req.method.as_str(), req.path.as_str()) {
        ("GET", "/") | ("GET", "/health") => {
            let t = hub.tape.lock().unwrap();
            let symbols: Vec<Value> = SYMBOLS
                .iter()
                .enumerate()
                .map(|(i, s)| {
                    let q = t.quote[i];
                    json!({"symbol": s.name, "id": s.id, "source": t.owner[i].label(), "bid": q.map(|q| q.bid), "ask": q.map(|q| q.ask), "ticks": t.ticks[i], "lastTickAgeMs": q.map(|q| now - q.t), "bars1m": t.series[i].bars.len()})
                })
                .collect();
            let body = json!({"ok": true, "connected": true, "tape": "floor_tape", "uptimeSec": (now - t.started) / 1000, "clients": hub.clients.lock().unwrap().len(), "seed": t.seed, "ingested": t.ingested, "l2": t.l2.is_some(), "symbols": symbols});
            (200, body.to_string())
        }
        ("GET", "/bars") => {
            let Some(sym) = symbol() else {
                return (404, json!({"error": format!("unknown symbol {}", req.query.get("symbol").cloned().unwrap_or_default())}).to_string());
            };
            let tf_s = req
                .query
                .get("timeframe")
                .cloned()
                .unwrap_or_else(|| "1m".into());
            let range_s = req
                .query
                .get("range")
                .cloned()
                .unwrap_or_else(|| "-6h".into());
            let Some(tf) = parse_timeframe(&tf_s) else {
                return (
                    400,
                    r#"{"error":"timeframe must look like 1m, 5m, 15m, 1h, 1d"}"#.into(),
                );
            };
            let Some(range) = parse_range(&range_s) else {
                return (
                    400,
                    r#"{"error":"range must look like -6h, -2d, -1w"}"#.into(),
                );
            };
            let delta = req.query.get("delta").map(|d| d == "1").unwrap_or(false);
            let t = hub.tape.lock().unwrap();
            let data = t.series[sym].query(now - range, tf, delta);
            (200, json!({"symbol": SYMBOLS[sym].name, "timeframe": tf_s, "range": range_s, "source": t.owner[sym].label(), "delta": delta, "count": data.len(), "data": data}).to_string())
        }
        ("GET", "/book") => {
            let Some(sym) = symbol() else {
                return (404, r#"{"error":"unknown symbol"}"#.into());
            };
            let t = hub.tape.lock().unwrap();
            let (Some(q), Some((b, a))) = (t.quote[sym], t.ladder[sym].clone()) else {
                return (
                    404,
                    json!({"error": format!("no book yet for {}", SYMBOLS[sym].name)}).to_string(),
                );
            };
            (200, json!({"symbol": SYMBOLS[sym].name, "bid": q.bid, "ask": q.ask, "t": q.t, "bids": b, "asks": a}).to_string())
        }
        ("GET", "/orderbook") => {
            let Some(sym) = symbol() else {
                return (404, r#"{"error":"unknown symbol"}"#.into());
            };
            let t = hub.tape.lock().unwrap();
            if t.owner[sym] != Owner::Synthetic {
                return (503, json!({"error": format!("no retail order book source for {} ({} data)", SYMBOLS[sym].name, t.owner[sym].label())}).to_string());
            }
            let Some(q) = t.quote[sym] else {
                return (202, json!({"pending": true}).to_string());
            };
            match synthetic_orderbook(t.seed, sym, q.bid, now) {
                Some(b) => (200, b.to_string()),
                None => (
                    404,
                    json!({"error": format!("no order book for {}", SYMBOLS[sym].name)})
                        .to_string(),
                ),
            }
        }
        ("POST", "/ingest") => ingest(hub, body, false),
        ("POST", "/ingest/bars") => ingest(hub, body, true),
        _ => (404, r#"{"error":"not found"}"#.into()),
    }
}

/// Rows from a body: a JSON array, one object, or one object per line.
fn rows_of(body: &[u8]) -> Result<Vec<Value>, String> {
    let text = std::str::from_utf8(body).map_err(|_| "body is not UTF-8")?;
    let trimmed = text.trim();
    if trimmed.starts_with('[') {
        return serde_json::from_str::<Vec<Value>>(trimmed).map_err(|e| e.to_string());
    }
    trimmed
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str::<Value>(l).map_err(|e| e.to_string()))
        .collect()
}

fn time_of(v: Option<&Value>) -> Option<i64> {
    match v? {
        Value::Number(n) => csv_source::parse_time(&n.to_string()),
        Value::String(s) => csv_source::parse_time(s),
        _ => None,
    }
}

fn ingest(hub: &Hub, body: &[u8], bars: bool) -> (u16, String) {
    let rows = match rows_of(body) {
        Ok(r) => r,
        Err(e) => return (400, json!({"error": format!("bad JSON: {e}")}).to_string()),
    };
    let owner_ok = |sym: usize| hub.tape.lock().unwrap().owner[sym] == Owner::Ingest;
    let mut rng = Rng::new(now_ms() as u64);
    let mut taken = 0;
    let mut errors: Vec<String> = Vec::new();
    for (i, r) in rows.iter().enumerate() {
        let Some(sym) = r.get("symbol").and_then(|s| s.as_str()).and_then(by_name) else {
            errors.push(format!(
                "row {i}: unknown or missing symbol (names: {})",
                SYMBOLS.map(|s| s.name).join(", ")
            ));
            continue;
        };
        if !owner_ok(sym) {
            errors.push(format!(
                "row {i}: {} is not an ingest symbol; start floor_tape with --ingest {}",
                SYMBOLS[sym].name, SYMBOLS[sym].name
            ));
            continue;
        }
        let spec = &SYMBOLS[sym];
        if bars {
            // {"symbol": .., "bars": [{timestamp, open, high, low, close, volume} | [t, o, h, l, c, v]]}
            let list = r
                .get("bars")
                .and_then(|b| b.as_array())
                .cloned()
                .unwrap_or_default();
            let parsed: Vec<Bar> = list
                .iter()
                .filter_map(|b| {
                    let (t, o, h, l, c, v) = if let Some(a) = b.as_array() {
                        (
                            time_of(a.first())?,
                            a.get(1)?.as_f64()?,
                            a.get(2)?.as_f64()?,
                            a.get(3)?.as_f64()?,
                            a.get(4)?.as_f64()?,
                            a.get(5).and_then(|v| v.as_f64()).unwrap_or(0.0),
                        )
                    } else {
                        (
                            time_of(b.get("timestamp").or(b.get("t")).or(b.get("time")))?,
                            b.get("open")?.as_f64()?,
                            b.get("high")?.as_f64()?,
                            b.get("low")?.as_f64()?,
                            b.get("close")?.as_f64()?,
                            b.get("volume").and_then(|v| v.as_f64()).unwrap_or(0.0),
                        )
                    };
                    Some(Bar {
                        t: store::minute_start(t),
                        o,
                        h,
                        l,
                        c,
                        n: v as u32,
                        up: 0,
                        down: 0,
                        spread_sum: 0.0,
                        spread_n: 0,
                    })
                })
                .collect();
            taken += parsed.len();
            if parsed.len() < list.len() {
                errors.push(format!(
                    "row {i}: {} of {} bars unreadable",
                    list.len() - parsed.len(),
                    list.len()
                ));
            }
            let mut t = hub.tape.lock().unwrap();
            t.series[sym].add_bars(parsed);
            continue;
        }
        if r.get("type").and_then(|t| t.as_str()) == Some("depth") {
            let side = |k: &str| -> Vec<(f64, f64)> {
                r.get(k)
                    .and_then(|x| x.as_array())
                    .into_iter()
                    .flatten()
                    .filter_map(|l| Some((l.get(0)?.as_f64()?, l.get(1)?.as_f64()?)))
                    .take(LEVELS)
                    .collect()
            };
            let (bids, asks) = (side("bids"), side("asks"));
            let t = time_of(r.get("t")).unwrap_or_else(now_ms);
            hub.tape.lock().unwrap().ladder[sym] = Some((bids.clone(), asks.clone()));
            let row = |v: &[(f64, f64)]| v.iter().map(|(p, s)| json!([p, s])).collect::<Vec<_>>();
            hub.broadcast(Channel::Tape, json!({"type": "depth", "data": {"symbolId": spec.id, "symbol": spec.name, "t": t, "bids": row(&bids), "asks": row(&asks)}}).to_string());
            taken += 1;
            continue;
        }
        let Some(bid) = r.get("bid").and_then(|b| b.as_f64()) else {
            errors.push(format!("row {i}: no bid"));
            continue;
        };
        let unit = 10f64.powi(-(spec.digits as i32));
        let ask = r
            .get("ask")
            .and_then(|a| a.as_f64())
            .filter(|a| *a > bid)
            .unwrap_or_else(|| round_to(bid + spec.spread.max(unit), spec.digits));
        let t = time_of(r.get("t").or(r.get("time"))).unwrap_or_else(now_ms);
        hub.publish(sym, Tick { t, bid, ask }, None, &mut rng);
        taken += 1;
    }
    hub.tape.lock().unwrap().ingested += taken as u64;
    errors.truncate(20);
    let status = if taken == 0 && !errors.is_empty() {
        400
    } else {
        200
    };
    (
        status,
        json!({"ok": taken > 0, "taken": taken, "errors": errors}).to_string(),
    )
}

// ---------------------------------------------------------------------------
// Offline outputs
// ---------------------------------------------------------------------------

/// 15m history for the genome backtests and the desk's warm start, in the cache format they
/// read: `ctrader_<SYM>_SB_15m.json`, rows `[t_ms, open, high, low, close, ticks]`. (The name is
/// the format's: the desk and the backtests first read a broker's history cache.)
fn write_cache(dir: &PathBuf, seed: u64, from: i64) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let now = now_ms();
    let end_m = (minute_of(now) / 15) * 15; // completed 15m bars only
    // One thread a symbol: each path is independent.
    let handles: Vec<_> = (0..SYMBOLS.len())
        .map(|sym| {
            let dir = dir.clone();
            std::thread::spawn(move || -> Result<(), String> {
                let spec = &SYMBOLS[sym];
                let mut sim = Sim::new(seed);
                let mut rows: Vec<[f64; 6]> = Vec::new();
                sim.range(sym, minute_of(from), end_m, |m, ticks| {
                    let t = T0_MS + (m / 15) * 15 * MINUTE_MS;
                    for tk in &ticks {
                        match rows.last_mut() {
                            Some(r) if r[0] as i64 == t => {
                                r[2] = r[2].max(tk.bid);
                                r[3] = r[3].min(tk.bid);
                                r[4] = tk.bid;
                                r[5] += 1.0;
                            }
                            _ => rows.push([t as f64, tk.bid, tk.bid, tk.bid, tk.bid, 1.0]),
                        }
                    }
                });
                let path = dir.join(format!("ctrader_{}_SB_15m.json", spec.name));
                let text = serde_json::to_string(
                    &rows
                        .iter()
                        .map(|r| json!([r[0] as i64, r[1], r[2], r[3], r[4], r[5]]))
                        .collect::<Vec<_>>(),
                )
                .map_err(|e| e.to_string())?;
                std::fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))?;
                eprintln!("floor_tape: wrote {} ({} bars)", path.display(), rows.len());
                Ok(())
            })
        })
        .collect();
    for h in handles {
        h.join()
            .map_err(|_| "a cache thread panicked".to_string())??;
    }
    Ok(())
}

/// The synthetic market as one-minute bar CSVs, in the format `--csv` replays.
fn write_csv(dir: &PathBuf, seed: u64, days: i64) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let end_m = minute_of(now_ms());
    let mut sim = Sim::new(seed);
    for (sym, spec) in SYMBOLS.iter().enumerate() {
        let mut out = String::from("time,open,high,low,close,volume\n");
        sim.range(sym, end_m - days * 1440, end_m, |m, ticks| {
            let (o, c) = (ticks[0].bid, ticks[ticks.len() - 1].bid);
            let h = ticks.iter().map(|t| t.bid).fold(f64::MIN, f64::max);
            let l = ticks.iter().map(|t| t.bid).fold(f64::MAX, f64::min);
            out.push_str(&format!(
                "{},{o},{h},{l},{c},{}\n",
                T0_MS + m * MINUTE_MS,
                ticks.len()
            ));
        });
        let path = dir.join(format!("{}.csv", spec.name));
        std::fs::write(&path, out).map_err(|e| format!("{}: {e}", path.display()))?;
        eprintln!("floor_tape: wrote {}", path.display());
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

fn main() {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
    };
    if let Some(dir) = &args.write_cache {
        if let Err(e) = write_cache(dir, args.seed, args.from_ms) {
            eprintln!("floor_tape: {e}");
            std::process::exit(1);
        }
        return;
    }
    if let Some(dir) = &args.write_csv {
        if let Err(e) = write_csv(dir, args.seed, args.days) {
            eprintln!("floor_tape: {e}");
            std::process::exit(1);
        }
        return;
    }

    let now = now_ms();
    let csv = match &args.csv {
        Some(dir) => match csv_source::load_dir(dir) {
            Ok(d) => Some(csv_source::Replay::new(
                d,
                args.warmup_ms,
                args.speed,
                args.looping,
                now,
            )),
            Err(e) => {
                eprintln!("floor_tape: {e}");
                std::process::exit(1);
            }
        },
        None => None,
    };
    let owner: Vec<Owner> = (0..SYMBOLS.len())
        .map(|i| {
            if args.ingest.contains(&i) {
                Owner::Ingest
            } else if csv.as_ref().is_some_and(|c| c.data.contains_key(&i)) {
                Owner::Csv
            } else if args.only.as_ref().is_none_or(|o| o.contains(&i)) {
                Owner::Synthetic
            } else {
                Owner::Off
            }
        })
        .collect();

    let listener = match TcpListener::bind((args.bind.as_str(), args.port)) {
        Ok(l) => l,
        Err(e) => {
            eprintln!(
                "floor_tape: cannot listen on {}:{}: {e}",
                args.bind, args.port
            );
            std::process::exit(1);
        }
    };
    let hub = Hub {
        tape: Arc::new(Mutex::new(Tape {
            owner: owner.clone(),
            series: (0..SYMBOLS.len()).map(|_| Series::default()).collect(),
            quote: vec![None; SYMBOLS.len()],
            ticks: vec![0; SYMBOLS.len()],
            ladder: vec![None; SYMBOLS.len()],
            l2: None,
            history_ms: args.history_ms,
            seed: args.seed,
            started: now,
            ingested: 0,
        })),
        clients: Arc::new(Mutex::new(Vec::new())),
    };

    // History first: the synthetic path up to now, and the CSV warm-up.
    let mut sim = Sim::new(args.seed);
    let now_m = minute_of(now);
    let from_m = minute_of(now - args.history_ms);
    let mut live: HashMap<usize, (i64, Vec<Tick>, usize)> = HashMap::new();
    {
        let mut t = hub.tape.lock().unwrap();
        for sym in (0..SYMBOLS.len()).filter(|s| owner[*s] == Owner::Synthetic) {
            let series = &mut t.series[sym];
            sim.range(sym, from_m, now_m, |_, ticks| {
                ticks.iter().for_each(|tk| _ = series.add_tick(tk))
            });
            let cur = sim.minute(sym, now_m);
            let done = cur.partition_point(|tk| tk.t <= now);
            cur[..done].iter().for_each(|tk| _ = series.add_tick(tk));
            t.quote[sym] = cur[..done].last().copied().or_else(|| {
                series.bars.back().map(|b| Tick {
                    t: b.t,
                    bid: b.c,
                    ask: b.c,
                })
            });
            live.insert(sym, (now_m, cur, done));
        }
        if owner[BTC] == Owner::Synthetic
            && let Some(q) = t.quote[BTC]
        {
            let ask = if q.ask > q.bid {
                q.ask
            } else {
                q.bid + SYMBOLS[BTC].spread
            };
            t.l2 = Some(l2::L2::new(args.seed, q.bid, ask));
        }
    }
    let mut csv = csv;
    if let Some(r) = csv.as_mut() {
        let mut t = hub.tape.lock().unwrap();
        for (sym, tk) in r.history() {
            t.series[sym].add_tick(&tk);
            t.quote[sym] = Some(tk);
        }
    }

    let summary: Vec<String> = SYMBOLS
        .iter()
        .zip(&owner)
        .map(|(s, o)| format!("{}={}", s.name, o.label()))
        .collect();
    eprintln!(
        "floor_tape: listening on {}:{}  seed={}  {}",
        args.bind,
        args.port,
        args.seed,
        summary.join(" ")
    );
    if owner.contains(&Owner::Ingest) {
        eprintln!(
            "floor_tape: POST ticks to http://{}:{}/ingest  e.g. {{\"symbol\":\"BTCUSD\",\"bid\":100000.5,\"ask\":100001.0}}",
            if args.bind == "0.0.0.0" {
                "localhost"
            } else {
                &args.bind
            },
            args.port
        );
    }

    let accept_hub = hub.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let hub = accept_hub.clone();
            std::thread::spawn(move || handle(hub, stream));
        }
    });

    // The clock: every 50 ms, play what is due.
    let mut rng = Rng::new(args.seed ^ 0xD3);
    let mut last_trim = now;
    loop {
        std::thread::sleep(Duration::from_millis(50));
        let now = now_ms();
        let m = minute_of(now);
        let mut due: Vec<(usize, Tick)> = Vec::new();
        for (sym, (cur_m, ticks, next)) in live.iter_mut() {
            while *cur_m <= m {
                while *next < ticks.len() && ticks[*next].t <= now {
                    due.push((*sym, ticks[*next]));
                    *next += 1;
                }
                if *cur_m == m {
                    break;
                }
                *cur_m += 1;
                *ticks = sim.minute(*sym, *cur_m);
                *next = 0;
            }
        }
        if let Some(r) = csv.as_mut() {
            due.extend(r.poll(now));
        }
        due.sort_by_key(|(_, t)| t.t);
        for (sym, tk) in due {
            hub.publish(sym, tk, None, &mut rng);
        }
        if now - last_trim > 10 * MINUTE_MS {
            last_trim = now;
            let mut t = hub.tape.lock().unwrap();
            let keep = now - t.history_ms;
            t.series.iter_mut().for_each(|s| s.trim(keep));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::decode;

    #[test]
    fn decode_handles_escapes_plus_and_stray_percents() {
        assert_eq!(decode("EUR%2FUSD"), "EUR/USD");
        assert_eq!(decode("a+b"), "a b");
        assert_eq!(decode("100%"), "100%");
        assert_eq!(decode("%zz"), "%zz");
        // A `%` right before a multi-byte character used to panic on a char boundary.
        assert_eq!(decode("%é"), "%é");
        assert_eq!(decode("x%€y"), "x%€y");
    }
}
