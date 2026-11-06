//! jev_relay: holds the TypeSafe (Jev) API key and forwards System One calls.
//!
//! A deliberately small HTTP/1.1 server on tokio (no framework): `GET /health`, `POST /jev`,
//! `GET /paper/trades` (the closed-trade log for the desk page),
//! CORS preflight. One upstream call at a time with a minimum gap between calls, so a buggy
//! client cannot burn through the TypeSafe budget. Every call is appended to a JSONL log with
//! an observation hash so the floor can later replay recorded inferences.
//!
//! It also carries the headless paper desk's scoreboard to the browser: the desk
//! (`opencry-visual/src/bin/paper_desk.rs`) posts a heartbeat every few seconds with its
//! per-trader running totals and open positions (`POST /paper/heartbeat`); the app reads the
//! last one back (`GET /paper/board`) and, while the desk is alive, shows that board instead of
//! running its own book. The relay computes nothing: it keeps the last heartbeat and stamps it
//! with its age.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;

const UPSTREAM: &str = "https://api.typesafe.ai/v1/systemone";
const MAX_BODY: usize = 256 * 1024;
const UPSTREAM_TIMEOUT: Duration = Duration::from_secs(30);
/// A desk whose last heartbeat is older than this is not alive: the app runs its own book.
const DESK_ALIVE_MS: u64 = 30_000;

struct Config {
    bind: String,
    min_gap: Duration,
    log_path: PathBuf,
    model: String,
}

#[derive(Default, Serialize, Clone)]
struct Stats {
    calls: u64,
    ok_calls: u64,
    failed: u64,
    in_flight: bool,
    last_call_unix_ms: u64,
    last_latency_ms: u64,
    last_error: String,
}

struct Relay {
    key: String,
    cfg: Config,
    http: reqwest::Client,
    /// Serialises upstream calls and holds the time the next one may start.
    gate: Mutex<Option<Instant>>,
    stats: Mutex<Stats>,
    /// The paper desk's last heartbeat and when it arrived (relay clock, Unix ms).
    heartbeat: Mutex<Option<(u64, Value)>>,
    /// Manual paper orders from the page's trade ticket, for the desk to take: (seq, order).
    orders: Mutex<Vec<(u64, Value)>>,
}

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Never printed, never returned: the key is read once and lives in the relay only.
fn load_key() -> Option<String> {
    for var in ["JEV_KEY", "TYPESAFE_API_KEY"] {
        if let Ok(v) = std::env::var(var) {
            let v = v.trim().to_string();
            if !v.is_empty() {
                return Some(v);
            }
        }
    }
    let candidates = [
        manifest_dir().join(".env"),
        manifest_dir().join("../jevfloor_learning_day/.env"),
        PathBuf::from(".env"),
    ];
    for path in candidates {
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        for line in text.lines() {
            let line = line.trim();
            if let Some(rest) = line
                .strip_prefix("JEV_KEY=")
                .or_else(|| line.strip_prefix("TYPESAFE_API_KEY="))
            {
                let v = rest.trim().trim_matches('"').trim_matches('\'').trim();
                if !v.is_empty() {
                    eprintln!("jev_relay: key loaded from {}", path.display());
                    return Some(v.to_string());
                }
            }
        }
    }
    None
}

fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// FNV-1a 64 of the observation text: enough to match a replayed observation to its inference.
fn fnv64(s: &str) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}")
}

/// A desk heartbeat must be a JSON object with a `traders` array; anything else is refused so
/// a stray POST cannot blank the board.
fn parse_heartbeat(body: &[u8]) -> Result<Value, String> {
    let v: Value = serde_json::from_slice(body).map_err(|e| format!("invalid JSON: {e}"))?;
    let Value::Object(m) = &v else {
        return Err("body must be a JSON object".into());
    };
    match m.get("traders") {
        Some(Value::Array(_)) => Ok(v),
        Some(_) => Err("`traders` must be an array".into()),
        None => Err("missing `traders` array".into()),
    }
}

/// The board the app reads: the last heartbeat with `desk_alive`, `heartbeat_age_ms` and
/// `received_ms` added. With no heartbeat yet, a dead board with no traders.
fn board_from(heartbeat: Option<&(u64, Value)>, now_ms: u64) -> Value {
    match heartbeat {
        Some((received, Value::Object(m))) => {
            let age = now_ms.saturating_sub(*received);
            let mut m = m.clone();
            m.insert("desk_alive".into(), json!(age <= DESK_ALIVE_MS));
            m.insert("heartbeat_age_ms".into(), json!(age));
            m.insert("received_ms".into(), json!(received));
            Value::Object(m)
        }
        _ => json!({"desk_alive": false, "heartbeat_age_ms": Value::Null, "traders": []}),
    }
}

struct Request {
    method: String,
    path: String,
    body: Vec<u8>,
}

async fn read_request(stream: &mut TcpStream) -> Result<Request, String> {
    let mut reader = BufReader::new(stream);
    let mut head = Vec::new();
    let mut buf = [0u8; 1];
    // Read the head byte by byte up to CRLFCRLF; heads are tiny and this keeps the parser dumb.
    while !head.ends_with(b"\r\n\r\n") {
        let n = reader.read(&mut buf).await.map_err(|e| e.to_string())?;
        if n == 0 {
            return Err("connection closed before headers".into());
        }
        head.push(buf[0]);
        if head.len() > 16 * 1024 {
            return Err("headers too large".into());
        }
    }
    let head = String::from_utf8_lossy(&head);
    let mut lines = head.split("\r\n");
    let request_line = lines.next().unwrap_or("");
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("").to_uppercase();
    let path = parts.next().unwrap_or("/").to_string();
    let mut content_length = 0usize;
    for line in lines {
        if let Some((k, v)) = line.split_once(':')
            && k.trim().eq_ignore_ascii_case("content-length")
        {
            content_length = v.trim().parse().unwrap_or(0);
        }
    }
    if content_length > MAX_BODY {
        return Err(format!("body too large ({content_length} bytes)"));
    }
    let mut body = vec![0u8; content_length];
    if content_length > 0 {
        reader
            .read_exact(&mut body)
            .await
            .map_err(|e| e.to_string())?;
    }
    Ok(Request { method, path, body })
}

async fn write_response(stream: &mut TcpStream, status: u16, body: &str, extra: &[(&str, String)]) {
    let reason = match status {
        200 => "OK",
        204 => "No Content",
        400 => "Bad Request",
        404 => "Not Found",
        405 => "Method Not Allowed",
        413 => "Payload Too Large",
        429 => "Too Many Requests",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        _ => "OK",
    };
    let mut out = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\
         Access-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: GET, POST, OPTIONS\r\n\
         Access-Control-Allow-Headers: Content-Type\r\nAccess-Control-Max-Age: 86400\r\nConnection: close\r\n",
        body.len()
    );
    for (k, v) in extra {
        out.push_str(&format!("{k}: {v}\r\n"));
    }
    out.push_str("\r\n");
    out.push_str(body);
    let _ = stream.write_all(out.as_bytes()).await;
    let _ = stream.shutdown().await;
}

impl Relay {
    async fn health(&self) -> String {
        let s = self.stats.lock().await.clone();
        let desk_alive = self
            .heartbeat
            .lock()
            .await
            .as_ref()
            .is_some_and(|(at, _)| unix_ms().saturating_sub(*at) <= DESK_ALIVE_MS);
        json!({
            "ok": true,
            "key_loaded": !self.key.is_empty(),
            "model": self.cfg.model,
            "min_gap_ms": self.cfg.min_gap.as_millis() as u64,
            "desk_alive": desk_alive,
            "stats": s,
        })
        .to_string()
    }

    async fn paper_heartbeat(&self, body: &[u8]) -> (u16, String) {
        match parse_heartbeat(body) {
            Ok(v) => {
                *self.heartbeat.lock().await = Some((unix_ms(), v));
                (200, json!({"ok": true}).to_string())
            }
            Err(e) => (400, json!({"error": e}).to_string()),
        }
    }

    /// Closed trades from `paper_trades.jsonl` (the `/log/paper_trades` sink) with
    /// `closed_ms >= since_ms`, oldest first, the last `limit` of them. The file is read on
    /// each call: it is a few megabytes at most and this is polled every ten seconds or so.
    async fn paper_trades(
        &self,
        since_ms: i64,
        limit: usize,
        source: Option<&str>,
        trader: Option<&str>,
    ) -> String {
        let path = self.cfg.log_path.with_file_name("paper_trades.jsonl");
        let text = tokio::fs::read_to_string(&path).await.unwrap_or_default();
        let mut rows: Vec<Value> = Vec::new();
        let mut lines = 0usize;
        for line in text.lines() {
            lines += 1;
            let Ok(v) = serde_json::from_str::<Value>(line) else {
                continue;
            };
            if v.get("closed_ms").and_then(Value::as_i64).unwrap_or(0) < since_ms {
                continue;
            }
            if let Some(src) = source
                && v.get("source").and_then(Value::as_str).unwrap_or("") != src
            {
                continue;
            }
            if let Some(name) = trader
                && v.get("trader").and_then(Value::as_str).unwrap_or("") != name
            {
                continue;
            }
            rows.push(v);
        }
        rows.sort_by_key(|v| v.get("closed_ms").and_then(Value::as_i64).unwrap_or(0));
        let truncated = rows.len() > limit;
        if truncated {
            rows.drain(..rows.len() - limit);
        }
        json!({"trades": rows, "count": rows.len(), "since_ms": since_ms, "truncated": truncated, "file_lines": lines}).to_string()
    }

    async fn paper_board(&self) -> String {
        board_from(self.heartbeat.lock().await.as_ref(), unix_ms()).to_string()
    }

    /// Forward one System One call. Returns (status, body).
    async fn jev(&self, body: &[u8]) -> (u16, String) {
        let mut req: Value = match serde_json::from_slice(body) {
            Ok(Value::Object(m)) => Value::Object(m),
            Ok(_) => {
                return (
                    400,
                    json!({"error": "body must be a JSON object"}).to_string(),
                );
            }
            Err(e) => {
                return (
                    400,
                    json!({"error": format!("invalid JSON: {e}")}).to_string(),
                );
            }
        };
        let state = match req.get("state").and_then(Value::as_str) {
            Some(s) if !s.trim().is_empty() => s.to_string(),
            _ => return (400, json!({"error": "missing `state` string"}).to_string()),
        };
        if !req.get("questions").map(Value::is_object).unwrap_or(false) {
            return (
                400,
                json!({"error": "missing `questions` object"}).to_string(),
            );
        }
        if req.get("model").is_none() {
            req["model"] = Value::String(self.cfg.model.clone());
        }

        if self.key.is_empty() {
            return (503, json!({"error": "no Jev key on this relay: the floor uses its rule-of-thumb stand-in", "no_key": true}).to_string());
        }
        // One call at a time, and not before the gap has passed.
        let mut gate = match self.gate.try_lock() {
            Ok(g) => g,
            Err(_) => {
                return (
                    429,
                    json!({"error": "relay busy: one call at a time"}).to_string(),
                );
            }
        };
        if let Some(next) = *gate {
            let now = Instant::now();
            if now < next {
                let wait = (next - now).as_millis() as u64;
                return (
                    429,
                    json!({"error": "relay rate limit", "retry_after_ms": wait}).to_string(),
                );
            }
        }
        *gate = Some(Instant::now() + self.cfg.min_gap);
        {
            let mut s = self.stats.lock().await;
            s.in_flight = true;
            s.calls += 1;
            s.last_call_unix_ms = unix_ms();
        }

        let started = Instant::now();
        let result = self
            .http
            .post(UPSTREAM)
            .bearer_auth(&self.key)
            .json(&req)
            .send()
            .await;
        let latency = started.elapsed().as_millis() as u64;
        let (status, text) = match result {
            Ok(r) => {
                let status = r.status().as_u16();
                (status, r.text().await.unwrap_or_default())
            }
            Err(e) => (
                502,
                json!({"error": format!("upstream request failed: {e}")}).to_string(),
            ),
        };
        drop(gate);

        let ok = (200..300).contains(&status);
        {
            let mut s = self.stats.lock().await;
            s.in_flight = false;
            s.last_latency_ms = latency;
            if ok {
                s.ok_calls += 1;
                s.last_error.clear();
            } else {
                s.failed += 1;
                s.last_error = format!(
                    "HTTP {status}: {}",
                    text.chars().take(160).collect::<String>()
                );
            }
        }
        eprintln!(
            "jev_relay: {} {status} {latency} ms hash={} state={} chars",
            if ok { "ok  " } else { "FAIL" },
            fnv64(&state),
            state.len()
        );

        let response: Value =
            serde_json::from_str(&text).unwrap_or_else(|_| Value::String(text.clone()));
        let line = json!({
            "ts": unix_ms(),
            "observation_hash": fnv64(&state),
            "model": req.get("model").cloned().unwrap_or(Value::Null),
            "latency_ms": latency,
            "status": status,
            "request": req,
            "response": response,
        });
        self.append_log(&line).await;
        (status, text)
    }

    async fn append_log(&self, line: &Value) {
        if let Some(dir) = self.cfg.log_path.parent() {
            let _ = tokio::fs::create_dir_all(dir).await;
        }
        let mut text = line.to_string();
        text.push('\n');
        match tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.cfg.log_path)
            .await
        {
            Ok(mut f) => {
                let _ = f.write_all(text.as_bytes()).await;
            }
            Err(e) => eprintln!(
                "jev_relay: cannot write {}: {e}",
                self.cfg.log_path.display()
            ),
        }
    }
}

async fn handle(relay: Arc<Relay>, mut stream: TcpStream) {
    let req = match read_request(&mut stream).await {
        Ok(r) => r,
        Err(e) => {
            let status = if e.contains("too large") { 413 } else { 400 };
            write_response(&mut stream, status, &json!({"error": e}).to_string(), &[]).await;
            return;
        }
    };
    let path = req.path.split('?').next().unwrap_or("/");
    match (req.method.as_str(), path) {
        ("OPTIONS", _) => write_response(&mut stream, 204, "", &[]).await,
        ("GET", "/health") | ("GET", "/") => {
            let body = relay.health().await;
            write_response(&mut stream, 200, &body, &[]).await;
        }
        ("POST", "/jev") => {
            let started = Instant::now();
            let (status, body) = relay.jev(&req.body).await;
            let extra = [(
                "X-Jev-Relay-Latency-Ms",
                started.elapsed().as_millis().to_string(),
            )];
            write_response(&mut stream, status, &body, &extra).await;
        }
        ("GET", "/jev") => {
            write_response(
                &mut stream,
                405,
                &json!({"error": "POST a System One body"}).to_string(),
                &[],
            )
            .await
        }
        ("POST", "/paper/heartbeat") => {
            let (status, body) = relay.paper_heartbeat(&req.body).await;
            write_response(&mut stream, status, &body, &[]).await;
        }
        ("GET", "/paper/board") => {
            let body = relay.paper_board().await;
            write_response(&mut stream, 200, &body, &[]).await;
        }
        ("GET", "/paper/trades") => {
            // The closed-trade log for the desk page: ?since_ms=<unix ms>&limit=<n>&source=desk
            let query = req.path.split_once('?').map(|x| x.1).unwrap_or("");
            let q = |k: &str| {
                query
                    .split('&')
                    .find_map(|kv| kv.strip_prefix(&format!("{k}=")).map(url_decode))
            };
            let since_ms: i64 = q("since_ms").and_then(|v| v.parse().ok()).unwrap_or(0);
            let limit: usize = q("limit")
                .and_then(|v| v.parse().ok())
                .unwrap_or(4000)
                .clamp(1, 20000);
            let source = q("source");
            let trader = q("trader");
            let body = relay
                .paper_trades(since_ms, limit, source.as_deref(), trader.as_deref())
                .await;
            write_response(&mut stream, 200, &body, &[]).await;
        }
        ("POST", "/paper/order") => {
            // A manual order from the page's trade ticket, queued here for the paper desk, which
            // polls `/paper/orders` and fills them on paper. The relay never talks to a broker.
            match serde_json::from_slice::<Value>(&req.body) {
                Ok(Value::Object(mut m)) if matches!(m.get("action").and_then(|a| a.as_str()), Some("buy" | "sell" | "close")) && m.get("symbol").and_then(|x| x.as_str()).is_some()
                    || matches!(m.get("action").and_then(|a| a.as_str()), Some("name")) && m.get("trader").and_then(|x| x.as_str()).is_some_and(|t| !t.trim().is_empty()) => {
                    let mut q = relay.orders.lock().await;
                    let seq = q.last().map(|o| o.0 + 1).unwrap_or(1);
                    m.insert("seq".into(), json!(seq));
                    m.insert("placed_ms".into(), json!(unix_ms()));
                    q.push((seq, Value::Object(m)));
                    let n = q.len();
                    if n > 200 {
                        q.drain(..n - 200);
                    }
                    write_response(&mut stream, 200, &json!({"ok": true, "seq": seq}).to_string(), &[]).await;
                }
                _ => write_response(&mut stream, 400, &json!({"error": "body must be {action: buy|sell|close, symbol, ...} or {action: name, trader}"}).to_string(), &[]).await,
            }
        }
        ("GET", "/paper/orders") => {
            // ?after=<seq>: the orders queued since then, oldest first.
            let after: u64 = req
                .path
                .split_once("after=")
                .and_then(|x| x.1.split('&').next())
                .and_then(|v| v.parse().ok())
                .unwrap_or(0);
            let q = relay.orders.lock().await;
            let list: Vec<&Value> = q.iter().filter(|o| o.0 > after).map(|o| &o.1).collect();
            let last = q.last().map(|o| o.0).unwrap_or(0);
            write_response(
                &mut stream,
                200,
                &json!({"orders": list, "last_seq": last}).to_string(),
                &[],
            )
            .await;
        }
        ("POST", p) if p.starts_with("/log/") => {
            // Append-only JSONL sink for the floor's own records (paper trades, decisions):
            // the browser cannot write files, the relay can. Name is restricted to a safe set.
            let name: String = p["/log/".len()..]
                .chars()
                .take(40)
                .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect();
            if name.is_empty() {
                write_response(
                    &mut stream,
                    400,
                    &json!({"error": "log name must be [A-Za-z0-9_]"}).to_string(),
                    &[],
                )
                .await;
                return;
            }
            match serde_json::from_slice::<Value>(&req.body) {
                Ok(Value::Object(mut m)) => {
                    m.insert("relay_ts".into(), json!(unix_ms()));
                    let path = relay.cfg.log_path.with_file_name(format!("{name}.jsonl"));
                    let mut text = Value::Object(m).to_string();
                    text.push('\n');
                    let ok = match tokio::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(&path)
                        .await
                    {
                        Ok(mut f) => f.write_all(text.as_bytes()).await.is_ok(),
                        Err(_) => false,
                    };
                    let status = if ok { 200 } else { 500 };
                    write_response(
                        &mut stream,
                        status,
                        &json!({"ok": ok, "log": name}).to_string(),
                        &[],
                    )
                    .await;
                }
                _ => {
                    write_response(
                        &mut stream,
                        400,
                        &json!({"error": "body must be a JSON object"}).to_string(),
                        &[],
                    )
                    .await
                }
            }
        }
        _ => {
            write_response(
                &mut stream,
                404,
                &json!({"error": "not found"}).to_string(),
                &[],
            )
            .await
        }
    }
}

#[tokio::main]
async fn main() {
    // Without a key the relay still runs: the paper logs, the desk's board and the trade
    // queries need no Jev, and `/jev` answers 503 at once, which the floor and the desk take as
    // "use the rule-of-thumb stand-in" (it is the same fallback they use when Jev is slow).
    let key = load_key().unwrap_or_else(|| {
        eprintln!("jev_relay: no API key, so /jev answers 503 and the floor uses its free rule-of-thumb stand-in. To use Jev, set JEV_KEY or TYPESAFE_API_KEY, or put JEV_KEY=... in jevfloor_learning_day/.env");
        String::new()
    });
    let cfg = Config {
        bind: std::env::var("JEV_RELAY_BIND").unwrap_or_else(|_| "0.0.0.0:8099".into()),
        min_gap: Duration::from_millis(
            std::env::var("JEV_RELAY_MIN_GAP_MS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(2000),
        ),
        log_path: std::env::var("JEV_RELAY_LOG")
            .map(PathBuf::from)
            .unwrap_or_else(|_| manifest_dir().join("log/inferences.jsonl")),
        model: std::env::var("JEV_MODEL").unwrap_or_else(|_| "jev-latest".into()),
    };
    let http = reqwest::Client::builder()
        .timeout(UPSTREAM_TIMEOUT)
        .build()
        .expect("http client");
    let listener = match TcpListener::bind(&cfg.bind).await {
        Ok(l) => l,
        Err(e) => {
            eprintln!("jev_relay: cannot bind {}: {e}", cfg.bind);
            std::process::exit(1);
        }
    };
    eprintln!(
        "jev_relay: listening on {}  model={}  min_gap={} ms  log={}",
        cfg.bind,
        cfg.model,
        cfg.min_gap.as_millis(),
        cfg.log_path.display()
    );
    let relay = Arc::new(Relay {
        key,
        cfg,
        http,
        gate: Mutex::new(None),
        stats: Mutex::new(Stats::default()),
        heartbeat: Mutex::new(None),
        orders: Mutex::new(Vec::new()),
    });
    loop {
        match listener.accept().await {
            Ok((stream, _)) => {
                let relay = relay.clone();
                tokio::spawn(async move { handle(relay, stream).await });
            }
            Err(e) => eprintln!("jev_relay: accept failed: {e}"),
        }
    }
}

/// Decodes a query value: `%XX` bytes and `+` as a space (so `?trader=Ada%20Lovelace` works).
fn url_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let hex = |c: u8| (c as char).to_digit(16).map(|d| d as u8);
        match b[i] {
            b'%' if i + 2 < b.len() && hex(b[i + 1]).is_some() && hex(b[i + 2]).is_some() => {
                out.push(hex(b[i + 1]).unwrap() * 16 + hex(b[i + 2]).unwrap());
                i += 3;
                continue;
            }
            b'+' => out.push(b' '),
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_query_value_is_decoded() {
        assert_eq!(url_decode("Ada%20Lovelace"), "Ada Lovelace");
        assert_eq!(url_decode("Ada+Lovelace"), "Ada Lovelace");
        assert_eq!(url_decode("Zo%C3%AB%26Co"), "Zoë&Co");
        assert_eq!(url_decode("100%"), "100%");
        assert_eq!(url_decode("%zz"), "%zz");
    }

    #[test]
    fn fnv_is_stable() {
        assert_eq!(fnv64(""), "cbf29ce484222325");
        assert_eq!(fnv64("a"), "af63dc4c8601ec8c");
        assert_ne!(fnv64("silver up"), fnv64("silver down"));
    }

    #[test]
    fn a_heartbeat_needs_a_traders_array() {
        assert!(
            parse_heartbeat(br#"{"traders":[{"trader":"Colin","net_r":0.5}],"sent_ms":1}"#).is_ok()
        );
        assert_eq!(
            parse_heartbeat(b"[]").unwrap_err(),
            "body must be a JSON object"
        );
        assert_eq!(
            parse_heartbeat(br#"{"sent_ms":1}"#).unwrap_err(),
            "missing `traders` array"
        );
        assert_eq!(
            parse_heartbeat(br#"{"traders":3}"#).unwrap_err(),
            "`traders` must be an array"
        );
        assert!(
            parse_heartbeat(b"{not json")
                .unwrap_err()
                .starts_with("invalid JSON")
        );
    }

    #[test]
    fn the_board_is_the_last_heartbeat_stamped_with_its_age() {
        let dead = board_from(None, 1000);
        assert_eq!(dead["desk_alive"], json!(false));
        assert_eq!(dead["traders"], json!([]));
        assert!(dead["heartbeat_age_ms"].is_null());

        let hb = (
            10_000u64,
            json!({"traders": [{"trader": "Grace", "net_r": 1.25}], "jev_mode": "live"}),
        );
        let fresh = board_from(Some(&hb), 10_000 + 29_999);
        assert_eq!(fresh["desk_alive"], json!(true));
        assert_eq!(fresh["heartbeat_age_ms"], json!(29_999));
        assert_eq!(fresh["received_ms"], json!(10_000));
        assert_eq!(fresh["jev_mode"], json!("live"));
        assert_eq!(fresh["traders"][0]["trader"], json!("Grace"));

        let stale = board_from(Some(&hb), 10_000 + 30_001);
        assert_eq!(stale["desk_alive"], json!(false), "older than 30 s is dead");
        assert_eq!(
            stale["traders"][0]["net_r"],
            json!(1.25),
            "the last numbers are still served"
        );
        // A clock that went backwards never makes a negative age.
        assert_eq!(board_from(Some(&hb), 5)["heartbeat_age_ms"], json!(0));
    }
}
