// ### Step 1: Imports & JSON Deserialization
use futures::stream::{self, StreamExt}; // RxJS stream operators (.map, .then)
use serde::Deserialize;

// JSON shape from Coinbase: {"data": {"base": "BTC", "amount": "83400.00"}}
#[derive(Deserialize, Debug)]
struct SpotResponse {
    data: SpotData,
}

#[derive(Deserialize, Debug)]
struct SpotData {
    base: String,
    amount: String,
}
//  ### Step 2: The Fetch Function (async fn)
//  This is just like async function fetchPrice(pair: string) in TypeScript:
async fn fetch_price(pair: &str) -> SpotResponse {
    let url = format!("https://api.coinbase.com/v2/prices/{}/spot", pair);

    reqwest::get(&url)
        .await
        .expect("network request failed")
        .json::<SpotResponse>()
        .await
        .expect("failed to parse JSON")
}
// ### Step 3: main with Tokio & RxJS Comparisons

// Notice the attribute #[tokio::main]. This initializes the background async event loop (like Node.js's event loop) and allows main to be async!

// #### Part A: RxJS forkJoin → tokio::join!

//In RxJS: forkJoin([fetchPrice('BTC-USD'), fetchPrice('ETH-USD')])

#[tokio::main]
async fn main() {
    println!("--- 1. Parallel Fetch (RxJS forkJoin) ---");

    // Fires both requests concurrently and waits for both:
    let (btc, eth) = tokio::join!(fetch_price("BTC-USD"), fetch_price("ETH-USD"));

    println!(
        "Combined: {} is ${}, {} is ${}",
        btc.data.base, btc.data.amount, eth.data.base, eth.data.amount
    );

    // #### Part B: RxJS Stream Pipeline (map + switchMap / then)
    // In RxJS:

    // from(["BTC-USD", "ETH-USD", "SOL-USD"]).pipe(
    //     map(pair => pair.toUpperCase()),
    //     switchMap(pair => fetchPrice(pair))
    // ).subscribe(res => console.log(res));

    println!("\n--- 2. Stream Pipeline (RxJS map + switchMap) ---");

    let pairs = vec!["BTC-USD", "ETH-USD", "SOL-USD"];

    // stream::iter is RxJS `from(pairs)`
    stream::iter(pairs)
        .map(|pair| pair.to_uppercase()) // RxJS: map(...)
        .then(|pair| async move {
            // RxJS: switchMap / async transform
            fetch_price(&pair).await
        })
        .for_each(|res| async move {
            // RxJS: .subscribe(...)
            println!("Stream emitted: {} = ${}", res.data.base, res.data.amount);
        })
        .await;
}
