#[derive(Debug)]
struct Candle {
    open: i64,
    close: i64,
}

fn latest(prices: &[i64]) -> Option<&i64> {
    if prices.is_empty() {
        None
    } else {
        Some(&prices[prices.len() - 1])
    }
}

// It refuses to compile because it has no idea whether the return value borrows from a or from b.
// fn higher_close(a: &Candle, b: &Candle) -> &Candle {
//     if a.close >= b.close { a } else { b }
// }

// with lifetime annotations the compiler knows to let a and b live for the same duration
fn higher_close<'a>(a: &'a Candle, b: &'a Candle) -> &'a Candle {
    if a.close >= b.close { a } else { b }
}

// fn invalid_local() -> &Candle {
//     let c = Candle {
//         open: 100,
//         close: 105,
//     };
//     &c
// }

fn main() {
    let prices = vec![100, 101, 103];
    if let Some(p) = latest(&prices) {
        println!("latest={}", p);
    }

    // let c1 = Candle {
    //     open: 100,
    //     close: 105,
    // };
    // let c2 = Candle {
    //     open: 100,
    //     close: 102,
    // };

    // let higher = higher_close(&c1, &c2);

    let higher;
    let c1 = Candle {
        open: 100,
        close: 105,
    }; // lives for the whole main

    // Remove the code block to fix the lifetime error
    // {
    let c2 = Candle {
        open: 100,
        close: 102,
    }; // lives ONLY inside this { } block
    higher = higher_close(&c1, &c2);
    // } // <--- c2 is destroyed here!

    println!("higher={}", higher.close);
}
