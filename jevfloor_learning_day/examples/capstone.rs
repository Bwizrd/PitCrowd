//! JevFloor Learning Day — Runnable Capstone
//!
//! Part 1: Event Channel and RollingBuffer Ingestion Pipeline
//! Part 2: Slices and Deterministic Feature Extraction
//! Part 3: Schema Validation and System One Interpretation
//! Part 4: Freshness Gate, Policy Engine & Strategy
//! Part 5: Multi-Scenario CLI Runner
//! Part 6: Deterministic Replay Engine
//! Part 7: Configurable Policy Thresholds & Curriculum Exit Verification
//!
//! Architecture:
//! Market Events (Channel) -> RollingBuffer (capacity 3, oldest evicted) -> Slice View (&[f64]) -> Deterministic Features -> System One Interpretation (Interpreter Trait / Mock) -> Schema Validation -> Freshness Gate & Policy Engine (Strategy Trait) -> Multi-Scenario CLI Runner -> Deterministic Offline Replay -> Configurable Policy Thresholds (Unscripted Experimentation)


use serde::Deserialize;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender};
use std::sync::Arc;
use std::thread;
use std::time::Duration;


/// Discrete market event carrying symbol and price
#[derive(Debug, Clone, PartialEq)]
pub enum MarketEvent {
    Tick { symbol: String, price: f64 },
}

/// Bounded sliding window of recent prices.
/// When capacity is reached, the oldest price is evicted.
#[derive(Debug, Clone, PartialEq)]
pub struct RollingBuffer {
    capacity: usize,
    prices: Vec<f64>,
}

impl RollingBuffer {
    /// Create a new empty rolling buffer with fixed capacity
    pub fn new(capacity: usize) -> Self {
        assert!(capacity > 0, "RollingBuffer capacity must be greater than zero");
        Self {
            capacity,
            prices: Vec::with_capacity(capacity),
        }
    }

    /// Push a new price into the buffer.
    /// If the buffer is full, the oldest price (index 0) is evicted first.
    pub fn push(&mut self, price: f64) {
        if self.prices.len() >= self.capacity {
            self.prices.remove(0);
        }
        self.prices.push(price);
    }

    /// Borrow an immutable slice view of the current prices
    pub fn as_slice(&self) -> &[f64] {
        &self.prices[..]
    }

    /// Number of prices currently held in the buffer
    pub fn len(&self) -> usize {
        self.prices.len()
    }

    /// Whether the buffer is empty
    pub fn is_empty(&self) -> bool {
        self.prices.is_empty()
    }

    /// Maximum capacity of the buffer
    pub fn capacity(&self) -> usize {
        self.capacity
    }
}

/// Deterministic features derived from a slice of historical prices
#[derive(Debug, Clone, PartialEq)]
pub struct Features {
    /// Overall price change from first observation to last observation (last - first)
    pub delta: f64,
    /// Number of positive price steps between adjacent observations
    pub positive_changes: usize,
}

/// Calculate deterministic features from a slice of prices.
/// Returns None if there are fewer than 2 prices, preventing panic on insufficient history.
pub fn calculate_features(prices: &[f64]) -> Option<Features> {
    if prices.len() < 2 {
        return None;
    }
    let first = *prices.first().unwrap();
    let last = *prices.last().unwrap();
    let delta = last - first;

    let positive_changes = prices
        .windows(2)
        .filter(|w| w[1] > w[0])
        .count();

    Some(Features {
        delta,
        positive_changes,
    })
}

/// Detailed error variants returned during bridge communication or parsing
#[derive(Debug, PartialEq, Clone)]
pub enum BridgeError {
    ConnectionFailed(String),
    HttpStatus(u16),
    Timeout(String),
    MalformedJson(String),
    BridgeReportedError(String),
    EmptyData,
}

impl std::fmt::Display for BridgeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BridgeError::ConnectionFailed(msg) => write!(f, "Connection failed: {msg}"),
            BridgeError::HttpStatus(code) => write!(f, "HTTP error status: {code}"),
            BridgeError::Timeout(msg) => write!(f, "Request timed out: {msg}"),
            BridgeError::MalformedJson(msg) => write!(f, "Malformed JSON response: {msg}"),
            BridgeError::BridgeReportedError(msg) => write!(f, "cTrader bridge error: {msg}"),
            BridgeError::EmptyData => write!(f, "Bridge returned empty candle data"),
        }
    }
}

impl std::error::Error for BridgeError {}

/// JSON candle shape returned by local cTrader REST bridge
#[derive(Debug, Deserialize, PartialEq, Clone)]
pub struct CTraderCandle {
    #[allow(dead_code)]
    pub timestamp: i64,
    pub close: f64,
}

/// JSON envelope returned by cTrader REST bridge
#[derive(Debug, Deserialize, PartialEq, Clone)]
pub struct CTraderResponse {
    #[serde(default)]
    pub data: Vec<CTraderCandle>,
    #[serde(default)]
    pub error: Option<String>,
}

/// Pure function: parse JSON string returned by cTrader REST bridge into a vector of close prices.
/// Extracts the most recent `bars_count` candle close prices.
pub fn parse_ctrader_response(
    json_str: &str,
    bars_count: usize,
) -> Result<Vec<f64>, BridgeError> {
    let body: CTraderResponse = serde_json::from_str(json_str)
        .map_err(|e| BridgeError::MalformedJson(e.to_string()))?;

    if let Some(err) = body.error {
        return Err(BridgeError::BridgeReportedError(err));
    }

    if body.data.is_empty() {
        return Err(BridgeError::EmptyData);
    }

    // Take the most recent `bars_count` candles (last items in array)
    let prices = body
        .data
        .iter()
        .rev()
        .take(bars_count)
        .map(|candle| candle.close)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();

    Ok(prices)
}

/// Fetch candle close prices from the cTrader REST bridge via HTTP.
pub fn fetch_ctrader_prices(
    client: &reqwest::blocking::Client,
    url: &str,
    bars_count: usize,
) -> Result<Vec<f64>, BridgeError> {
    let resp = client.get(url).send().map_err(|e| {
        if e.is_timeout() {
            BridgeError::Timeout(e.to_string())
        } else {
            BridgeError::ConnectionFailed(e.to_string())
        }
    })?;

    if !resp.status().is_success() {
        return Err(BridgeError::HttpStatus(resp.status().as_u16()));
    }

    let text = resp
        .text()
        .map_err(|e| BridgeError::ConnectionFailed(e.to_string()))?;

    parse_ctrader_response(&text, bars_count)
}

/// Ingestion source for market ticks
pub enum IngestionMode {
    /// Deterministic curriculum ticks: 100.0, 101.0, 103.0
    Synthetic,
    /// Configurable synthetic ticks (e.g. empty scenario with 0 ticks)
    SyntheticTicks(Vec<f64>),
    /// Live data from local cTrader REST bridge (pair 238 = XAGUSD_SB Silver)
    LiveCTrader { url: String, bars_count: usize },
}

impl IngestionMode {
    pub fn live_ctrader_pair(pair_id: u32, bars_count: usize) -> Self {
        Self::live_ctrader_url(pair_id, "1m", bars_count)
    }

    pub fn live_ctrader_url(pair_id: u32, timeframe: &str, bars_count: usize) -> Self {
        Self::LiveCTrader {
            url: format!(
                "http://localhost:8000/getData?pair={}&timeframe={}&range=-1d",
                pair_id, timeframe
            ),
            bars_count,
        }
    }
}

/// Spawn the tick producer thread and return a receiver channel.
/// In Synthetic mode, returns an active receiver channel immediately.
/// In LiveCTrader mode, fetches prices from the bridge; if the bridge is unavailable or invalid,
/// returns `Err(BridgeError)` immediately — NEVER silently falling back to synthetic ticks.
pub fn spawn_tick_producer(
    mode: IngestionMode,
    channel_bound: usize,
) -> Result<Receiver<MarketEvent>, BridgeError> {
    let (tx, rx): (SyncSender<MarketEvent>, Receiver<MarketEvent>) = sync_channel(channel_bound);

    match mode {
        IngestionMode::Synthetic => {
            thread::spawn(move || {
                let symbol = "SILVER".to_string();
                let curriculum_prices = [100.0, 101.0, 103.0];
                for &price in &curriculum_prices {
                    let event = MarketEvent::Tick {
                        symbol: symbol.clone(),
                        price,
                    };
                    if tx.send(event).is_err() {
                        break;
                    }
                }
            });
            Ok(rx)
        }
        IngestionMode::SyntheticTicks(prices) => {
            thread::spawn(move || {
                let symbol = "SILVER".to_string();
                for price in prices {
                    let event = MarketEvent::Tick {
                        symbol: symbol.clone(),
                        price,
                    };
                    if tx.send(event).is_err() {
                        break;
                    }
                }
            });
            Ok(rx)
        }
        IngestionMode::LiveCTrader { url, bars_count } => {
            let client = reqwest::blocking::Client::builder()
                .timeout(Duration::from_secs(5))
                .build()
                .map_err(|e| BridgeError::ConnectionFailed(e.to_string()))?;

            // Fetch live prices; fail immediately if bridge is unavailable or data invalid
            let prices = fetch_ctrader_prices(&client, &url, bars_count)?;

            thread::spawn(move || {
                let symbol = "SILVER".to_string();
                for price in prices {
                    let event = MarketEvent::Tick {
                        symbol: symbol.clone(),
                        price,
                    };
                    if tx.send(event).is_err() {
                        break;
                    }
                }
            });

            Ok(rx)
        }
    }
}

// ============================================================================
// Part 3: Schema Validation & System One Boundary (Lessons 18–21, 24)
// ============================================================================

/// Observation snapshot metadata passed across the inference boundary
#[derive(Debug, Clone, PartialEq)]
pub struct Snapshot {
    pub id: u64,
    pub observed_at_ms: u64,
}

/// Typed System One interpretation output carrying calibrated probabilities and confidence
#[derive(Debug, Clone, PartialEq)]
pub struct InterpretationPayload {
    pub p_persistent: f64,
    pub p_active: f64,
    pub confidence: f64,
}

/// Boundary envelope packaging the schema version, snapshot correlation ID, and payload
#[derive(Debug, Clone, PartialEq)]
pub struct Envelope {
    pub schema_version: u32,
    pub snapshot_id: u64,
    pub interpretation: InterpretationPayload,
}

/// Explicit error conditions at the System One provider and schema boundary
#[derive(Debug, Clone, PartialEq)]
pub enum InferenceError {
    ProviderUnavailable,
    StaleData,
    InvalidSchema,
    ValidationFailed(String),
}

impl std::fmt::Display for InferenceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InferenceError::ProviderUnavailable => write!(f, "Provider unavailable"),
            InferenceError::StaleData => write!(f, "Stale data"),
            InferenceError::InvalidSchema => write!(f, "Invalid schema version"),
            InferenceError::ValidationFailed(msg) => write!(f, "Validation failed: {msg}"),
        }
    }
}

impl std::error::Error for InferenceError {}

/// Validate envelope against schema version 1, expected snapshot ID, and valid probability bounds [0.0, 1.0].
pub fn validate_envelope(env: &Envelope, expected_snapshot_id: u64) -> Result<(), InferenceError> {
    if env.schema_version != 1 {
        return Err(InferenceError::InvalidSchema);
    }

    if env.snapshot_id != expected_snapshot_id {
        return Err(InferenceError::ValidationFailed(format!(
            "Snapshot ID mismatch: expected {}, got {}",
            expected_snapshot_id, env.snapshot_id
        )));
    }

    let values = [
        ("p_persistent", env.interpretation.p_persistent),
        ("p_active", env.interpretation.p_active),
        ("confidence", env.interpretation.confidence),
    ];

    for (name, val) in values {
        if val.is_nan() {
            return Err(InferenceError::ValidationFailed(format!("{name} is NaN")));
        }
        if !(0.0..=1.0).contains(&val) {
            return Err(InferenceError::ValidationFailed(format!(
                "{name} out of bounds: {val} (must be in [0.0, 1.0])"
            )));
        }
    }

    Ok(())
}

/// Provider-neutral trait defining the System One interpretation boundary
pub trait Interpreter {
    fn interpret(&self, snapshot: &Snapshot) -> Result<Envelope, InferenceError>;

    /// Context-aware interpretation receiving market prices and extracted features.
    /// Default implementation delegates to `self.interpret(snapshot)` and returns ("mock", false).
    fn interpret_market(
        &self,
        snapshot: &Snapshot,
        prices: &[f64],
        features: Option<&Features>,
    ) -> Result<(Envelope, String), InferenceError> {
        let _ = (prices, features);
        let env = self.interpret(snapshot)?;
        Ok((env, "mock".to_string()))
    }

    /// Whether this interpreter connects to a real external AI provider.
    fn is_real(&self) -> bool {
        false
    }
}

/// Configurable mock interpreter returning pre-configured envelopes or inference errors
#[derive(Debug, Clone)]
pub struct MockInterpreter {
    pub result: Result<Envelope, InferenceError>,
    pub call_count: Arc<AtomicUsize>,
}

impl MockInterpreter {
    pub fn new(result: Result<Envelope, InferenceError>) -> Self {
        Self {
            result,
            call_count: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// Convenience constructor for standard curriculum fixture: p=(0.85, 0.90), conf=0.90
    pub fn standard_fixture(snapshot_id: u64) -> Self {
        Self::new(Ok(Envelope {
            schema_version: 1,
            snapshot_id,
            interpretation: InterpretationPayload {
                p_persistent: 0.85,
                p_active: 0.90,
                confidence: 0.90,
            },
        }))
    }

    /// Number of times interpret() has been called
    pub fn call_count(&self) -> usize {
        self.call_count.load(Ordering::SeqCst)
    }
}

impl Interpreter for MockInterpreter {
    fn interpret(&self, _snapshot: &Snapshot) -> Result<Envelope, InferenceError> {
        self.call_count.fetch_add(1, Ordering::SeqCst);
        self.result.clone()
    }

    fn interpret_market(
        &self,
        snapshot: &Snapshot,
        _prices: &[f64],
        _features: Option<&Features>,
    ) -> Result<(Envelope, String), InferenceError> {
        let env = self.interpret(snapshot)?;
        Ok((env, "mock".to_string()))
    }

    fn is_real(&self) -> bool {
        false
    }
}

/// Load TypeSafe AI API key from environment variables or .env files
pub fn load_typesafe_api_key() -> Option<String> {
    if let Ok(key) = std::env::var("TYPESAFE_API_KEY") {
        if !key.trim().is_empty() {
            return Some(key.trim().to_string());
        }
    }
    if let Ok(key) = std::env::var("JEV_KEY") {
        if !key.trim().is_empty() {
            return Some(key.trim().to_string());
        }
    }
    for path in &[".env", "jevfloor_learning_day/.env", "../.env"] {
        if let Ok(content) = std::fs::read_to_string(path) {
            for line in content.lines() {
                let trimmed = line.trim();
                if let Some(rest) = trimmed
                    .strip_prefix("TYPESAFE_API_KEY=")
                    .or_else(|| trimmed.strip_prefix("JEV_KEY="))
                {
                    let val = rest.trim().trim_matches('"').trim_matches('\'').trim();
                    if !val.is_empty() {
                        return Some(val.to_string());
                    }
                }
            }
        }
    }
    None
}

/// Pure helper: parse the raw JSON string returned by TypeSafe System One API into an Envelope and model string.
pub fn parse_jev_response(
    json_str: &str,
    snapshot_id: u64,
) -> Result<(Envelope, String), InferenceError> {
    let val: serde_json::Value = serde_json::from_str(json_str)
        .map_err(|e| InferenceError::ValidationFailed(format!("Malformed Jev JSON: {e}")))?;

    if let Some(err_msg) = val.get("error").and_then(|e| e.as_str()) {
        return Err(InferenceError::ValidationFailed(format!("Jev API error: {err_msg}")));
    }

    let model = val
        .get("model")
        .and_then(|m| m.as_str())
        .unwrap_or("jev-unknown")
        .to_string();

    let answers = val.get("answers").ok_or_else(|| {
        InferenceError::ValidationFailed("Missing 'answers' object in Jev response".to_string())
    })?;

    let extract_noul = |name: &str| -> Result<f64, InferenceError> {
        let answer_obj = answers.get(name).ok_or_else(|| {
            InferenceError::ValidationFailed(format!("Missing answer for '{name}'"))
        })?;
        answer_obj
            .get("noul")
            .and_then(|v| v.as_f64())
            .ok_or_else(|| {
                InferenceError::ValidationFailed(format!(
                    "Field 'noul' missing or not f64 in answer '{name}'"
                ))
            })
    };

    let p_persistent = extract_noul("p_persistent")?;
    let p_active = extract_noul("p_active")?;
    let confidence = extract_noul("confidence")?;

    let env = Envelope {
        schema_version: 1,
        snapshot_id,
        interpretation: InterpretationPayload {
            p_persistent,
            p_active,
            confidence,
        },
    };

    validate_envelope(&env, snapshot_id)?;

    Ok((env, model))
}

/// Build the TypeSafe System One JSON payload from market prices and features
pub fn build_jev_payload(
    model: &str,
    prices: &[f64],
    features: Option<&Features>,
) -> serde_json::Value {
    let prices_str = prices
        .iter()
        .map(|p| format!("{:.3}", p))
        .collect::<Vec<_>>()
        .join(" -> ");
    let delta_desc = match features {
        Some(f) => format!("delta={:+.3}, positive_changes={}", f.delta, f.positive_changes),
        None => "insufficient price history".to_string(),
    };
    let state = format!("Market SILVER: prices [{}], {}", prices_str, delta_desc);

    serde_json::json!({
        "state": state,
        "model": model,
        "questions": {
            "p_persistent": {
                "type": "noul",
                "instructions": "Is this market price movement persistent and trending?"
            },
            "p_active": {
                "type": "noul",
                "instructions": "Is trading activity and volume dynamic or high?"
            },
            "confidence": {
                "type": "noul",
                "instructions": "Is this market signal clear, reliable, and high-confidence?"
            }
        }
    })
}

/// Real Live Provider calling TypeSafe System One HTTP API (/v1/systemone)
#[derive(Debug, Clone)]
pub struct RealJevInterpreter {
    pub api_key: String,
    pub endpoint_url: String,
    pub model: String,
    pub client: reqwest::blocking::Client,
}

impl RealJevInterpreter {
    pub fn new(api_key: String) -> Self {
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(15))
            .build()
            .expect("Failed to build HTTP client for RealJevInterpreter");
        Self {
            api_key,
            endpoint_url: "https://api.typesafe.ai/v1/systemone".to_string(),
            model: "jev-latest".to_string(),
            client,
        }
    }

    pub fn with_endpoint(api_key: String, endpoint_url: String) -> Self {
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(15))
            .build()
            .expect("Failed to build HTTP client for RealJevInterpreter");
        Self {
            api_key,
            endpoint_url,
            model: "jev-latest".to_string(),
            client,
        }
    }
}

impl Interpreter for RealJevInterpreter {
    fn interpret(&self, snapshot: &Snapshot) -> Result<Envelope, InferenceError> {
        self.interpret_market(snapshot, &[100.0, 101.0, 103.0], None)
            .map(|(env, _)| env)
    }

    fn interpret_market(
        &self,
        snapshot: &Snapshot,
        prices: &[f64],
        features: Option<&Features>,
    ) -> Result<(Envelope, String), InferenceError> {
        let payload = build_jev_payload(&self.model, prices, features);

        println!(">>> JEV INFERENCE: REAL (TypeSafe System One API: {})", self.endpoint_url);
        println!(">>> REQUEST SENT:");
        println!("    Endpoint: {}", self.endpoint_url);
        println!("    Model: {}", self.model);
        println!("    Authorization: Bearer [REDACTED_API_KEY]");
        println!("    State: {}", payload["state"]);
        println!("    Questions: p_persistent (noul), p_active (noul), confidence (noul)");

        let resp = self
            .client
            .post(&self.endpoint_url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&payload)
            .send()
            .map_err(|e| {
                if e.is_timeout() || e.is_connect() {
                    InferenceError::ProviderUnavailable
                } else {
                    InferenceError::ValidationFailed(e.to_string())
                }
            })?;

        let status = resp.status();
        let body = resp
            .text()
            .map_err(|e| InferenceError::ValidationFailed(e.to_string()))?;

        if !status.is_success() {
            return Err(InferenceError::ValidationFailed(format!(
                "HTTP {}: {}",
                status.as_u16(),
                body
            )));
        }

        println!(">>> RAW JEV RESPONSE:");
        println!("    {}", body.trim());

        parse_jev_response(&body, snapshot.id)
    }

    fn is_real(&self) -> bool {
        true
    }
}

// ============================================================================
// Part 4: Freshness Gate, Policy Engine & Strategy (Lessons 11, 20, 22, 24)
// ============================================================================

/// Visual behaviour returned to caller / UI
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VisualBehaviour {
    Hesitate,
    Walk,
    Run,
}

impl std::fmt::Display for VisualBehaviour {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VisualBehaviour::Hesitate => write!(f, "Hesitate"),
            VisualBehaviour::Walk => write!(f, "Walk"),
            VisualBehaviour::Run => write!(f, "Run"),
        }
    }
}

/// Display decision mapped deterministically from visual behaviour.
/// Note: These labels are informational display states, not order instructions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayDecision {
    Wait,
    Observe,
    Investigate,
}

impl DisplayDecision {
    /// Deterministic mapping: Run -> Investigate, Walk -> Observe, Hesitate -> Wait
    pub fn from_visual(visual: VisualBehaviour) -> Self {
        match visual {
            VisualBehaviour::Run => DisplayDecision::Investigate,
            VisualBehaviour::Walk => DisplayDecision::Observe,
            VisualBehaviour::Hesitate => DisplayDecision::Wait,
        }
    }
}

impl From<VisualBehaviour> for DisplayDecision {
    fn from(visual: VisualBehaviour) -> Self {
        Self::from_visual(visual)
    }
}

impl std::fmt::Display for DisplayDecision {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DisplayDecision::Wait => write!(f, "Wait"),
            DisplayDecision::Observe => write!(f, "Observe"),
            DisplayDecision::Investigate => write!(f, "Investigate"),
        }
    }
}

/// Freshness gate verifying timestamp sanity and age limit.
pub fn check_freshness(snapshot: &Snapshot, now_ms: u64, max_age_ms: u64) -> Result<u64, String> {
    if snapshot.observed_at_ms > now_ms {
        return Err(format!(
            "Future observation timestamp: observed_at_ms={} > now_ms={}",
            snapshot.observed_at_ms, now_ms
        ));
    }
    let age_ms = now_ms - snapshot.observed_at_ms;
    if age_ms > max_age_ms {
        return Err(format!(
            "Stale observation: age {}ms exceeds max {}ms",
            age_ms, max_age_ms
        ));
    }
    Ok(age_ms)
}

/// Configurable decision thresholds for policy tuning and unscripted testing (Part 7)
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PolicyThresholds {
    pub min_confidence: f64,
    pub min_p_persistent: f64,
    pub min_p_active: f64,
}

impl Default for PolicyThresholds {
    fn default() -> Self {
        Self {
            min_confidence: 0.60,
            min_p_persistent: 0.80,
            min_p_active: 0.80,
        }
    }
}

/// Deterministic policy function combining features, interpretation, and freshness.
/// Fails safe to `(VisualBehaviour::Hesitate, reason)` on any error, stale data,
/// missing features, or low confidence (< 0.60).
pub fn decide_behaviour(
    features: Option<&Features>,
    envelope: Result<&Envelope, &InferenceError>,
    snapshot: &Snapshot,
    now_ms: u64,
    max_age_ms: u64,
) -> (VisualBehaviour, String) {
    decide_behaviour_with_thresholds(
        features,
        envelope,
        snapshot,
        now_ms,
        max_age_ms,
        PolicyThresholds::default(),
    )
}

/// Policy function with configurable thresholds for unscripted experimentation (Part 7)
pub fn decide_behaviour_with_thresholds(
    features: Option<&Features>,
    envelope: Result<&Envelope, &InferenceError>,
    snapshot: &Snapshot,
    now_ms: u64,
    max_age_ms: u64,
    thresholds: PolicyThresholds,
) -> (VisualBehaviour, String) {
    // 1. Freshness check
    if let Err(reason) = check_freshness(snapshot, now_ms, max_age_ms) {
        return (VisualBehaviour::Hesitate, reason);
    }

    // 2. Inference error check
    let env = match envelope {
        Ok(env) => env,
        Err(err) => {
            return (VisualBehaviour::Hesitate, format!("Inference error: {err}"));
        }
    };

    // 3. Schema & snapshot correlation validation
    if let Err(err) = validate_envelope(env, snapshot.id) {
        return (VisualBehaviour::Hesitate, format!("Validation error: {err}"));
    }

    // 4. Feature availability check
    let feats = match features {
        Some(f) => f,
        None => {
            return (
                VisualBehaviour::Hesitate,
                "Insufficient price history for features".to_string(),
            );
        }
    };

    // 5. Delta polarity check (must be strictly positive)
    if feats.delta <= 0.0 {
        return (
            VisualBehaviour::Hesitate,
            format!("Non-positive price delta ({:.3})", feats.delta),
        );
    }

    // 6. Confidence gate
    if env.interpretation.confidence < thresholds.min_confidence {
        return (
            VisualBehaviour::Hesitate,
            format!(
                "Low confidence ({:.2} < {:.2})",
                env.interpretation.confidence, thresholds.min_confidence
            ),
        );
    }

    // 7. Probability classification
    if env.interpretation.p_persistent >= thresholds.min_p_persistent
        && env.interpretation.p_active >= thresholds.min_p_active
    {
        (
            VisualBehaviour::Run,
            "Strong trend and high activity".to_string(),
        )
    } else {
        (
            VisualBehaviour::Walk,
            "Positive delta with moderate probability".to_string(),
        )
    }
}

/// Lightweight Strategy trait consolidating Lesson 11.
pub trait Strategy {
    fn evaluate(
        &self,
        features: Option<&Features>,
        envelope: Result<&Envelope, &InferenceError>,
        snapshot: &Snapshot,
        now_ms: u64,
        max_age_ms: u64,
    ) -> (VisualBehaviour, DisplayDecision, String);
}

/// Default deterministic strategy evaluating the Lesson 22/24 policy.
#[derive(Debug, Default, Clone, Copy)]
pub struct DeterministicStrategy;

impl Strategy for DeterministicStrategy {
    fn evaluate(
        &self,
        features: Option<&Features>,
        envelope: Result<&Envelope, &InferenceError>,
        snapshot: &Snapshot,
        now_ms: u64,
        max_age_ms: u64,
    ) -> (VisualBehaviour, DisplayDecision, String) {
        let (visual, reason) = decide_behaviour(features, envelope, snapshot, now_ms, max_age_ms);
        let display = DisplayDecision::from_visual(visual);
        (visual, display, reason)
    }
}

/// Strategy with custom policy thresholds for unscripted tuning (Part 7)
#[derive(Debug, Clone, Copy)]
pub struct ConfigurableStrategy {
    pub thresholds: PolicyThresholds,
}

impl ConfigurableStrategy {
    pub fn new(thresholds: PolicyThresholds) -> Self {
        Self { thresholds }
    }
}

impl Strategy for ConfigurableStrategy {
    fn evaluate(
        &self,
        features: Option<&Features>,
        envelope: Result<&Envelope, &InferenceError>,
        snapshot: &Snapshot,
        now_ms: u64,
        max_age_ms: u64,
    ) -> (VisualBehaviour, DisplayDecision, String) {
        let (visual, reason) = decide_behaviour_with_thresholds(
            features,
            envelope,
            snapshot,
            now_ms,
            max_age_ms,
            self.thresholds,
        );
        let display = DisplayDecision::from_visual(visual);
        (visual, display, reason)
    }
}

// ============================================================================
// Part 5: Multi-Scenario CLI Runner (Lessons 11, 20, 22, 24, 25)
// ============================================================================

/// Scenario modes for local testing of pipeline branches
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScenarioMode {
    Strong,
    Mixed,
    Uncertain,
    Stale,
    Error,
    Empty,
}

impl ScenarioMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            ScenarioMode::Strong => "strong",
            ScenarioMode::Mixed => "mixed",
            ScenarioMode::Uncertain => "uncertain",
            ScenarioMode::Stale => "stale",
            ScenarioMode::Error => "error",
            ScenarioMode::Empty => "empty",
        }
    }

    pub fn config(&self) -> ScenarioConfig {
        let snapshot = Snapshot {
            id: 7,
            observed_at_ms: 1000,
        };
        let max_age_ms = 200;

        match self {
            ScenarioMode::Strong => ScenarioConfig {
                mode: *self,
                symbol: "SILVER".to_string(),
                ticks: vec![100.0, 101.0, 103.0],
                snapshot,
                now_ms: 1100,
                max_age_ms,
                interpretation_result: Ok(Envelope {
                    schema_version: 1,
                    snapshot_id: 7,
                    interpretation: InterpretationPayload {
                        p_persistent: 0.85,
                        p_active: 0.90,
                        confidence: 0.90,
                    },
                }),
            },
            ScenarioMode::Mixed => ScenarioConfig {
                mode: *self,
                symbol: "SILVER".to_string(),
                ticks: vec![100.0, 101.0, 103.0],
                snapshot,
                now_ms: 1100,
                max_age_ms,
                interpretation_result: Ok(Envelope {
                    schema_version: 1,
                    snapshot_id: 7,
                    interpretation: InterpretationPayload {
                        p_persistent: 0.65, // persistence below strong threshold (0.80)
                        p_active: 0.90,
                        confidence: 0.90,
                    },
                }),
            },
            ScenarioMode::Uncertain => ScenarioConfig {
                mode: *self,
                symbol: "SILVER".to_string(),
                ticks: vec![100.0, 101.0, 103.0],
                snapshot,
                now_ms: 1100,
                max_age_ms,
                interpretation_result: Ok(Envelope {
                    schema_version: 1,
                    snapshot_id: 7,
                    interpretation: InterpretationPayload {
                        p_persistent: 0.85,
                        p_active: 0.90,
                        confidence: 0.40, // confidence below threshold (0.60)
                    },
                }),
            },
            ScenarioMode::Stale => ScenarioConfig {
                mode: *self,
                symbol: "SILVER".to_string(),
                ticks: vec![100.0, 101.0, 103.0],
                snapshot,
                now_ms: 1301, // age = 301ms exceeds max 200ms
                max_age_ms,
                interpretation_result: Ok(Envelope {
                    schema_version: 1,
                    snapshot_id: 7,
                    interpretation: InterpretationPayload {
                        p_persistent: 0.85,
                        p_active: 0.90,
                        confidence: 0.90,
                    },
                }),
            },
            ScenarioMode::Error => ScenarioConfig {
                mode: *self,
                symbol: "SILVER".to_string(),
                ticks: vec![100.0, 101.0, 103.0],
                snapshot,
                now_ms: 1100,
                max_age_ms,
                interpretation_result: Err(InferenceError::ProviderUnavailable),
            },
            ScenarioMode::Empty => ScenarioConfig {
                mode: *self,
                symbol: "SILVER".to_string(),
                ticks: vec![], // 0 ticks supplied to buffer
                snapshot,
                now_ms: 1100,
                max_age_ms,
                interpretation_result: Ok(Envelope {
                    schema_version: 1,
                    snapshot_id: 7,
                    interpretation: InterpretationPayload {
                        p_persistent: 0.85,
                        p_active: 0.90,
                        confidence: 0.90,
                    },
                }),
            },
        }
    }
}

impl std::str::FromStr for ScenarioMode {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "strong" => Ok(ScenarioMode::Strong),
            "mixed" => Ok(ScenarioMode::Mixed),
            "uncertain" => Ok(ScenarioMode::Uncertain),
            "stale" => Ok(ScenarioMode::Stale),
            "error" => Ok(ScenarioMode::Error),
            "empty" => Ok(ScenarioMode::Empty),
            other => Err(format!(
                "Unknown scenario '{}'. Valid scenarios: strong, mixed, uncertain, stale, error, empty",
                other
            )),
        }
    }
}

/// Scenario configuration parameters
#[derive(Debug, Clone)]
pub struct ScenarioConfig {
    pub mode: ScenarioMode,
    pub symbol: String,
    pub ticks: Vec<f64>,
    pub snapshot: Snapshot,
    pub now_ms: u64,
    pub max_age_ms: u64,
    pub interpretation_result: Result<Envelope, InferenceError>,
}

/// Result of executing the full capstone pipeline
#[derive(Debug, Clone)]
pub struct ScenarioResult {
    pub mode: ScenarioMode,
    pub symbol: String,
    pub snapshot_id: u64,
    pub prices: Vec<f64>,
    pub features: Option<Features>,
    pub interpretation: Result<Envelope, InferenceError>,
    pub age_ms: u64,
    pub valid: bool,
    pub decision: DisplayDecision,
    pub visual: VisualBehaviour,
    pub reason: String,
    pub source: String,
    pub model_version: Option<String>,
}

impl ScenarioResult {
    /// Print compact output matching curriculum specification
    pub fn print_compact(&self) {
        if self.source == "REAL" {
            let model_str = self.model_version.as_deref().unwrap_or("jev-latest");
            println!(
                "source=REAL model={} symbol={} snapshot={}",
                model_str, self.symbol, self.snapshot_id
            );
        } else {
            println!(
                "source=MOCK symbol={} snapshot={}",
                self.symbol, self.snapshot_id
            );
        }
        let prices_display: Vec<String> = self
            .prices
            .iter()
            .map(|p| {
                if (p.fract() - 0.0).abs() < 1e-6 {
                    format!("{:.0}", p)
                } else {
                    format!("{:.3}", p)
                }
            })
            .collect();
        println!("prices=[{}]", prices_display.join(", "));
        if let Some(ref feats) = self.features {
            let delta_str = if (feats.delta.fract() - 0.0).abs() < 1e-6 {
                format!("{:.0}", feats.delta)
            } else {
                format!("{:.3}", feats.delta)
            };
            println!(
                "delta={} positive_changes={}",
                delta_str, feats.positive_changes
            );
        } else {
            println!("delta=N/A positive_changes=N/A");
        }
        match &self.interpretation {
            Ok(env) => {
                println!(
                    "p_persistent={:.2} p_active={:.2} confidence={:.2}",
                    env.interpretation.p_persistent,
                    env.interpretation.p_active,
                    env.interpretation.confidence
                );
            }
            Err(err) => {
                println!("provider_error={err}");
            }
        }
        println!("age_ms={} valid={}", self.age_ms, self.valid);
        println!("decision={}", self.decision);
        println!("visual={}", self.visual);
    }
}

/// Execute the capstone pipeline with customizable options
pub fn run_pipeline_with_options<I: Interpreter>(
    config: &ScenarioConfig,
    interpreter: &I,
    is_live: bool,
    timeframe: &str,
    pair_id: u32,
) -> Result<ScenarioResult, BridgeError> {
    let mode = if is_live {
        IngestionMode::live_ctrader_url(pair_id, timeframe, 3)
    } else {
        IngestionMode::SyntheticTicks(config.ticks.clone())
    };

    let mut buffer = RollingBuffer::new(3);
    let rx = spawn_tick_producer(mode, 2)?;

    for event in rx {
        match event {
            MarketEvent::Tick { price, .. } => {
                buffer.push(price);
            }
        }
    }

    let prices = buffer.as_slice().to_vec();
    let features = calculate_features(&prices);

    let (interpretation, model_version) = match interpreter.interpret_market(
        &config.snapshot,
        &prices,
        features.as_ref(),
    ) {
        Ok((env, model)) => (Ok(env), Some(model)),
        Err(err) => (Err(err), None),
    };

    let strategy = DeterministicStrategy;
    let (visual, decision, reason) = strategy.evaluate(
        features.as_ref(),
        interpretation.as_ref(),
        &config.snapshot,
        config.now_ms,
        config.max_age_ms,
    );

    let age_ms = config.now_ms.saturating_sub(config.snapshot.observed_at_ms);
    let is_fresh = check_freshness(&config.snapshot, config.now_ms, config.max_age_ms).is_ok();
    let is_schema_valid = interpretation
        .as_ref()
        .map_or(false, |env| validate_envelope(env, config.snapshot.id).is_ok());
    let valid = is_fresh && is_schema_valid;

    let source = if interpreter.is_real() {
        "REAL".to_string()
    } else {
        "MOCK".to_string()
    };

    Ok(ScenarioResult {
        mode: config.mode,
        symbol: config.symbol.clone(),
        snapshot_id: config.snapshot.id,
        prices,
        features,
        interpretation,
        age_ms,
        valid,
        decision,
        visual,
        reason,
        source,
        model_version,
    })
}

/// Execute the capstone pipeline with a given interpreter
pub fn run_pipeline_with_interpreter<I: Interpreter>(
    config: &ScenarioConfig,
    interpreter: &I,
    is_live: bool,
) -> Result<ScenarioResult, BridgeError> {
    run_pipeline_with_options(config, interpreter, is_live, "1m", 238)
}

/// Execute the capstone pipeline using MockInterpreter with configured fixture
pub fn run_pipeline(
    config: &ScenarioConfig,
    is_live: bool,
) -> Result<ScenarioResult, BridgeError> {
    let interpreter = MockInterpreter::new(config.interpretation_result.clone());
    run_pipeline_with_interpreter(config, &interpreter, is_live)
}

/// Command line configuration options for Capstone
#[derive(Debug, Clone, PartialEq)]
pub struct CliArgs {
    pub scenario: ScenarioMode,
    pub is_live: bool,
    pub is_real_jev: bool,
    pub timeframe: String,
    pub pair_id: u32,
}

impl Default for CliArgs {
    fn default() -> Self {
        Self {
            scenario: ScenarioMode::Strong,
            is_live: false,
            is_real_jev: false,
            timeframe: "1m".to_string(),
            pair_id: 238,
        }
    }
}

/// Parse command-line arguments supporting scenario modes, --live, --jev real|mock, --timeframe, --pair
pub fn parse_cli_args(args: &[String]) -> Result<CliArgs, String> {
    let mut cli = CliArgs::default();
    let mut scenario_set = false;
    let mut i = 0;

    while i < args.len() {
        let arg = &args[i];
        if arg == "--live" {
            cli.is_live = true;
            i += 1;
        } else if arg == "--real-jev" {
            cli.is_real_jev = true;
            i += 1;
        } else if arg == "--mock-jev" {
            cli.is_real_jev = false;
            i += 1;
        } else if arg == "--jev" {
            if i + 1 >= args.len() {
                return Err("--jev requires an argument ('real' or 'mock')".to_string());
            }
            let mode = &args[i + 1];
            if mode == "real" {
                cli.is_real_jev = true;
            } else if mode == "mock" {
                cli.is_real_jev = false;
            } else {
                return Err(format!("Unknown --jev mode '{mode}'. Use 'real' or 'mock'."));
            }
            i += 2;
        } else if arg == "--timeframe" {
            if i + 1 >= args.len() {
                return Err("--timeframe requires an argument (e.g. 1m, 5m)".to_string());
            }
            cli.timeframe = args[i + 1].clone();
            i += 2;
        } else if arg == "--pair" {
            if i + 1 >= args.len() {
                return Err("--pair requires an argument (e.g. 238)".to_string());
            }
            cli.pair_id = args[i + 1]
                .parse::<u32>()
                .map_err(|e| format!("Invalid pair id: {e}"))?;
            i += 2;
        } else if !arg.starts_with("--") {
            if scenario_set {
                return Err(format!(
                    "Multiple scenario arguments provided: already set to '{}', unexpected '{}'",
                    cli.scenario.as_str(),
                    arg
                ));
            }
            cli.scenario = arg.parse::<ScenarioMode>()?;
            scenario_set = true;
            i += 1;
        } else {
            return Err(format!("Unknown flag '{arg}'"));
        }
    }

    Ok(cli)
}

/// Parse command-line arguments into (ScenarioMode, is_live) for backward compatibility
pub fn parse_args(args: &[String]) -> Result<(ScenarioMode, bool), String> {
    let cli = parse_cli_args(args)?;
    Ok((cli.scenario, cli.is_live))
}

// ============================================================================
// Part 6: Deterministic Replay Engine (Lesson 25)
// ============================================================================

pub const CURRENT_POLICY_VERSION: u32 = 1;
pub const CURRENT_SCHEMA_VERSION: u32 = 1;

/// Recorded decision input for offline deterministic replay.
/// Bundles snapshot, features, recorded interpretation, logical time, schema version, and policy version.
#[derive(Debug, Clone, PartialEq)]
pub struct ReplayRecord {
    pub snapshot: Snapshot,
    pub features: Option<Features>,
    pub interpretation: Option<InterpretationPayload>,
    pub source: String,
    pub now_ms: u64,
    pub schema_version: u32,
    pub policy_version: u32,
}

impl ReplayRecord {
    /// Create a ReplayRecord from a pipeline scenario execution
    pub fn from_result(
        result: &ScenarioResult,
        now_ms: u64,
        policy_version: u32,
    ) -> Self {
        let (schema_v, payload) = match &result.interpretation {
            Ok(env) => (env.schema_version, Some(env.interpretation.clone())),
            Err(_) => (CURRENT_SCHEMA_VERSION, None),
        };
        Self {
            snapshot: Snapshot {
                id: result.snapshot_id,
                observed_at_ms: now_ms.saturating_sub(result.age_ms),
            },
            features: result.features.clone(),
            interpretation: payload,
            source: "recorded".to_string(),
            now_ms,
            schema_version: schema_v,
            policy_version,
        }
    }
}

/// Errors occurring during replay record validation
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplayError {
    UnsupportedPolicyVersion(u32),
    UnsupportedSchemaVersion(u32),
}

impl std::fmt::Display for ReplayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReplayError::UnsupportedPolicyVersion(v) => {
                write!(f, "Unsupported policy version: {v} (expected {CURRENT_POLICY_VERSION})")
            }
            ReplayError::UnsupportedSchemaVersion(v) => {
                write!(f, "Unsupported schema version: {v} (expected {CURRENT_SCHEMA_VERSION})")
            }
        }
    }
}

/// Pure deterministic replay evaluator.
/// Executes the policy engine without calling any provider.
pub fn evaluate_replay(
    record: &ReplayRecord,
    current_policy_version: u32,
    max_age_ms: u64,
) -> Result<(VisualBehaviour, DisplayDecision, String), ReplayError> {
    if record.schema_version != CURRENT_SCHEMA_VERSION {
        return Err(ReplayError::UnsupportedSchemaVersion(record.schema_version));
    }
    if record.policy_version != current_policy_version {
        return Err(ReplayError::UnsupportedPolicyVersion(record.policy_version));
    }

    let envelope_res: Result<Envelope, InferenceError> = match &record.interpretation {
        Some(interp) => Ok(Envelope {
            schema_version: record.schema_version,
            snapshot_id: record.snapshot.id,
            interpretation: interp.clone(),
        }),
        None => Err(InferenceError::ProviderUnavailable),
    };

    let (visual, reason) = decide_behaviour(
        record.features.as_ref(),
        envelope_res.as_ref(),
        &record.snapshot,
        record.now_ms,
        max_age_ms,
    );
    let decision = DisplayDecision::from_visual(visual);

    Ok((visual, decision, reason))
}

fn main() {
    let raw_args: Vec<String> = std::env::args().skip(1).collect();
    let cli = match parse_cli_args(&raw_args) {
        Ok(res) => res,
        Err(err) => {
            eprintln!("\n[ERROR] {err}");
            eprintln!("Usage: cargo run --example capstone -- [SCENARIO] [--live] [--jev real|mock] [--timeframe TF] [--pair ID]");
            eprintln!("Available scenarios: strong, mixed, uncertain, stale, error, empty\n");
            std::process::exit(1);
        }
    };

    println!("====================================================");
    println!("  JevFloor Capstone: Parts 1–7 — Unified Pipeline   ");
    println!("  Mode: {:<9} | Channel -> Buffer -> Jev -> Strategy", cli.scenario.as_str().to_uppercase());
    println!("====================================================\n");

    if cli.is_live {
        println!(">>> INGESTION: LIVE cTrader Bridge (pair {}, timeframe {})", cli.pair_id, cli.timeframe);
    } else {
        println!(">>> INGESTION: SYNTHETIC (Scenario '{}')", cli.scenario.as_str());
        println!("    (Pass --live to query local cTrader/Influx bridge)");
    }

    let config = cli.scenario.config();
    let result = if cli.is_real_jev {
        let api_key = match load_typesafe_api_key() {
            Some(key) => key,
            None => {
                eprintln!("\n[FATAL] Real Jev mode requested (--jev real), but no TYPESAFE_API_KEY or JEV_KEY found!");
                eprintln!("FAIL-FAST: Real Jev mode will NEVER silently fall back to mock fixtures.");
                std::process::exit(1);
            }
        };
        let interpreter = RealJevInterpreter::new(api_key);
        let res = match run_pipeline_with_options(&config, &interpreter, cli.is_live, &cli.timeframe, cli.pair_id) {
            Ok(r) => r,
            Err(err) => {
                eprintln!("\n[FATAL] Ingestion failed: {err}");
                eprintln!("When --live is specified, fallback to synthetic data is prohibited.");
                std::process::exit(1);
            }
        };

        if let Err(ref err) = res.interpretation {
            eprintln!("\n[FATAL] Real TypeSafe Jev inference failed: {err}");
            eprintln!("FAIL-FAST: Real Jev mode will NEVER silently fall back to mock fixtures.");
            std::process::exit(1);
        }

        res
    } else {
        match run_pipeline_with_options(
            &config,
            &MockInterpreter::new(config.interpretation_result.clone()),
            cli.is_live,
            &cli.timeframe,
            cli.pair_id,
        ) {
            Ok(res) => res,
            Err(err) => {
                eprintln!("\n[FATAL] Live ingestion failed: {err}");
                eprintln!("When --live is specified, fallback to synthetic data is prohibited.");
                std::process::exit(1);
            }
        }
    };

    println!("\n----------------------------------------------------");
    println!("   Compact Formatted Output                         ");
    println!("----------------------------------------------------");
    result.print_compact();
    println!("reason={}", result.reason);

    println!("\n----------------------------------------------------");
    println!("   Part 6: Deterministic Replay Engine              ");
    println!("----------------------------------------------------");
    let replay_record = ReplayRecord::from_result(&result, config.now_ms, CURRENT_POLICY_VERSION);
    let test_interpreter = MockInterpreter::standard_fixture(config.snapshot.id);
    let calls_before = test_interpreter.call_count();

    let replay1 = evaluate_replay(&replay_record, CURRENT_POLICY_VERSION, config.max_age_ms)
        .expect("Replay evaluation 1 should succeed");
    let replay2 = evaluate_replay(&replay_record, CURRENT_POLICY_VERSION, config.max_age_ms)
        .expect("Replay evaluation 2 should succeed");
    let calls_during_replay = test_interpreter.call_count() - calls_before;

    assert_eq!(replay1, replay2, "Replay outputs must be identical");
    assert_eq!(calls_during_replay, 0, "Provider call count during replay must be strictly 0");

    println!("Replay 1 result: visual={:<8} decision={:<11} (reason: {})", replay1.0, replay1.1, replay1.2);
    println!("Replay 2 result: visual={:<8} decision={:<11} (reason: {})", replay2.0, replay2.1, replay2.2);
    println!("Replay verification: replay1 == replay2 (DETERMINISTIC)");
    println!("Provider calls:  {} (strictly 0 calls made during replay)", calls_during_replay);

    println!("\n----------------------------------------------------");
    println!("   Part 7: Curriculum Exit & Threshold Tuning       ");
    println!("----------------------------------------------------");
    let relaxed_strategy = ConfigurableStrategy::new(PolicyThresholds {
        min_confidence: 0.35,
        ..PolicyThresholds::default()
    });
    let (tuned_visual, tuned_decision, tuned_reason) = relaxed_strategy.evaluate(
        result.features.as_ref(),
        result.interpretation.as_ref(),
        &config.snapshot,
        config.now_ms,
        config.max_age_ms,
    );
    println!(
        "Tuned threshold check (min_conf=0.35): visual={:<8} decision={:<11} (reason: {})",
        tuned_visual, tuned_decision, tuned_reason
    );
    println!("All curriculum exit criteria and acceptance tests verified successfully!");

    println!("\nCapstone pipeline (Parts 1–7) executed successfully for '{}'!", cli.scenario.as_str());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_buffer_starts_empty() {
        let buf = RollingBuffer::new(3);
        assert!(buf.is_empty());
        assert_eq!(buf.len(), 0);
        assert_eq!(buf.as_slice(), &[] as &[f64]);
        assert_eq!(buf.capacity(), 3);
    }

    #[test]
    fn test_rolling_buffer_eviction() {
        let mut buf = RollingBuffer::new(3);

        buf.push(10.0);
        assert_eq!(buf.as_slice(), &[10.0]);

        buf.push(20.0);
        assert_eq!(buf.as_slice(), &[10.0, 20.0]);

        buf.push(30.0);
        assert_eq!(buf.as_slice(), &[10.0, 20.0, 30.0]);

        // 4th insertion evicts the oldest item (10.0)
        buf.push(40.0);
        assert_eq!(buf.as_slice(), &[20.0, 30.0, 40.0]);
        assert_eq!(buf.len(), 3);

        // 5th insertion evicts 20.0
        buf.push(50.0);
        assert_eq!(buf.as_slice(), &[30.0, 40.0, 50.0]);
    }

    #[test]
    fn test_synthetic_channel_ingestion() {
        let rx = spawn_tick_producer(IngestionMode::Synthetic, 2).expect("Producer should spawn");
        let mut buf = RollingBuffer::new(3);

        for event in rx {
            let MarketEvent::Tick { price, .. } = event;
            buf.push(price);
        }

        assert_eq!(buf.len(), 3);
        assert_eq!(buf.as_slice(), &[100.0, 101.0, 103.0]);
    }

    #[test]
    fn test_feature_arithmetic() {
        let prices = [100.0, 101.0, 103.0];
        let features = calculate_features(&prices).expect("Features should calculate");
        assert_eq!(
            features,
            Features {
                delta: 3.0,
                positive_changes: 2,
            }
        );
    }

    #[test]
    fn test_empty_and_insufficient_features() {
        assert_eq!(calculate_features(&[]), None);
        assert_eq!(calculate_features(&[100.0]), None);
    }

    #[test]
    fn test_features_negative_delta() {
        let prices = [105.0, 102.0, 100.0];
        let features = calculate_features(&prices).expect("Features should calculate");
        assert_eq!(
            features,
            Features {
                delta: -5.0,
                positive_changes: 0,
            }
        );
    }

    #[test]
    fn test_features_flat() {
        let prices = [100.0, 100.0, 100.0];
        let features = calculate_features(&prices).expect("Features should calculate");
        assert_eq!(
            features,
            Features {
                delta: 0.0,
                positive_changes: 0,
            }
        );
    }

    #[test]
    fn test_features_two_elements() {
        let prices = [100.0, 105.0];
        let features = calculate_features(&prices).expect("Features should calculate");
        assert_eq!(
            features,
            Features {
                delta: 5.0,
                positive_changes: 1,
            }
        );
    }

    #[test]
    fn test_buffer_to_features_pipeline() {
        let mut buf = RollingBuffer::new(3);
        buf.push(90.0);
        buf.push(100.0);
        buf.push(101.0);
        buf.push(103.0); // 90.0 evicted

        assert_eq!(buf.as_slice(), &[100.0, 101.0, 103.0]);
        let features = calculate_features(buf.as_slice()).expect("Features should calculate");
        assert_eq!(
            features,
            Features {
                delta: 3.0,
                positive_changes: 2,
            }
        );
    }

    // --- Bridge Parsing & Safety Tests ---

    #[test]
    fn test_parse_valid_json() {
        let json = r#"{"data":[{"timestamp":1,"close":60.1},{"timestamp":2,"close":60.2},{"timestamp":3,"close":60.3}]}"#;
        let prices = parse_ctrader_response(json, 3).expect("Should parse valid JSON");
        assert_eq!(prices, vec![60.1, 60.2, 60.3]);
    }

    #[test]
    fn test_parse_fewer_than_requested_bars() {
        let json = r#"{"data":[{"timestamp":1,"close":60.1},{"timestamp":2,"close":60.2}]}"#;
        let prices = parse_ctrader_response(json, 3).expect("Should parse available bars");
        assert_eq!(prices, vec![60.1, 60.2]);
        assert_eq!(prices.len(), 2);
    }

    #[test]
    fn test_parse_empty_data() {
        let json = r#"{"data":[]}"#;
        let result = parse_ctrader_response(json, 3);
        assert_eq!(result, Err(BridgeError::EmptyData));
    }

    #[test]
    fn test_parse_error_payload() {
        let json = r#"{"error":"Pair not found","data":[]}"#;
        let result = parse_ctrader_response(json, 3);
        assert_eq!(
            result,
            Err(BridgeError::BridgeReportedError("Pair not found".to_string()))
        );
    }

    #[test]
    fn test_parse_malformed_json() {
        let json = r#"{"data": [ broken json"#;
        let result = parse_ctrader_response(json, 3);
        assert!(matches!(result, Err(BridgeError::MalformedJson(_))));
    }

    #[test]
    fn test_bridge_connection_refused() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener); // Close port immediately

        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_millis(500))
            .build()
            .unwrap();

        let url = format!("http://127.0.0.1:{port}/getData");
        let result = fetch_ctrader_prices(&client, &url, 3);
        assert!(matches!(result, Err(BridgeError::ConnectionFailed(_))));
    }

    #[test]
    fn test_bridge_http_non_200() {
        use std::io::{Read, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();

        let server_thread = thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut req_buf = [0u8; 1024];
                let _ = stream.read(&mut req_buf);
                let response = "HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.flush();
            }
        });

        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(1))
            .build()
            .unwrap();

        let url = format!("http://127.0.0.1:{port}/getData");
        let result = fetch_ctrader_prices(&client, &url, 3);
        assert_eq!(result, Err(BridgeError::HttpStatus(500)));

        server_thread.join().unwrap();
    }

    #[test]
    fn test_bridge_malformed_response() {
        use std::io::{Read, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();

        let server_thread = thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut req_buf = [0u8; 1024];
                let _ = stream.read(&mut req_buf);
                let body = "{not valid json";
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.flush();
            }
        });

        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(1))
            .build()
            .unwrap();

        let url = format!("http://127.0.0.1:{port}/getData");
        let result = fetch_ctrader_prices(&client, &url, 3);
        assert!(matches!(result, Err(BridgeError::MalformedJson(_))));

        server_thread.join().unwrap();
    }

    #[test]
    fn test_bridge_empty_data_response() {
        use std::io::{Read, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();

        let server_thread = thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut req_buf = [0u8; 1024];
                let _ = stream.read(&mut req_buf);
                let body = r#"{"data":[]}"#;
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.flush();
            }
        });

        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(1))
            .build()
            .unwrap();

        let url = format!("http://127.0.0.1:{port}/getData");
        let result = fetch_ctrader_prices(&client, &url, 3);
        assert_eq!(result, Err(BridgeError::EmptyData));

        server_thread.join().unwrap();
    }

    #[test]
    fn test_bridge_timeout() {
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();

        let server_thread = thread::spawn(move || {
            if let Ok((_stream, _)) = listener.accept() {
                // Sleep longer than client timeout without sending response
                thread::sleep(Duration::from_millis(300));
            }
        });

        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_millis(50))
            .build()
            .unwrap();

        let url = format!("http://127.0.0.1:{port}/getData");
        let result = fetch_ctrader_prices(&client, &url, 3);
        assert!(matches!(result, Err(BridgeError::Timeout(_))));

        server_thread.join().unwrap();
    }

    #[test]
    fn test_live_mode_failure_does_not_fallback_to_synthetic() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener); // Guarantee closed port

        let mode = IngestionMode::LiveCTrader {
            url: format!("http://127.0.0.1:{port}/getData"),
            bars_count: 3,
        };

        let result = spawn_tick_producer(mode, 2);
        // Live mode MUST return Err, NEVER silently fall back to synthetic channel
        assert!(matches!(result, Err(BridgeError::ConnectionFailed(_))));
    }

    // --- Part 3: Schema Validation & Interpreter Tests ---

    #[test]
    fn test_validate_envelope_valid() {
        let env = Envelope {
            schema_version: 1,
            snapshot_id: 7,
            interpretation: InterpretationPayload {
                p_persistent: 0.85,
                p_active: 0.90,
                confidence: 0.90,
            },
        };
        assert_eq!(validate_envelope(&env, 7), Ok(()));
    }

    #[test]
    fn test_schema_invalid_probabilities() {
        // Test > 1.0
        let env_high = Envelope {
            schema_version: 1,
            snapshot_id: 7,
            interpretation: InterpretationPayload {
                p_persistent: 1.20,
                p_active: 0.90,
                confidence: 0.90,
            },
        };
        assert!(matches!(
            validate_envelope(&env_high, 7),
            Err(InferenceError::ValidationFailed(_))
        ));

        // Test < 0.0
        let env_neg = Envelope {
            schema_version: 1,
            snapshot_id: 7,
            interpretation: InterpretationPayload {
                p_persistent: 0.85,
                p_active: -0.10,
                confidence: 0.90,
            },
        };
        assert!(matches!(
            validate_envelope(&env_neg, 7),
            Err(InferenceError::ValidationFailed(_))
        ));

        // Test NaN
        let env_nan = Envelope {
            schema_version: 1,
            snapshot_id: 7,
            interpretation: InterpretationPayload {
                p_persistent: 0.85,
                p_active: 0.90,
                confidence: f64::NAN,
            },
        };
        assert!(matches!(
            validate_envelope(&env_nan, 7),
            Err(InferenceError::ValidationFailed(_))
        ));
    }

    #[test]
    fn test_schema_wrong_version() {
        let env = Envelope {
            schema_version: 2,
            snapshot_id: 7,
            interpretation: InterpretationPayload {
                p_persistent: 0.85,
                p_active: 0.90,
                confidence: 0.90,
            },
        };
        assert_eq!(validate_envelope(&env, 7), Err(InferenceError::InvalidSchema));
    }

    #[test]
    fn test_snapshot_mismatch_rejection() {
        let env = Envelope {
            schema_version: 1,
            snapshot_id: 8, // Expecting 7
            interpretation: InterpretationPayload {
                p_persistent: 0.85,
                p_active: 0.90,
                confidence: 0.90,
            },
        };
        assert!(matches!(
            validate_envelope(&env, 7),
            Err(InferenceError::ValidationFailed(_))
        ));
    }

    #[test]
    fn test_mock_interpreter_standard_fixture() {
        let snapshot = Snapshot {
            id: 7,
            observed_at_ms: 1000,
        };
        let interpreter = MockInterpreter::standard_fixture(snapshot.id);
        let env = interpreter.interpret(&snapshot).expect("Should succeed");
        assert_eq!(env.snapshot_id, 7);
        assert_eq!(env.schema_version, 1);
        assert_eq!(env.interpretation.p_persistent, 0.85);
        assert_eq!(env.interpretation.p_active, 0.90);
        assert_eq!(env.interpretation.confidence, 0.90);
    }

    #[test]
    fn test_provider_failure_fallback() {
        let snapshot = Snapshot {
            id: 7,
            observed_at_ms: 1000,
        };
        let interpreter = MockInterpreter::new(Err(InferenceError::ProviderUnavailable));
        assert_eq!(
            interpreter.interpret(&snapshot),
            Err(InferenceError::ProviderUnavailable)
        );
    }

    #[test]
    fn test_boundary_probabilities() {
        let env = Envelope {
            schema_version: 1,
            snapshot_id: 7,
            interpretation: InterpretationPayload {
                p_persistent: 0.0,
                p_active: 1.0,
                confidence: 1.0,
            },
        };
        assert_eq!(validate_envelope(&env, 7), Ok(()));
    }

    // ========================================================================
    // Part 4 Unit Tests: Freshness Gate, Policy Engine & Strategy
    // ========================================================================

    #[test]
    fn test_display_decision_mapping() {
        assert_eq!(
            DisplayDecision::from_visual(VisualBehaviour::Run),
            DisplayDecision::Investigate
        );
        assert_eq!(
            DisplayDecision::from_visual(VisualBehaviour::Walk),
            DisplayDecision::Observe
        );
        assert_eq!(
            DisplayDecision::from_visual(VisualBehaviour::Hesitate),
            DisplayDecision::Wait
        );

        assert_eq!(
            DisplayDecision::from(VisualBehaviour::Run),
            DisplayDecision::Investigate
        );
        assert_eq!(
            DisplayDecision::from(VisualBehaviour::Walk),
            DisplayDecision::Observe
        );
        assert_eq!(
            DisplayDecision::from(VisualBehaviour::Hesitate),
            DisplayDecision::Wait
        );
    }

    #[test]
    fn test_freshness_gate_cases() {
        let snapshot = Snapshot {
            id: 7,
            observed_at_ms: 1000,
        };

        // Fresh within limit (1100 - 1000 = 100ms <= 200ms)
        assert_eq!(check_freshness(&snapshot, 1100, 200), Ok(100));

        // Exact boundary (1200 - 1000 = 200ms == 200ms) is accepted
        assert_eq!(check_freshness(&snapshot, 1200, 200), Ok(200));

        // Stale (1201 - 1000 = 201ms > 200ms)
        assert!(check_freshness(&snapshot, 1201, 200).is_err());

        // Future timestamp (999 < 1000)
        assert!(check_freshness(&snapshot, 999, 200).is_err());
    }

    #[test]
    fn test_policy_strong_branch() {
        let snapshot = Snapshot {
            id: 7,
            observed_at_ms: 1000,
        };
        let features = Features {
            delta: 3.0,
            positive_changes: 2,
        };
        let env = Envelope {
            schema_version: 1,
            snapshot_id: 7,
            interpretation: InterpretationPayload {
                p_persistent: 0.85,
                p_active: 0.90,
                confidence: 0.90,
            },
        };

        let (visual, reason) = decide_behaviour(
            Some(&features),
            Ok(&env),
            &snapshot,
            1100,
            200,
        );
        assert_eq!(visual, VisualBehaviour::Run);
        assert!(reason.contains("Strong trend and high activity"));
        assert_eq!(DisplayDecision::from(visual), DisplayDecision::Investigate);
    }

    #[test]
    fn test_policy_mixed_branch() {
        let snapshot = Snapshot {
            id: 7,
            observed_at_ms: 1000,
        };
        let features = Features {
            delta: 3.0,
            positive_changes: 2,
        };
        let env = Envelope {
            schema_version: 1,
            snapshot_id: 7,
            interpretation: InterpretationPayload {
                p_persistent: 0.65, // < 0.80
                p_active: 0.90,
                confidence: 0.90,
            },
        };

        let (visual, reason) = decide_behaviour(
            Some(&features),
            Ok(&env),
            &snapshot,
            1100,
            200,
        );
        assert_eq!(visual, VisualBehaviour::Walk);
        assert!(reason.contains("Positive delta with moderate probability"));
        assert_eq!(DisplayDecision::from(visual), DisplayDecision::Observe);
    }

    #[test]
    fn test_policy_low_confidence_gate() {
        let snapshot = Snapshot {
            id: 7,
            observed_at_ms: 1000,
        };
        let features = Features {
            delta: 3.0,
            positive_changes: 2,
        };
        let env = Envelope {
            schema_version: 1,
            snapshot_id: 7,
            interpretation: InterpretationPayload {
                p_persistent: 0.85,
                p_active: 0.90,
                confidence: 0.40, // < 0.60
            },
        };

        let (visual, reason) = decide_behaviour(
            Some(&features),
            Ok(&env),
            &snapshot,
            1100,
            200,
        );
        assert_eq!(visual, VisualBehaviour::Hesitate);
        assert!(reason.contains("Low confidence"));
        assert_eq!(DisplayDecision::from(visual), DisplayDecision::Wait);
    }

    #[test]
    fn test_freshness_exact_boundary() {
        let snapshot = Snapshot {
            id: 7,
            observed_at_ms: 1000,
        };
        let features = Features {
            delta: 3.0,
            positive_changes: 2,
        };
        let env = Envelope {
            schema_version: 1,
            snapshot_id: 7,
            interpretation: InterpretationPayload {
                p_persistent: 0.85,
                p_active: 0.90,
                confidence: 0.90,
            },
        };

        // Exactly 200ms age (1200 - 1000 = 200) -> accepted as fresh
        let (visual, _) = decide_behaviour(
            Some(&features),
            Ok(&env),
            &snapshot,
            1200,
            200,
        );
        assert_eq!(visual, VisualBehaviour::Run);
    }

    #[test]
    fn test_freshness_stale_rejection() {
        let snapshot = Snapshot {
            id: 7,
            observed_at_ms: 1000,
        };
        let features = Features {
            delta: 3.0,
            positive_changes: 2,
        };
        let env = Envelope {
            schema_version: 1,
            snapshot_id: 7,
            interpretation: InterpretationPayload {
                p_persistent: 0.85,
                p_active: 0.90,
                confidence: 0.90,
            },
        };

        // 201ms age (1201 - 1000 = 201) -> rejected as stale
        let (visual, reason) = decide_behaviour(
            Some(&features),
            Ok(&env),
            &snapshot,
            1201,
            200,
        );
        assert_eq!(visual, VisualBehaviour::Hesitate);
        assert!(reason.contains("Stale observation"));
    }

    #[test]
    fn test_freshness_future_timestamp_rejection() {
        let snapshot = Snapshot {
            id: 7,
            observed_at_ms: 1200,
        };
        let features = Features {
            delta: 3.0,
            positive_changes: 2,
        };
        let env = Envelope {
            schema_version: 1,
            snapshot_id: 7,
            interpretation: InterpretationPayload {
                p_persistent: 0.85,
                p_active: 0.90,
                confidence: 0.90,
            },
        };

        let (visual, reason) = decide_behaviour(
            Some(&features),
            Ok(&env),
            &snapshot,
            1100, // now < observed_at_ms
            200,
        );
        assert_eq!(visual, VisualBehaviour::Hesitate);
        assert!(reason.contains("Future observation timestamp"));
    }

    #[test]
    fn test_policy_nonpositive_delta() {
        let snapshot = Snapshot {
            id: 7,
            observed_at_ms: 1000,
        };
        let flat_features = Features {
            delta: 0.0,
            positive_changes: 0,
        };
        let neg_features = Features {
            delta: -1.5,
            positive_changes: 0,
        };
        let env = Envelope {
            schema_version: 1,
            snapshot_id: 7,
            interpretation: InterpretationPayload {
                p_persistent: 0.85,
                p_active: 0.90,
                confidence: 0.90,
            },
        };

        let (visual1, reason1) = decide_behaviour(
            Some(&flat_features),
            Ok(&env),
            &snapshot,
            1100,
            200,
        );
        assert_eq!(visual1, VisualBehaviour::Hesitate);
        assert!(reason1.contains("Non-positive price delta"));

        let (visual2, reason2) = decide_behaviour(
            Some(&neg_features),
            Ok(&env),
            &snapshot,
            1100,
            200,
        );
        assert_eq!(visual2, VisualBehaviour::Hesitate);
        assert!(reason2.contains("Non-positive price delta"));
    }

    #[test]
    fn test_policy_missing_features() {
        let snapshot = Snapshot {
            id: 7,
            observed_at_ms: 1000,
        };
        let env = Envelope {
            schema_version: 1,
            snapshot_id: 7,
            interpretation: InterpretationPayload {
                p_persistent: 0.85,
                p_active: 0.90,
                confidence: 0.90,
            },
        };

        let (visual, reason) = decide_behaviour(
            None,
            Ok(&env),
            &snapshot,
            1100,
            200,
        );
        assert_eq!(visual, VisualBehaviour::Hesitate);
        assert!(reason.contains("Insufficient price history for features"));
    }

    #[test]
    fn test_strategy_trait_evaluation() {
        let strategy = DeterministicStrategy;
        let snapshot = Snapshot {
            id: 7,
            observed_at_ms: 1000,
        };
        let features = Features {
            delta: 3.0,
            positive_changes: 2,
        };
        let env = Envelope {
            schema_version: 1,
            snapshot_id: 7,
            interpretation: InterpretationPayload {
                p_persistent: 0.85,
                p_active: 0.90,
                confidence: 0.90,
            },
        };

        // Strong evaluation
        let (visual, decision, reason) = strategy.evaluate(
            Some(&features),
            Ok(&env),
            &snapshot,
            1100,
            200,
        );
        assert_eq!(visual, VisualBehaviour::Run);
        assert_eq!(decision, DisplayDecision::Investigate);
        assert!(reason.contains("Strong trend and high activity"));

        // Error evaluation
        let err = InferenceError::ProviderUnavailable;
        let (visual_err, decision_err, reason_err) = strategy.evaluate(
            Some(&features),
            Err(&err),
            &snapshot,
            1100,
            200,
        );
        assert_eq!(visual_err, VisualBehaviour::Hesitate);
        assert_eq!(decision_err, DisplayDecision::Wait);
        assert!(reason_err.contains("Inference error"));
    }

    // ========================================================================
    // Part 5 Unit Tests: Multi-Scenario CLI Runner
    // ========================================================================

    #[test]
    fn test_scenario_mode_parsing() {
        assert_eq!("strong".parse::<ScenarioMode>(), Ok(ScenarioMode::Strong));
        assert_eq!("mixed".parse::<ScenarioMode>(), Ok(ScenarioMode::Mixed));
        assert_eq!("uncertain".parse::<ScenarioMode>(), Ok(ScenarioMode::Uncertain));
        assert_eq!("stale".parse::<ScenarioMode>(), Ok(ScenarioMode::Stale));
        assert_eq!("error".parse::<ScenarioMode>(), Ok(ScenarioMode::Error));
        assert_eq!("empty".parse::<ScenarioMode>(), Ok(ScenarioMode::Empty));

        // Case insensitivity
        assert_eq!("STRONG".parse::<ScenarioMode>(), Ok(ScenarioMode::Strong));
        assert_eq!("Mixed".parse::<ScenarioMode>(), Ok(ScenarioMode::Mixed));

        // Invalid mode
        assert!("invalid_mode".parse::<ScenarioMode>().is_err());

        // parse_args helper
        assert_eq!(
            parse_args(&["mixed".to_string()]),
            Ok((ScenarioMode::Mixed, false))
        );
        assert_eq!(
            parse_args(&["--live".to_string(), "stale".to_string()]),
            Ok((ScenarioMode::Stale, true))
        );
        assert_eq!(
            parse_args(&[]),
            Ok((ScenarioMode::Strong, false))
        );
    }

    #[test]
    fn test_scenario_pipeline_strong() {
        let config = ScenarioMode::Strong.config();
        let res = run_pipeline(&config, false).expect("Pipeline should succeed");
        assert_eq!(res.mode, ScenarioMode::Strong);
        assert_eq!(res.prices, vec![100.0, 101.0, 103.0]);
        let feats = res.features.expect("Features should be present");
        assert_eq!(feats.delta, 3.0);
        assert_eq!(feats.positive_changes, 2);
        assert_eq!(res.age_ms, 100);
        assert!(res.valid);
        assert_eq!(res.decision, DisplayDecision::Investigate);
        assert_eq!(res.visual, VisualBehaviour::Run);
        assert!(res.reason.contains("Strong trend and high activity"));
    }

    #[test]
    fn test_scenario_pipeline_mixed() {
        let config = ScenarioMode::Mixed.config();
        let res = run_pipeline(&config, false).expect("Pipeline should succeed");
        assert_eq!(res.mode, ScenarioMode::Mixed);
        assert_eq!(res.prices, vec![100.0, 101.0, 103.0]);
        assert_eq!(res.age_ms, 100);
        assert!(res.valid);
        assert_eq!(res.decision, DisplayDecision::Observe);
        assert_eq!(res.visual, VisualBehaviour::Walk);
        assert!(res.reason.contains("Positive delta with moderate probability"));
    }

    #[test]
    fn test_scenario_pipeline_uncertain() {
        let config = ScenarioMode::Uncertain.config();
        let res = run_pipeline(&config, false).expect("Pipeline should succeed");
        assert_eq!(res.mode, ScenarioMode::Uncertain);
        assert_eq!(res.age_ms, 100);
        assert!(res.valid); // schema is valid
        assert_eq!(res.decision, DisplayDecision::Wait);
        assert_eq!(res.visual, VisualBehaviour::Hesitate);
        assert!(res.reason.contains("Low confidence"));
    }

    #[test]
    fn test_scenario_pipeline_stale() {
        let config = ScenarioMode::Stale.config();
        let res = run_pipeline(&config, false).expect("Pipeline should succeed");
        assert_eq!(res.mode, ScenarioMode::Stale);
        assert_eq!(res.age_ms, 301);
        assert!(!res.valid); // stale -> not valid
        assert_eq!(res.decision, DisplayDecision::Wait);
        assert_eq!(res.visual, VisualBehaviour::Hesitate);
        assert!(res.reason.contains("Stale observation"));
    }

    #[test]
    fn test_scenario_pipeline_error() {
        let config = ScenarioMode::Error.config();
        let res = run_pipeline(&config, false).expect("Pipeline should succeed");
        assert_eq!(res.mode, ScenarioMode::Error);
        assert!(!res.valid); // provider error -> not valid
        assert_eq!(res.decision, DisplayDecision::Wait);
        assert_eq!(res.visual, VisualBehaviour::Hesitate);
        assert!(res.reason.contains("Inference error"));
    }

    #[test]
    fn test_scenario_pipeline_empty() {
        let config = ScenarioMode::Empty.config();
        let res = run_pipeline(&config, false).expect("Pipeline should succeed");
        assert_eq!(res.mode, ScenarioMode::Empty);
        assert!(res.prices.is_empty());
        assert!(res.features.is_none());
        assert_eq!(res.decision, DisplayDecision::Wait);
        assert_eq!(res.visual, VisualBehaviour::Hesitate);
        assert!(res.reason.contains("Insufficient price history"));
    }

    // ========================================================================
    // Part 6 Unit Tests: Deterministic Replay Engine
    // ========================================================================

    #[test]
    fn test_deterministic_replay_zero_calls() {
        let interpreter = MockInterpreter::standard_fixture(7);
        let record = ReplayRecord {
            snapshot: Snapshot {
                id: 7,
                observed_at_ms: 1000,
            },
            features: Some(Features {
                delta: 3.0,
                positive_changes: 2,
            }),
            interpretation: Some(InterpretationPayload {
                p_persistent: 0.85,
                p_active: 0.90,
                confidence: 0.90,
            }),
            source: "recorded".to_string(),
            now_ms: 1100,
            schema_version: 1,
            policy_version: 1,
        };

        let initial_calls = interpreter.call_count();
        let replay1 = evaluate_replay(&record, CURRENT_POLICY_VERSION, 200).expect("Replay 1 should succeed");
        let replay2 = evaluate_replay(&record, CURRENT_POLICY_VERSION, 200).expect("Replay 2 should succeed");
        let final_calls = interpreter.call_count();

        // 1. Identical outputs
        assert_eq!(replay1, replay2);
        assert_eq!(replay1.0, VisualBehaviour::Run);
        assert_eq!(replay1.1, DisplayDecision::Investigate);
        assert!(replay1.2.contains("Strong trend and high activity"));

        // 2. Exactly zero provider calls made
        assert_eq!(final_calls - initial_calls, 0);
    }

    #[test]
    fn test_replay_stale_data() {
        let record = ReplayRecord {
            snapshot: Snapshot {
                id: 7,
                observed_at_ms: 1000,
            },
            features: Some(Features {
                delta: 3.0,
                positive_changes: 2,
            }),
            interpretation: Some(InterpretationPayload {
                p_persistent: 0.85,
                p_active: 0.90,
                confidence: 0.90,
            }),
            source: "recorded".to_string(),
            now_ms: 1301, // 301ms age > 200ms
            schema_version: 1,
            policy_version: 1,
        };

        let res = evaluate_replay(&record, CURRENT_POLICY_VERSION, 200).expect("Stale evaluation succeeds with Hesitate");
        assert_eq!(res.0, VisualBehaviour::Hesitate);
        assert_eq!(res.1, DisplayDecision::Wait);
        assert!(res.2.contains("Stale observation"));
    }

    #[test]
    fn test_replay_unsupported_policy_version() {
        let record = ReplayRecord {
            snapshot: Snapshot {
                id: 7,
                observed_at_ms: 1000,
            },
            features: Some(Features {
                delta: 3.0,
                positive_changes: 2,
            }),
            interpretation: Some(InterpretationPayload {
                p_persistent: 0.85,
                p_active: 0.90,
                confidence: 0.90,
            }),
            source: "recorded".to_string(),
            now_ms: 1100,
            schema_version: 1,
            policy_version: 2, // Unsupported policy version
        };

        let res = evaluate_replay(&record, CURRENT_POLICY_VERSION, 200);
        assert_eq!(res, Err(ReplayError::UnsupportedPolicyVersion(2)));
    }

    #[test]
    fn test_replay_unsupported_schema_version() {
        let record = ReplayRecord {
            snapshot: Snapshot {
                id: 7,
                observed_at_ms: 1000,
            },
            features: Some(Features {
                delta: 3.0,
                positive_changes: 2,
            }),
            interpretation: Some(InterpretationPayload {
                p_persistent: 0.85,
                p_active: 0.90,
                confidence: 0.90,
            }),
            source: "recorded".to_string(),
            now_ms: 1100,
            schema_version: 2, // Unsupported schema version
            policy_version: 1,
        };

        let res = evaluate_replay(&record, CURRENT_POLICY_VERSION, 200);
        assert_eq!(res, Err(ReplayError::UnsupportedSchemaVersion(2)));
    }

    #[test]
    fn test_replay_missing_features() {
        let record = ReplayRecord {
            snapshot: Snapshot {
                id: 7,
                observed_at_ms: 1000,
            },
            features: None,
            interpretation: Some(InterpretationPayload {
                p_persistent: 0.85,
                p_active: 0.90,
                confidence: 0.90,
            }),
            source: "recorded".to_string(),
            now_ms: 1100,
            schema_version: 1,
            policy_version: 1,
        };

        let res = evaluate_replay(&record, CURRENT_POLICY_VERSION, 200).expect("Should evaluate to Hesitate");
        assert_eq!(res.0, VisualBehaviour::Hesitate);
        assert_eq!(res.1, DisplayDecision::Wait);
        assert!(res.2.contains("Insufficient price history"));
    }

    #[test]
    fn test_replay_provider_failure_payload() {
        let record = ReplayRecord {
            snapshot: Snapshot {
                id: 7,
                observed_at_ms: 1000,
            },
            features: Some(Features {
                delta: 3.0,
                positive_changes: 2,
            }),
            interpretation: None, // Provider failed during live capture
            source: "recorded".to_string(),
            now_ms: 1100,
            schema_version: 1,
            policy_version: 1,
        };

        let res = evaluate_replay(&record, CURRENT_POLICY_VERSION, 200).expect("Should evaluate to Hesitate");
        assert_eq!(res.0, VisualBehaviour::Hesitate);
        assert_eq!(res.1, DisplayDecision::Wait);
        assert!(res.2.contains("Inference error"));
    }

    // ========================================================================
    // Part 7 Unit Tests: Configurable Policy Thresholds (Unscripted Tests)
    // ========================================================================

    #[test]
    fn test_default_thresholds_match_standard_policy() {
        let snapshot = Snapshot {
            id: 7,
            observed_at_ms: 1000,
        };
        let features = Features {
            delta: 3.0,
            positive_changes: 2,
        };
        let env = Envelope {
            schema_version: 1,
            snapshot_id: 7,
            interpretation: InterpretationPayload {
                p_persistent: 0.85,
                p_active: 0.90,
                confidence: 0.90,
            },
        };

        let std_result = decide_behaviour(Some(&features), Ok(&env), &snapshot, 1100, 200);
        let custom_result = decide_behaviour_with_thresholds(
            Some(&features),
            Ok(&env),
            &snapshot,
            1100,
            200,
            PolicyThresholds::default(),
        );

        assert_eq!(std_result, custom_result);
        assert_eq!(std_result.0, VisualBehaviour::Run);
    }

    #[test]
    fn test_unscripted_threshold_relaxation_turns_uncertain_to_run() {
        let snapshot = Snapshot {
            id: 7,
            observed_at_ms: 1000,
        };
        let features = Features {
            delta: 3.0,
            positive_changes: 2,
        };
        // Fixture with low confidence (0.40)
        let env = Envelope {
            schema_version: 1,
            snapshot_id: 7,
            interpretation: InterpretationPayload {
                p_persistent: 0.85,
                p_active: 0.90,
                confidence: 0.40,
            },
        };

        // Standard threshold (0.60) -> Hesitate
        let (std_visual, _) = decide_behaviour(Some(&features), Ok(&env), &snapshot, 1100, 200);
        assert_eq!(std_visual, VisualBehaviour::Hesitate);

        // Relaxed confidence threshold (0.35) -> Run!
        let relaxed_thresholds = PolicyThresholds {
            min_confidence: 0.35,
            ..PolicyThresholds::default()
        };
        let (relaxed_visual, reason) = decide_behaviour_with_thresholds(
            Some(&features),
            Ok(&env),
            &snapshot,
            1100,
            200,
            relaxed_thresholds,
        );
        assert_eq!(relaxed_visual, VisualBehaviour::Run);
        assert!(reason.contains("Strong trend and high activity"));
    }

    #[test]
    fn test_unscripted_threshold_tightening_turns_strong_to_walk() {
        let snapshot = Snapshot {
            id: 7,
            observed_at_ms: 1000,
        };
        let features = Features {
            delta: 3.0,
            positive_changes: 2,
        };
        let env = Envelope {
            schema_version: 1,
            snapshot_id: 7,
            interpretation: InterpretationPayload {
                p_persistent: 0.85,
                p_active: 0.90,
                confidence: 0.90,
            },
        };

        // Standard threshold (p >= 0.80) -> Run
        let (std_visual, _) = decide_behaviour(Some(&features), Ok(&env), &snapshot, 1100, 200);
        assert_eq!(std_visual, VisualBehaviour::Run);

        // Tightened persistence threshold (0.90) -> Walk (since 0.85 < 0.90)
        let strict_thresholds = PolicyThresholds {
            min_p_persistent: 0.90,
            ..PolicyThresholds::default()
        };
        let (strict_visual, reason) = decide_behaviour_with_thresholds(
            Some(&features),
            Ok(&env),
            &snapshot,
            1100,
            200,
            strict_thresholds,
        );
        assert_eq!(strict_visual, VisualBehaviour::Walk);
        assert!(reason.contains("Positive delta with moderate probability"));
    }

    #[test]
    fn test_configurable_strategy_evaluation() {
        let snapshot = Snapshot {
            id: 7,
            observed_at_ms: 1000,
        };
        let features = Features {
            delta: 3.0,
            positive_changes: 2,
        };
        let env = Envelope {
            schema_version: 1,
            snapshot_id: 7,
            interpretation: InterpretationPayload {
                p_persistent: 0.85,
                p_active: 0.90,
                confidence: 0.40,
            },
        };

        let strategy = ConfigurableStrategy::new(PolicyThresholds {
            min_confidence: 0.35,
            ..PolicyThresholds::default()
        });

        let (visual, decision, reason) = strategy.evaluate(
            Some(&features),
            Ok(&env),
            &snapshot,
            1100,
            200,
        );
        assert_eq!(visual, VisualBehaviour::Run);
        assert_eq!(decision, DisplayDecision::Investigate);
        assert!(reason.contains("Strong trend and high activity"));
    }

    #[test]
    fn test_parse_jev_response_valid() {
        let json_payload = r#"{
            "model": "jev-1.13.0",
            "answers": {
                "p_persistent": { "type": "noul", "noul": 0.85 },
                "p_active": { "type": "noul", "noul": 0.90 },
                "confidence": { "type": "noul", "noul": 0.92 }
            },
            "usage": { "input_tokens": 300, "output_tokens": 50 }
        }"#;

        let (env, model) = parse_jev_response(json_payload, 7).expect("Parsing should succeed");
        assert_eq!(model, "jev-1.13.0");
        assert_eq!(env.schema_version, 1);
        assert_eq!(env.snapshot_id, 7);
        assert!((env.interpretation.p_persistent - 0.85).abs() < 1e-6);
        assert!((env.interpretation.p_active - 0.90).abs() < 1e-6);
        assert!((env.interpretation.confidence - 0.92).abs() < 1e-6);
    }

    #[test]
    fn test_parse_jev_response_malformed_json() {
        let res = parse_jev_response("not valid json", 7);
        match res {
            Err(InferenceError::ValidationFailed(msg)) => {
                assert!(msg.contains("Malformed Jev JSON"));
            }
            other => panic!("Expected ValidationFailed, got {other:?}"),
        }
    }

    #[test]
    fn test_parse_jev_response_missing_fields() {
        let missing_answer = r#"{
            "model": "jev-1.13.0",
            "answers": {
                "p_persistent": { "type": "noul", "noul": 0.85 }
            }
        }"#;
        let res = parse_jev_response(missing_answer, 7);
        assert!(matches!(res, Err(InferenceError::ValidationFailed(_))));

        let missing_answers_obj = r#"{"model": "jev-1.13.0"}"#;
        let res2 = parse_jev_response(missing_answers_obj, 7);
        assert!(matches!(res2, Err(InferenceError::ValidationFailed(_))));
    }

    #[test]
    fn test_parse_jev_response_invalid_probability() {
        let out_of_bounds = r#"{
            "model": "jev-1.13.0",
            "answers": {
                "p_persistent": { "type": "noul", "noul": 1.50 },
                "p_active": { "type": "noul", "noul": 0.90 },
                "confidence": { "type": "noul", "noul": 0.90 }
            }
        }"#;
        let res = parse_jev_response(out_of_bounds, 7);
        match res {
            Err(InferenceError::ValidationFailed(msg)) => {
                assert!(msg.contains("out of bounds"));
            }
            other => panic!("Expected ValidationFailed, got {other:?}"),
        }
    }

    #[test]
    fn test_parse_jev_response_api_error_payload() {
        let err_json = r#"{"error": "Unauthorized: invalid api key"}"#;
        let res = parse_jev_response(err_json, 7);
        match res {
            Err(InferenceError::ValidationFailed(msg)) => {
                assert!(msg.contains("Jev API error"));
            }
            other => panic!("Expected ValidationFailed, got {other:?}"),
        }
    }

    #[test]
    fn test_real_jev_interpreter_unreachable_endpoint() {
        let interpreter = RealJevInterpreter::with_endpoint(
            "dummy_key".to_string(),
            "http://127.0.0.1:1".to_string(),
        );
        assert!(interpreter.is_real());

        let snapshot = Snapshot {
            id: 7,
            observed_at_ms: 1000,
        };
        let res = interpreter.interpret_market(&snapshot, &[100.0, 101.0, 103.0], None);
        assert!(matches!(res, Err(InferenceError::ProviderUnavailable)));
    }

    #[test]
    fn test_parse_cli_args_jev_real() {
        let args = vec![
            "--live".to_string(),
            "--jev".to_string(),
            "real".to_string(),
            "--timeframe".to_string(),
            "5m".to_string(),
            "--pair".to_string(),
            "238".to_string(),
        ];
        let cli = parse_cli_args(&args).expect("Should parse");
        assert!(cli.is_live);
        assert!(cli.is_real_jev);
        assert_eq!(cli.timeframe, "5m");
        assert_eq!(cli.pair_id, 238);

        let args2 = vec!["--real-jev".to_string()];
        let cli2 = parse_cli_args(&args2).expect("Should parse");
        assert!(cli2.is_real_jev);
        assert!(!cli2.is_live);

        let args3 = vec!["--jev".to_string(), "invalid".to_string()];
        assert!(parse_cli_args(&args3).is_err());
    }
}




