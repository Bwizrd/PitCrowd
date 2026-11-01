fn main() {
    println!("JevFloor learning day: fake data only");
}

#[cfg(test)]
mod tests {
    #[test]
    fn price_delta_is_three() {
        let open = 100;
        let close = 103;
        assert_eq!(close - open, 3);
    }
}
