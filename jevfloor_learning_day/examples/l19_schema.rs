// ## Lesson 19 — Predefined Schemas and Typed Probabilistic Outputs
//
//  ### 1. Concept: Schema Validation at the AI Boundary
//
//  When consuming AI model outputs (like Jev), just because a payload parses into an f64 float does not mean it is a valid probability!
//
//  • In math, a probability p must be strictly within [0.0, 1.0].
//  • An f64 can easily be 1.20, -0.50, or f64::NAN (Not a Number).
//  • An envelope also carries a schema_version. If a remote provider upgrades to version: 2, our client must safely reject it rather than misinterpret fields.
//  ──────
// ### 2. Task: Create examples/l19_schema.rs

// Create l19_schema.rs:

// #### Step 1: Types & Validator

#[derive(Debug, PartialEq)]
enum ValidationError {
    InvalidProbability,
    NotANumber,
    UnsupportedSchemaVersion,
}

#[derive(Debug)]
struct Envelope {
    schema_version: u32,
    p_persistent: f64,
    p_active: f64,
    confidence: f64,
}

fn validate(env: &Envelope) -> Result<(), ValidationError> {
    // 1. Enforce supported schema version:
    if env.schema_version != 1 {
        return Err(ValidationError::UnsupportedSchemaVersion);
    }

    // 2. Validate all probability/confidence fields:
    let values = [env.p_persistent, env.p_active, env.confidence];
    for &val in &values {
        if val.is_nan() {
            return Err(ValidationError::NotANumber);
        }
        if !(0.0..=1.0).contains(&val) {
            return Err(ValidationError::InvalidProbability);
        }
    }

    Ok(())
}

//  #### Step 2: main (4 Test Fixtures)

fn main() {
    let valid = Envelope {
        schema_version: 1,
        p_persistent: 0.85,
        p_active: 0.90,
        confidence: 0.90,
    };
    let out_of_range = Envelope {
        schema_version: 1,
        p_persistent: 1.20,
        p_active: 0.90,
        confidence: 0.90,
    };
    let nan_val = Envelope {
        schema_version: 1,
        p_persistent: f64::NAN,
        p_active: 0.90,
        confidence: 0.90,
    };
    let wrong_version = Envelope {
        schema_version: 99,
        p_persistent: 0.85,
        p_active: 0.90,
        confidence: 0.90,
    };

    match validate(&valid) {
        Ok(()) => println!("valid accepted"),
        Err(_) => println!("valid rejected"),
    }

    match validate(&out_of_range) {
        Ok(()) => println!("out-of-range accepted"),
        Err(ValidationError::InvalidProbability) => println!("out-of-range rejected"),
        Err(_) => {}
    }

    match validate(&nan_val) {
        Ok(()) => println!("NaN accepted"),
        Err(ValidationError::NotANumber) => println!("NaN rejected"),
        Err(_) => {}
    }

    match validate(&wrong_version) {
        Ok(()) => println!("schema accepted"),
        Err(ValidationError::UnsupportedSchemaVersion) => println!("schema rejected"),
        Err(_) => {}
    }
}

// #### Step 3: Tests

//Add unit tests at the bottom to verify all 4 cases:

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_envelope() {
        let env = Envelope {
            schema_version: 1,
            p_persistent: 0.85,
            p_active: 0.90,
            confidence: 0.90,
        };
        assert_eq!(validate(&env), Ok(()));
    }

    #[test]
    fn test_out_of_range() {
        let env = Envelope {
            schema_version: 1,
            p_persistent: 1.20,
            p_active: 0.90,
            confidence: 0.90,
        };
        assert_eq!(validate(&env), Err(ValidationError::InvalidProbability));
    }

    #[test]
    fn test_nan() {
        let env = Envelope {
            schema_version: 1,
            p_persistent: f64::NAN,
            p_active: 0.90,
            confidence: 0.90,
        };
        assert_eq!(validate(&env), Err(ValidationError::NotANumber));
    }

    #[test]
    fn test_wrong_version() {
        let env = Envelope {
            schema_version: 99,
            p_persistent: 0.85,
            p_active: 0.90,
            confidence: 0.90,
        };
        assert_eq!(
            validate(&env),
            Err(ValidationError::UnsupportedSchemaVersion)
        );
    }
}
