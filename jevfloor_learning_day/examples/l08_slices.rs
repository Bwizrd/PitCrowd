use std::println;

fn sum_prices(prices: &[i64]) {
    let mut count = 0;
    let mut sum = 0;
    for price in prices {
        println!("{}", price);
        count += 1;
        sum += price;
    }
    println!("count: {}", count);
    println!("sum: {}", sum);
}

fn increment(prices: &mut [i64]) {
    for i in 0..prices.len() {
        prices[i] += 1;
    }
}

fn main() {
    let mut prices: [i64; 3] = [100, 101, 102];
    let all = &prices[..];
    let mut from = prices[1..].to_vec();
    let upto = &prices[..2];

    println!("{:?}", prices);
    println!("{:?}", all);
    println!("{:?}", from);
    println!("{:?}", upto);

    sum_prices(&prices);
    sum_prices(&from);
    sum_prices(&[]);

    increment(&mut prices);
    println!("{:?}", prices);

    increment(&mut from);
    println!("{:?}", prices);
}
