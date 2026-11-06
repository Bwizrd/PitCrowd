use std::sync::atomic::{AtomicUsize, Ordering};

#[allow(dead_code)]
#[derive(Debug)]
struct Snapshot {
    id: u64,
    observed_at_ms: u64,
}

#[allow(dead_code)]
#[derive(Debug)]
struct Features {
    delta: i64,
    positive_changes: usize,
}

#[allow(dead_code)]
#[derive(Debug)]
struct InterpretationPayload {
    p_persistent: f64,
    p_active: f64,
    confidence: f64,
}

#[allow(dead_code)]
#[derive(Debug)]
struct ReplayRecord {
    snapshot: Snapshot,
    features: Features,
    interpretation: Option<InterpretationPayload>,
    source: String,
    now_ms: u64,
    schema_version: u32,
    policy_version: u32,
}

#[derive(Debug, PartialEq, Clone, Copy)]
enum VisualBehaviour {
    Run,
    Walk,
    Hesitate,
}

#[derive(Debug, PartialEq)]
enum ReplayError {
    UnsupportedPolicyVersion(u32),
    UnsupportedSchemaVersion(u32),
}

static PROVIDER_CALLS: AtomicUsize = AtomicUsize::new(0);

fn evaluate_replay(
    record: &ReplayRecord,
    current_policy_version: u32,
    max_age_ms: u64,
) -> Result<VisualBehaviour, ReplayError> {
    if record.schema_version != 1 {
        return Err(ReplayError::UnsupportedSchemaVersion(record.schema_version));
    }

    if record.policy_version != current_policy_version {
        return Err(ReplayError::UnsupportedPolicyVersion(record.policy_version));
    }
    if record.snapshot.observed_at_ms > record.now_ms
        || record.now_ms - record.snapshot.observed_at_ms > max_age_ms
    {
        return Ok(VisualBehaviour::Hesitate);
    }

    if let Some(ref interp) = record.interpretation
        && record.features.delta > 0
        && interp.confidence >= 0.60
    {
        if interp.p_persistent >= 0.80 && interp.p_active >= 0.80 {
            return Ok(VisualBehaviour::Run);
        } else {
            return Ok(VisualBehaviour::Walk);
        }
    }

    Ok(VisualBehaviour::Hesitate)
}

fn main() {
    let current_policy_version = 1;
    let max_age_ms = 200;

    let valid_record = ReplayRecord {
        snapshot: Snapshot {
            id: 1,
            observed_at_ms: 900,
        },
        features: Features {
            delta: 3,
            positive_changes: 3,
        },
        interpretation: Some(InterpretationPayload {
            p_persistent: 0.85,
            p_active: 0.90,
            confidence: 0.90,
        }),
        source: "recorded".to_string(),
        now_ms: 1000,
        schema_version: 1,
        policy_version: 1,
    };

    let replay1 = evaluate_replay(&valid_record, current_policy_version, max_age_ms).unwrap();

    let replay2 = evaluate_replay(&valid_record, current_policy_version, max_age_ms).unwrap();

    let provider_calls = PROVIDER_CALLS.load(Ordering::SeqCst);

    println!("replay1={replay1:?} replay2={replay2:?} provider_calls={provider_calls}");

    let stale_record = ReplayRecord {
        snapshot: Snapshot {
            id: 2,
            observed_at_ms: 700,
        },
        features: Features {
            delta: 3,
            positive_changes: 3,
        },
        interpretation: Some(InterpretationPayload {
            p_persistent: 0.85,
            p_active: 0.90,
            confidence: 0.90,
        }),
        source: "recorded".to_string(),
        now_ms: 1000,
        schema_version: 1,
        policy_version: 1,
    };

    let stale = evaluate_replay(&stale_record, current_policy_version, max_age_ms).unwrap();

    println!("stale replay={stale:?}");

    let unsupported_record = ReplayRecord {
        snapshot: Snapshot {
            id: 3,
            observed_at_ms: 900,
        },
        features: Features {
            delta: 3,
            positive_changes: 3,
        },
        interpretation: Some(InterpretationPayload {
            p_persistent: 0.85,
            p_active: 0.90,
            confidence: 0.90,
        }),
        source: "recorded".to_string(),
        now_ms: 1000,
        schema_version: 1,
        policy_version: 2,
    };

    match evaluate_replay(&unsupported_record, current_policy_version, max_age_ms) {
        Err(ReplayError::UnsupportedPolicyVersion(2)) => {
            println!("unsupported policy rejected");
        }
        _ => {
            println!("unexpected result");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replay_is_deterministic_and_does_not_call_provider() {
        PROVIDER_CALLS.store(0, Ordering::SeqCst);

        let record = ReplayRecord {
            snapshot: Snapshot {
                id: 1,
                observed_at_ms: 900,
            },
            features: Features {
                delta: 3,
                positive_changes: 3,
            },
            interpretation: Some(InterpretationPayload {
                p_persistent: 0.85,
                p_active: 0.90,
                confidence: 0.90,
            }),
            source: "recorded".to_string(),
            now_ms: 1000,
            schema_version: 1,
            policy_version: 1,
        };

        let replay1 = evaluate_replay(&record, 1, 200).unwrap();
        let replay2 = evaluate_replay(&record, 1, 200).unwrap();

        assert_eq!(replay1, replay2);
        assert_eq!(replay1, VisualBehaviour::Run);
        assert_eq!(PROVIDER_CALLS.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn stale_record_returns_hesitate() {
        let record = ReplayRecord {
            snapshot: Snapshot {
                id: 2,
                observed_at_ms: 700,
            },
            features: Features {
                delta: 3,
                positive_changes: 3,
            },
            interpretation: Some(InterpretationPayload {
                p_persistent: 0.85,
                p_active: 0.90,
                confidence: 0.90,
            }),
            source: "recorded".to_string(),
            now_ms: 1000,
            schema_version: 1,
            policy_version: 1,
        };

        let result = evaluate_replay(&record, 1, 200).unwrap();

        assert_eq!(result, VisualBehaviour::Hesitate);
    }

    #[test]
    fn unsupported_policy_version_returns_error() {
        let record = ReplayRecord {
            snapshot: Snapshot {
                id: 3,
                observed_at_ms: 900,
            },
            features: Features {
                delta: 3,
                positive_changes: 3,
            },
            interpretation: Some(InterpretationPayload {
                p_persistent: 0.85,
                p_active: 0.90,
                confidence: 0.90,
            }),
            source: "recorded".to_string(),
            now_ms: 1000,
            schema_version: 1,
            policy_version: 2,
        };

        let result = evaluate_replay(&record, 1, 200);

        assert_eq!(result, Err(ReplayError::UnsupportedPolicyVersion(2)));
    }
}
