enum MarketEvent {
    Tick { symbol: String, price: i64 },
}

fn consume(event: MarketEvent) {
    match event {
        MarketEvent::Tick { symbol, .. } => println!("consumed: {}", symbol),
    }
}

fn describe(event: &MarketEvent) {
    match event {
        MarketEvent::Tick { symbol, price } => println!("tick {} {}", symbol, price),
    }
}

fn main() {
    let symbol = "SILVER".to_string();
    let price = 103;
    let tick = MarketEvent::Tick { symbol, price };
    describe(&tick);
    describe(&tick);
    consume(tick);
}
