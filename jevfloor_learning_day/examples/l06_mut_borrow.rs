use std::println;

struct Candle {
    open: i64,
    high: i64,
    low: i64,
    close: i64,
    volume: u64,
}

// this function needs to borrow the value and change it, this will automatically change main's
// candle value as it is a borrow and it pointing to the same data in memory
fn adjust_close(candle: &mut Candle, delta: i64) {
    candle.close += delta
}

// this function will receive the moved value, change it then move it back
fn move_adjust_close(mut candle: Candle, delta: i64) -> Candle {
    candle.close += delta;
    candle
}

fn main() {
    let mut candle = Candle {
        open: 100,
        high: 105,
        low: 98,
        close: 100,
        volume: 1000,
    };

    let delta = 3;

    println!("Before Borrow {}", candle.close);
    adjust_close(&mut candle, delta);
    println!("After Borrow {}", candle.close);
    println!("Before Move {}", candle.close);
    let candle = move_adjust_close(candle, delta);
    println!("Before Move {}", candle.close);

    let mut c2 = Candle {
        open: 100,
        high: 105,
        low: 98,
        close: 100,
        volume: 1000,
    };
    let view = &c2; // start peeking
    println!("{}", view.close); // the peek is used here
    adjust_close(&mut c2, 3);

    println!("{}", c2.close); // the peek is used here
}
