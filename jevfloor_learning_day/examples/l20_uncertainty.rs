// ### Task: Create examples/l20_uncertainty.rs

// Create l20_uncertainty.rs:
// #### Step 1: Policy Enum & Classifier
#[derive(Debug, PartialEq)]
enum MarketStance {
    Strong,
    Mixed,
    Uncertain,
}

// Display / decision policy:
// 1. If confidence < 0.60 -> Uncertain (unreliable estimate)
// 2. Else if probability >= 0.80 -> Strong
// 3. Otherwise -> Mixed
fn classify(p: f64, confidence: f64) -> MarketStance {
    if confidence < 0.60 {
        MarketStance::Uncertain
    } else if p >= 0.80 {
        MarketStance::Strong
    } else {
        MarketStance::Mixed
    }
}

// #### Step 2: main (3 Market Scenarios)

fn main() {
    let s1 = classify(0.85, 0.90);
    let s2 = classify(0.85, 0.40);
    let s3 = classify(0.50, 0.90);

    println!("{:?}", s1); // Strong
    println!("{:?}", s2); // Uncertain
    println!("{:?}", s3); // Mixed
}

// #### Step 3: Tests for Exact Boundaries (0.60 and 0.80)

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_exact_confidence_boundary() {
        // At exactly 0.60 confidence with p=0.85, it is Strong (not Uncertain)
        assert_eq!(classify(0.85, 0.60), MarketStance::Strong);
    }

    #[test]
    fn test_exact_probability_boundary() {
        // At exactly 0.80 probability with confidence=0.90, it is Strong
        assert_eq!(classify(0.80, 0.90), MarketStance::Strong);
        // Just below 0.80 with high confidence is Mixed
        assert_eq!(classify(0.799, 0.90), MarketStance::Mixed);
    }
}
