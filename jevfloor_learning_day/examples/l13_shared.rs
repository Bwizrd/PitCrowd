// Normally, Rust’s ownership rule says: one owner, no sharing. But in concurrent and async code, multiple threads or tasks need to reference the same data.
//
//  1. Box<T> (Unique Heap Storage)
//  Moves data off the stack onto the heap. Still has one owner, but lives at a fixed heap address.
//  2. Arc<T> (Atomic Reference Counting)
//  Allows multiple owners across threads. When you clone an Arc, it does not duplicate the data; it just increments an atomic counter. When the last Arc drops, the data is freed.
//  3. Mutex<T> (Mutual Exclusion)
//  Arc gives shared ownership, but shared access is read-only! To mutate data inside an Arc, you wrap it in a Mutex:
//    Arc<Mutex<T>>
//  To touch the data, a thread must call .lock().unwrap(). Only one thread can hold the lock at a time. When the lock guard goes out of scope, it automatically unlocks.

// Bring in standard library threading and sync tools at the top of the file:

use std::sync::{Arc, Mutex};
use std::thread;

struct Candle {
    open: i64,
    close: i64,
}

fn main() {
    // 1. Box a candle on the heap
    let boxed_candle = Box::new(Candle {
        open: 100,
        close: 103,
    });
    println!("boxed close={}", boxed_candle.close);

    //   //   Rust rules say a value can only have one owner. If you send a variable to Thread 1, Thread 2 can't have it.
    //   2. Problem 2: Safety when mutating
    //   If multiple threads try to write to the same number at the same time, you get a data race / corruption.
    //
    //   Rust solves these two problems with two distinct wrappers, layered like an onion:
    //
    //            Arc (Solves Problem 1: Shared Ownership)
    //             └── Mutex (Solves Problem 2: Safe Mutation)
    //                  └── 0u64 (The actual data)
    //   #### Why couldn't we use just Mutex?
    //
    // A Mutex<u64> protects against data races, but a Mutex can still only have one owner. If you move it into Thread 1, Thread 2 can't access it.
    //
    // #### Why couldn't we use just Arc?
    //
    // An Arc<u64> lets multiple threads own handles to the same number, but Arc only gives read-only access (&T)! Arc forbids mutation so that threads don't corrupt each other's data.
    //
    // #### The Combination: Arc<Mutex<T>>
    //
    // • The Mutex wraps the data so only one thread can modify it at a time.
    // • The Arc wraps the Mutex so multiple threads can hold a pointer to that same lock on the heap.
    // ──────
    // ### Everyday Analogy: The Shared Safe
    //
    // • 0u64: The money.
    // • Mutex: The steel safe with a lock. Only one person can open the door at a time.
    // • Arc: The access badges handed out to each thread, allowing them to walk into the room where the safe is located.
    //
    // You put the money in the safe (Mutex<0u64>), and then give multiple workers access to that safe (Arc<Mutex<0u64>>).

    //  1. Create a counter starting at 0:

    let counter = Arc::new(Mutex::new(0u64));

    //  2. Clone the Arc handle for each thread:
    let counter_t1 = Arc::clone(&counter);
    let t1 = thread::spawn(move || {
        let mut guard = counter_t1.lock().unwrap();
        *guard += 1;
    });

    let counter_t2 = Arc::clone(&counter);
    let t2 = thread::spawn(move || {
        let mut guard = counter_t2.lock().unwrap();
        *guard += 1;
    });

    //  3. Wait for both threads to finish (join), then inspect the final counter:
    t1.join().unwrap();
    t2.join().unwrap();

    println!("processed={}", *counter.lock().unwrap());
}
