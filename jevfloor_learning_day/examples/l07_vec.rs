fn main() {
    let mut prices = vec![100];
    prices.push(101);
    println!("Prices 1: {}", prices[0]);
    println!("Prices 2: {}", prices[1]);
    prices.push(102);
    let first = prices[0];
    println!("First {}", first);
    println!("len={} capacity={}", prices.len(), prices.capacity());
    prices.push(103);
    println!("len={} capacity={}", prices.len(), prices.capacity());
    prices.push(104);
    println!("len={} capacity={}", prices.len(), prices.capacity());
    prices.push(105);
    println!("len={} capacity={}", prices.len(), prices.capacity());
}
