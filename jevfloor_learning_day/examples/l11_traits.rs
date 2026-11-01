struct Candle {
    open: i64,
    close: i64,
}

// empty struct
struct PositiveClose;
// Implement the Strategy function with the PositiveCLose flavour
impl Strategy for PositiveClose {
    fn decide(&self, candle: &Candle) -> Decision {
        if candle.close > candle.open {
            Decision::Watch
        } else {
            Decision::Wait
        }
    }
}

//Empty Struct
struct AlwaysWait;
//Implement the Strategy Function with the Always Wait flavor
impl Strategy for AlwaysWait {
    fn decide(&self, _candle: &Candle) -> Decision {
        Decision::Wait
    }
}

// enum for potential return values
#[derive(Debug, PartialEq)]
enum Decision {
    Watch,
    Wait,
}

// A capability that can be part of an implemented struct
trait Strategy {
    fn decide(&self, candle: &Candle) -> Decision;
}

// function that takes the Strategy capability as a generic input and makes a decision
fn evaluate<S: Strategy>(strategy: &S, candle: &Candle) -> Decision {
    strategy.decide(candle)
}

fn main() {
    let candle = Candle {
        open: 100,
        close: 103,
    };

    println!("PositiveClose={:?}", evaluate(&PositiveClose, &candle));
    println!("AlwaysWait={:?}", evaluate(&AlwaysWait, &candle));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn positive_close_watches_when_close_greater_than_open() {
        let candle = Candle {
            open: 100,
            close: 103,
        };
        assert_eq!(evaluate(&PositiveClose, &candle), Decision::Watch);
    }

    #[test]
    fn always_wait_waits() {
        let candle = Candle {
            open: 100,
            close: 103,
        };
        assert_eq!(evaluate(&AlwaysWait, &candle), Decision::Wait);
    }
}
