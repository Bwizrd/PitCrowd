//! Where the floor's data comes from: one place that turns the page's query string (in the
//! browser) or the environment (natively) into the addresses of the tape, the relay, the
//! Level 2 feed and, optionally, a broker bridge.
//!
//! In the browser every service defaults to the machine that served the page, on its usual
//! port, and any of them can be pointed elsewhere from the address bar:
//!
//!   ?tape=18083                    same host, another port
//!   ?tape=prices.example:8083      another machine
//!   ?tape=https://prices.example   a full base URL (wss:// is derived for the socket)
//!   ?relay=...                     the same forms, for the Jev relay
//!   ?l2=tape|kraken|off|wss://...  Level 2: the tape's /l2 (default), Kraken's public feed,
//!                                  none, or any socket speaking Kraken's v2 book messages
//!   ?bridge=8000                   a cTrader-bridge-style /getData for deeper history
//!                                  (off unless given)
//!
//! Natively: OPENCRY_TAPE_URL, OPENCRY_FEED_URL, OPENCRY_JEV_URL, OPENCRY_L2_URL and
//! OPENCRY_BRIDGE_URL, with the same defaults.

pub const TAPE_PORT: u16 = 8083;
pub const RELAY_PORT: u16 = 8099;
pub const KRAKEN_WS: &str = "wss://ws.kraken.com/v2";

/// A value from the page's query string (`?name=value`), browser only.
pub fn page_param(name: &str) -> Option<String> {
    #[cfg(target_arch = "wasm32")]
    {
        let search = web_sys::window()?.location().search().ok()?;
        query_value(&search, name)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = name;
        None
    }
}

/// `name`'s value in a query string like `?a=1&b=two`, percent-decoded.
pub fn query_value(search: &str, name: &str) -> Option<String> {
    search.trim_start_matches('?').split('&').find_map(|kv| {
        let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
        (k == name).then(|| percent_decode(v))
    })
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16)
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(if b[i] == b'+' { b' ' } else { b[i] });
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The host that served the page (`localhost` natively or when there is none).
pub fn page_host() -> String {
    #[cfg(target_arch = "wasm32")]
    {
        web_sys::window()
            .and_then(|w| w.location().hostname().ok())
            .filter(|h| !h.is_empty())
            .unwrap_or_else(|| "localhost".to_string())
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        "localhost".to_string()
    }
}

/// Turn a `?tape=` style value into a base URL: a bare port goes on the page's host, a
/// `host:port` gets `http://`, and a full URL is kept (without a trailing slash).
pub fn base_url(value: &str, host: &str) -> String {
    let v = value.trim().trim_end_matches('/');
    if !v.is_empty() && v.chars().all(|c| c.is_ascii_digit()) {
        format!("http://{host}:{v}")
    } else if v.contains("://") {
        v.to_string()
    } else {
        format!("http://{v}")
    }
}

/// `http(s)://` to `ws(s)://`.
pub fn ws_from_http(url: &str) -> String {
    if let Some(rest) = url.strip_prefix("https://") {
        format!("wss://{rest}")
    } else if let Some(rest) = url.strip_prefix("http://") {
        format!("ws://{rest}")
    } else {
        url.to_string()
    }
}

fn service(param: &str, env: &str, port: u16) -> String {
    if let Some(v) = page_param(param).filter(|v| !v.is_empty()) {
        return base_url(&v, &page_host());
    }
    #[cfg(not(target_arch = "wasm32"))]
    if let Ok(v) = std::env::var(env) {
        return v.trim_end_matches('/').to_string();
    }
    let _ = env;
    format!("http://{}:{port}", page_host())
}

/// The tape's HTTP base (`/bars`, `/orderbook`, `/health`).
pub fn tape_url() -> String {
    service("tape", "OPENCRY_TAPE_URL", TAPE_PORT)
}

/// The tape's websocket (ticks and books).
pub fn feed_url() -> String {
    #[cfg(not(target_arch = "wasm32"))]
    if let Ok(v) = std::env::var("OPENCRY_FEED_URL") {
        return v;
    }
    ws_from_http(&tape_url())
}

/// The Jev relay (also the paper desk's board and logs).
pub fn relay_url() -> String {
    service("relay", "OPENCRY_JEV_URL", RELAY_PORT)
}

/// The Level 2 socket, or `None` when it is switched off.
pub fn l2_url() -> Option<String> {
    let choice = page_param("l2").or({
        #[cfg(not(target_arch = "wasm32"))]
        {
            std::env::var("OPENCRY_L2_URL").ok()
        }
        #[cfg(target_arch = "wasm32")]
        {
            None
        }
    });
    l2_choice(choice.as_deref(), &tape_url())
}

pub fn l2_choice(choice: Option<&str>, tape: &str) -> Option<String> {
    match choice.map(str::trim) {
        None | Some("") | Some("tape") => Some(format!("{}/l2", ws_from_http(tape))),
        Some("kraken") => Some(KRAKEN_WS.to_string()),
        Some("off") | Some("none") => None,
        Some(url) => Some(url.to_string()),
    }
}

/// A broker bridge with a `/getData` history route, only when one is configured.
pub fn bridge_url() -> Option<String> {
    if let Some(v) = page_param("bridge").filter(|v| !v.is_empty()) {
        return Some(base_url(&v, &page_host()));
    }
    #[cfg(not(target_arch = "wasm32"))]
    if let Ok(v) = std::env::var("OPENCRY_BRIDGE_URL") {
        return Some(v.trim_end_matches('/').to_string());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_service_can_be_named_by_port_host_or_url() {
        assert_eq!(base_url("18083", "10.0.0.5"), "http://10.0.0.5:18083");
        assert_eq!(
            base_url("prices.example:8083", "x"),
            "http://prices.example:8083"
        );
        assert_eq!(
            base_url("https://prices.example/", "x"),
            "https://prices.example"
        );
        assert_eq!(
            ws_from_http("https://prices.example"),
            "wss://prices.example"
        );
        assert_eq!(ws_from_http("http://h:8083"), "ws://h:8083");
    }

    #[test]
    fn the_query_string_is_read_and_decoded() {
        assert_eq!(
            query_value("?tape=18083&l2=kraken", "l2").as_deref(),
            Some("kraken")
        );
        assert_eq!(
            query_value("?tape=h%3A8083", "tape").as_deref(),
            Some("h:8083")
        );
        assert_eq!(query_value("?tape=1", "relay"), None);
        assert_eq!(query_value("", "tape"), None);
    }

    #[test]
    fn level_two_defaults_to_the_tape_and_can_be_kraken_or_off() {
        assert_eq!(
            l2_choice(None, "http://h:8083").as_deref(),
            Some("ws://h:8083/l2")
        );
        assert_eq!(
            l2_choice(Some("kraken"), "http://h:8083").as_deref(),
            Some(KRAKEN_WS)
        );
        assert_eq!(l2_choice(Some("off"), "http://h:8083"), None);
        assert_eq!(
            l2_choice(Some("wss://my.feed/book"), "http://h:8083").as_deref(),
            Some("wss://my.feed/book")
        );
    }
}
