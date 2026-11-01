struct Candle {
    open: i64,
    high: i64,
    low: i64,
    close: i64,
    volume: u64,
}

enum MarketEvent {
    Tick { symbol: String, price: i64 },
    Closed(Candle),
    Disconnected,
}

fn describe(event: MarketEvent) -> String {
    match event {
        MarketEvent::Tick { symbol, price } => {
            format!("tick {symbol} {price}")
        }
        MarketEvent::Closed(candle) => {
            format!("candle close={}", candle.close)
        }
        MarketEvent::Disconnected => "disconnected".to_string(),
    }
}

fn main() {
    let candle = Candle {
        open: 100,
        high: 105,
        low: 98,
        close: 103,
        volume: 1000,
    };

    let tick = MarketEvent::Tick {
        symbol: "SILVER".to_string(),
        price: 103,
    };

    let closed = MarketEvent::Closed(candle);

    let events = vec![tick, closed, MarketEvent::Disconnected];

    for event in events {
        println!("{}", describe(event));
    }
}
