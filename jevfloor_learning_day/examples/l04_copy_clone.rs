use std::println;

#[derive(Clone)]
enum MarketEvent {
    Tick { symbol: String, price: i64 },
}

fn main() {
    let price = 100;
    let mut copy = price;
    println!("{}", price);
    println!("{}", copy);
    copy += 1;
    println!("Original: {}, Copy: {}", price, copy);

    let symbol = "SILVER".to_string();
    let mut clone = symbol.clone();
    clone.push_str("-SIM");

    println!("Original: {}, Clone: {}", symbol, clone);
}
