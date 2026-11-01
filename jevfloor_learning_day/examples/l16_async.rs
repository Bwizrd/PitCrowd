// #### Step 1: Types & Imports

use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::{sleep, timeout};

#[derive(Debug)]
enum MarketEvent {
    Tick { symbol: String, price: i64 },
}

// #### Step 2: Async main & Producer Task

#[tokio::main]
async fn main() {
    // 1. Create a bounded async channel with capacity 2:
    let (tx, mut rx) = mpsc::channel::<MarketEvent>(2);

    // 2. Spawn an async producer task:
    let producer_task = tokio::spawn(async move {
        let prices = [100, 101, 103];
        for price in prices {
            // Asynchronously pause for 10ms without blocking any OS thread:
            sleep(Duration::from_millis(1000)).await;

            let event = MarketEvent::Tick {
                symbol: "SILVER".to_string(),
                price,
            };
            tx.send(event).await.unwrap();
        }
        // When task ends, `tx` is dropped!
    });

    // #### Step 3: Consumer with a Timeout

    // Guard the receiver with a timeout so network or channel bugs can never hang the process indefinitely:

    // 3. Receive all ticks wrapped in a 500ms timeout:
    let receive_result = timeout(Duration::from_millis(3500), async {
        while let Some(event) = rx.recv().await {
            let MarketEvent::Tick { price, .. } = event;
            println!("async tick={}", price);
        }
    })
    .await;

    // Verify it didn't time out:
    assert!(receive_result.is_ok(), "Consumer timed out!");

    // 4. Await the producer task handle:
    producer_task.await.unwrap();
    println!("async complete");
}
