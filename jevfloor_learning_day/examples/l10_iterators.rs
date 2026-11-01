fn main() {
    let mut prices = vec![100, 101, 103];

    let sum: i64 = prices.iter().sum(); // blank: the walk that just LOOKS
    println!("sum={}", sum);

    // exclusive access to change Vec in place
    for p in prices.iter_mut() {
        *p += 1; // p is a pointer not the value so it needs to be dereferenced using the *
    }

    println!("updated={:?}", prices);

    // Or you could write
    // for p in &mut prices {
    //  *p += 1;
    //}

    // Consume prices to filter for items greater than 101 and collect into a new vector:
    //    let selected: Vec<i64> = prices.into_iter().filter(|p| *p > 101).collect();
    // println!("selected={:?}", selected); // this should error as into_iter() dismantles the vector and hands out the items. After line 20, prices no longer exists—it has been consumed.

    //### Option 1: Don't destroy prices — borrow with .iter() (Most Common)

    //If you only need to read prices and build a new vector, you don't need into_iter() at all. Use iter():

    let selected: Vec<i64> = prices
        .iter()
        .copied() // converts &i64 into i64 by copying the number
        .filter(|p| *p > 101)
        .collect();

    println!("selected={:?}", selected);
    println!("prices is still alive={:?}", prices); // works perfectly!

    // • prices.iter() only borrows prices.
    // • .copied() (or .cloned() for non-numbers) turns each &i64 into an owned i64.
    // • prices is completely untouched and still fully usable.
    // ──────
    // ### Option 2: Clone the vector before into_iter()

    // If you explicitly have a pipeline that requires consuming an owned Vec, you can clone the entire vector first:

    let selected: Vec<i64> = prices
        .clone() // makes a full copy of the vector
        .into_iter() // consumes the clone, NOT the original
        .filter(|p| *p > 101)
        .collect();

    println!("prices is still alive={:?}", prices); // works!

    println!("selected={:?}", selected);
    // Notice the trade-off: prices.clone() duplicates the entire heap allocation.
    // ──────
    //### When would you actually want into_iter()?

    // You use into_iter() when you are finished with the original collection and want to transform it without paying to clone anything.

    // For example, if you had a Vec<String> and you wanted to filter it:

    // • Using iter().cloned() would duplicate every single string in memory.
    // • Using into_iter() moves the strings directly from the old vector into the new one for free (zero copies/allocations), dropping whatever you filtered out.
}
