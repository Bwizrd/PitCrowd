enum MarketEvent {
    Tick { symbol: String, price: i64 },
}

fn consume(event: MarketEvent) -> MarketEvent {
    println!("Consumer saw SILVER");
    event
}

fn main() {
    let tick = MarketEvent::Tick {
        symbol: "SILVER".to_string(),
        price: 103,
    };

    let tick = consume(tick);

    //match &tick {
    //  MarketEvent::Tick { symbol, .. } => println!("caller owns {symbol} again"),
    //}
    if let MarketEvent::Tick { symbol, .. } = &tick {
        println!("caller owns {symbol} again");
    }

    consume(tick);
}
