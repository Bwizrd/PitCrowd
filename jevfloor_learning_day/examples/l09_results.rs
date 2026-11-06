fn last_price(prices: &[i64]) -> Option<i64> {
    // if prices is empty → None
    if prices.is_empty() {
        None
    } else {
        Some(prices[prices.len() - 1])
    }
}

fn parse_price(text: &str) -> Result<i64, std::num::ParseIntError> {
    text.parse::<i64>()
}

fn price_plus_one(text: &str) -> Result<i64, std::num::ParseIntError> {
    let price = parse_price(text)?;
    Ok(price + 1)
}

fn price_plus_one_short(text: &str) -> Result<i64, std::num::ParseIntError> {
    let price = parse_price(text)?; // Ok → give me the number; Err → return it right now
    Ok(price + 1) // wrap the answer back up as a success
}

fn main() {
    let prices: [i64; 3] = [100, 101, 102];
    let all = &prices[..];
    println!("{:?}", last_price(&prices[..2]));
    println!("{:?}", last_price(&[]));
    println!("{:?}", last_price(all));
    match last_price(all) {
        Some(price) => println!("last={}", price), // price is the plain number
        None => println!("empty"),                 // must handle this too
    }
    match parse_price("103") {
        Ok(price) => println!("price={}", price),
        Err(_) => println!("parse failed"),
    }

    match parse_price("______") {
        // blank: some text that ISN'T a number
        Ok(price) => println!("price={}", price),
        Err(_) => println!("parse failed"),
    }

    match price_plus_one("103") {
        Ok(p) => println!("plus one={}", p),
        Err(_) => println!("______"),
    }

    match price_plus_one("oops") {
        Ok(p) => println!("plus one={}", p),
        Err(_) => println!("oops"),
    }

    match price_plus_one_short("104") {
        Ok(p) => println!("plus one={}", p),
        Err(_) => println!("______"),
    }

    match price_plus_one_short("oooooops") {
        Ok(p) => println!("plus one={}", p),
        Err(_) => println!("ooooooooops"),
    }
}

#[cfg(test)]
mod tests {
    use super::*; // new: "let the tests see the functions above"

    #[test]
    fn empty_is_none() {
        assert_eq!(last_price(&[]), None);
    }

    #[test]
    fn good_text_parses() {
        assert_eq!(parse_price("103"), Ok(103)); // blank: the number you expect
    }

    #[test]
    fn bad_text_fails() {
        assert!(parse_price("oops").is_err()); // is_err() = "is this a courier's note?"
    }
}
