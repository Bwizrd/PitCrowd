// #### Step 1: Types & Functions

#[derive(Debug)]
struct Interpretation {
    p_persistent: f64,
    p_active: f64,
    confidence: f64,
}

// 1. Pure deterministic Rust arithmetic (exact, instant):
fn price_delta(open: i64, close: i64) -> i64 {
    close - open
}

// 2. Stand-in for System One fast inference (returns typed probabilities):
fn mock_system_one(scenario: &str) -> Interpretation {
    match scenario {
        "bullish" => Interpretation {
            p_persistent: 0.85,
            p_active: 0.90,
            confidence: 0.90,
        },
        _ => Interpretation {
            p_persistent: 0.35,
            p_active: 0.40,
            confidence: 0.70,
        },
    }
}

// #### Step 2: main

fn main() {
    let delta = price_delta(100, 103);
    let interp = mock_system_one("bullish");

    println!(
        "delta={} source=MOCK p_persistent={:.2}",
        delta, interp.p_persistent
    );
}
