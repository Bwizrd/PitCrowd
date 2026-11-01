// #### Step 1: Types & Setup

use std::sync::mpsc;
use std::thread;

#[derive(Debug)]
enum MarketEvent {
    Tick { symbol: String, price: i64 },
}

// #### Step 2: Channel & Producer Thread

// Inside main():

fn main() {
    // 1. Create a bounded channel with capacity 2:
    let (tx, rx) = mpsc::sync_channel::<MarketEvent>(2);

    // 2. Spawn a producer thread that sends 3 ticks:
    let producer = thread::spawn(move || {
        let prices = [100, 101, 103];
        for price in prices {
            let event = MarketEvent::Tick {
                symbol: "SILVER".to_string(),
                price,
            };
            tx.send(event).unwrap();
        }
        // When this thread exits, `tx` is dropped!
    });

    // #### Step 3: Consumer in main

    // Drain the channel in main. Because rx implements IntoIterator, you can use a normal for loop:

    // 3. Receive events until all senders drop:
    let mut received = 0;
    for event in rx {
        if let MarketEvent::Tick { symbol, price } = event {
            println!("tick {} {}", symbol, price);
            received += 1;
        }
    }

    // 4. Join the producer thread after the channel closes:
    producer.join().unwrap();
    println!("stream closed; received={}", received);
}
