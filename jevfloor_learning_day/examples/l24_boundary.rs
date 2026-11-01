// 1. Data Types

#[derive(Debug)]
struct Snapshot {
    id: u64,
    observed_at_ms: u64,
}

#[derive(Debug)]
struct Envelope {
    p_persistent: f64,
    p_active: f64,
    confidence: f64,
}

#[derive(Debug)]
enum InferenceError {
    StaleData,
    ProviderError,
    FutureTimestamp,
}

#[derive(Debug, PartialEq)]
enum VisualBehaviour {
    Hesitate,
    Run,
}

// 2. Interpreter Trait

trait Interpreter {
    fn interpret(&self, snapshot: &Snapshot) -> Result<Envelope, InferenceError>;
}

// 3. Mock Providers

struct SuccessProvider;

impl Interpreter for SuccessProvider {
    fn interpret(&self, _snapshot: &Snapshot) -> Result<Envelope, InferenceError> {
        Ok(Envelope {
            p_persistent: 0.85,
            p_active: 0.90,
            confidence: 0.90,
        })
    }
}

struct ErrorProvider;

impl Interpreter for ErrorProvider {
    fn interpret(&self, _snapshot: &Snapshot) -> Result<Envelope, InferenceError> {
        Err(InferenceError::ProviderError)
    }
}

fn decide_behaviour(
    provider: &impl Interpreter,
    snapshot: &Snapshot,
    now_ms: u64,
    max_age_ms: u64,
) -> VisualBehaviour {
    // Reject timestamps from the future.
    if snapshot.observed_at_ms > now_ms {
        return VisualBehaviour::Hesitate;
    }

    // Reject stale data.
    let age = now_ms - snapshot.observed_at_ms;

    if age > max_age_ms {
        return VisualBehaviour::Hesitate;
    }

    // Data is fresh, so run inference.
    match provider.interpret(snapshot) {
        Ok(envelope) => {
            if envelope.confidence >= 0.60 {
                VisualBehaviour::Run
            } else {
                VisualBehaviour::Hesitate
            }
        }
        Err(_) => VisualBehaviour::Hesitate,
    }
}

fn main() {
    let now_ms = 1000;
    let max_age_ms = 200;

    let success_provider = SuccessProvider;
    let error_provider = ErrorProvider;

    // Fresh: age = 100ms
    let fresh = Snapshot {
        id: 1,
        observed_at_ms: 900,
    };

    let fresh_result = decide_behaviour(&success_provider, &fresh, now_ms, max_age_ms);

    println!("fresh={fresh_result:?}");

    // Stale: age = 300ms
    let stale = Snapshot {
        id: 2,
        observed_at_ms: 700,
    };

    let stale_result = decide_behaviour(&success_provider, &stale, now_ms, max_age_ms);

    println!("stale={stale_result:?}");

    // Provider error: fresh data, but inference fails
    let provider_error = Snapshot {
        id: 3,
        observed_at_ms: 900,
    };

    let error_result = decide_behaviour(&error_provider, &provider_error, now_ms, max_age_ms);

    println!("provider-error={error_result:?}");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn age_exactly_200ms_is_accepted() {
        let provider = SuccessProvider;

        let snapshot = Snapshot {
            id: 1,
            observed_at_ms: 800,
        };

        let result = decide_behaviour(&provider, &snapshot, 1000, 200);

        assert_eq!(result, VisualBehaviour::Run);
    }

    #[test]
    fn age_201ms_is_rejected() {
        let provider = SuccessProvider;

        let snapshot = Snapshot {
            id: 2,
            observed_at_ms: 799,
        };

        let result = decide_behaviour(&provider, &snapshot, 1000, 200);

        assert_eq!(result, VisualBehaviour::Hesitate);
    }

    #[test]
    fn provider_error_fails_safely() {
        let provider = ErrorProvider;

        let snapshot = Snapshot {
            id: 3,
            observed_at_ms: 900,
        };

        let result = decide_behaviour(&provider, &snapshot, 1000, 200);

        assert_eq!(result, VisualBehaviour::Hesitate);
    }
}
