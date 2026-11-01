#[derive(Debug, PartialEq)]
enum VisualBehaviour {
    Hesitate,
    Walk,
    Run,
}

fn decide_behaviour(
    delta: i64,
    p_persistent: f64,
    p_active: f64,
    confidence: f64,
) -> VisualBehaviour {
    if confidence < 0.60 {
        VisualBehaviour::Hesitate
    } else if delta > 0 && p_persistent >= 0.80 && p_active >= 0.80 {
        VisualBehaviour::Run
    } else if delta > 0 {
        VisualBehaviour::Walk
    } else {
        VisualBehaviour::Hesitate
    }
}

fn main() {
    let prices = vec![100, 101, 103];

    let delta = prices[prices.len() - 1] - prices[0];

    let mut positive_changes = 0;

    for i in 1..prices.len() {
        if prices[i] > prices[i - 1] {
            positive_changes += 1;
        }
    }

    println!("delta={delta} positive_changes={positive_changes}");

    let strong = decide_behaviour(3, 0.85, 0.90, 0.90);
    let mixed = decide_behaviour(3, 0.85, 0.50, 0.90);
    let uncertain = decide_behaviour(3, 0.85, 0.90, 0.40);
    let nonpositive = decide_behaviour(-1, 0.85, 0.90, 0.90);

    println!(
        "strong={:?} mixed={:?} uncertain={:?} nonpositive={:?}",
        strong, mixed, uncertain, nonpositive
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strong_should_run() {
        assert_eq!(decide_behaviour(3, 0.85, 0.90, 0.90), VisualBehaviour::Run);
    }

    #[test]
    fn mixed_should_walk() {
        assert_eq!(decide_behaviour(3, 0.85, 0.50, 0.90), VisualBehaviour::Walk);
    }

    #[test]
    fn uncertain_should_hesitate() {
        assert_eq!(
            decide_behaviour(3, 0.85, 0.90, 0.40),
            VisualBehaviour::Hesitate
        );
    }

    #[test]
    fn nonpositive_should_hesitate() {
        assert_eq!(
            decide_behaviour(-1, 0.85, 0.90, 0.90),
            VisualBehaviour::Hesitate
        );
    }
}
