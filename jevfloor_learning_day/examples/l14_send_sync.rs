// #### Step 1: Types & Compile-Time Assertions

use std::sync::{Arc, Mutex};
use std::thread;

enum MarketEvent {
    Tick { symbol: String, price: i64 },
}

// If T does not implement the trait, calling these functions fails at COMPILE time:
fn assert_send<T: Send>() {}
fn assert_sync<T: Sync>() {}

// #### Step 2: The Intentional Error (Moving Rc across threads)

// Inside main():

// 1. Verify that MarketEvent and Arc<Mutex<u64>> have the badges:
fn main() {
    assert_send::<MarketEvent>();
    assert_send::<Arc<Mutex<u64>>>();
    assert_sync::<Arc<Mutex<u64>>>();

    // 2. Deliberate failure: Try moving an Rc into a thread:
    //    let bad_counter = Rc::new(42);
    // let bad_clone = Rc::clone(&bad_counter);
    // thread::spawn(move || {
    //     println!("bad={}", bad_clone);
    // });

    // 3. Run cargo check --example l14_send_sync.
    //     • Read the compiler error! Notice how it says:
    //     Rc<i32> cannot be sent between threads safely because the trait Send is not implemented for Rc<i32>.
    // 4. Comment out that bad_counter block.

    // #### Step 3: Moving an owned MarketEvent

    // Send an owned MarketEvent::Tick across the thread boundary:

    let event = MarketEvent::Tick {
        symbol: "SILVER".to_string(),
        price: 103,
    };

    let handle = thread::spawn(move || {
        if let MarketEvent::Tick { symbol, .. } = event {
            println!("worker saw {}", symbol);
        }
    });

    handle.join().unwrap();
}
