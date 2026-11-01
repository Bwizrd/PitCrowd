# OpenCry --- Rust Learning Guide

**Purpose:** Learn enough Rust properly to understand, design, review,
debug, and extend OpenCry yourself.\
**Audience:** Experienced software engineer learning Rust, not a
beginner programmer.\
**Approach:** Learn Rust concepts by building small pieces of the
OpenCry domain before using AI to build the larger system.

------------------------------------------------------------------------

# 1. The Goal

The goal is **not** to memorise Rust syntax.

The goal is to reach the point where you can look at Rust code and
understand:

-   who owns a value;
-   who is borrowing it;
-   why a borrow is mutable or immutable;
-   why the compiler rejects a particular ownership pattern;
-   when data is copied, cloned, moved, or referenced;
-   how errors propagate;
-   how enums and pattern matching model application state;
-   how traits define behaviour;
-   how async tasks communicate;
-   why `Arc`, `Mutex`, `RwLock`, and channels exist;
-   what `Send` and `Sync` mean;
-   how Bevy ECS changes the ownership model;
-   how to structure OpenCry without fighting the borrow checker.

If you understand those things, AI becomes an accelerator rather than
the only entity that understands your codebase.

OpenCry is an excellent project for learning Rust because Rust's core
ideas genuinely matter to the problem.

------------------------------------------------------------------------

# 2. What You Already Know

As an experienced software engineer, you do **not** need a programming
course disguised as a Rust course.

You can move quickly through:

-   variables;
-   loops;
-   conditionals;
-   functions;
-   modules;
-   basic collections;
-   basic testing.

Spend your effort on what is genuinely different in Rust:

1.  Ownership
2.  Borrowing
3.  Lifetimes
4.  Enums and pattern matching
5.  `Option` and `Result`
6.  Traits
7.  Generics
8.  Iterators
9.  Smart pointers
10. Concurrency
11. Async Rust
12. ECS / Bevy

The first four are the foundation.

Do not rush them.

------------------------------------------------------------------------

# 3. Recommended Learning Rule

For the early exercises:

> **Do not ask AI to write the solution.**

Use AI for:

-   explaining compiler errors;
-   asking why something works;
-   reviewing code you wrote;
-   suggesting alternative idiomatic approaches;
-   explaining unfamiliar library APIs.

Avoid:

> "Build this exercise for me."

Instead ask:

> "Here is my implementation. Explain what I got wrong without giving me
> the finished code."

The borrow checker is part of the teacher.

If AI continually removes every borrow-checker problem for you, you will
remove one of the most valuable parts of learning Rust.

------------------------------------------------------------------------

# 4. Stage 0 --- Tooling

Install and understand:

``` bash
rustup
rustc
cargo
rustfmt
clippy
```

Know these commands:

``` bash
cargo new project_name
cargo run
cargo build
cargo check
cargo test
cargo fmt
cargo clippy
```

During learning, use:

``` bash
cargo check
```

constantly.

It is faster than performing a complete build and gives you compiler
feedback.

## Exercise

Create:

``` text
opencry-learning/
```

with:

``` bash
cargo new opencry-learning
cd opencry-learning
cargo run
```

Then deliberately introduce:

-   a type error;
-   an undefined variable;
-   an invalid mutable assignment.

Read the compiler messages.

Rust's compiler diagnostics are an important part of the development
workflow.

------------------------------------------------------------------------

# 5. Stage 1 --- Values, Variables and Types

Rust is statically typed.

Basic examples:

``` rust
let price: f64 = 42.17;
let volume: f64 = 1200.0;
let symbol: &str = "XAGUSD";
let active: bool = true;
```

Variables are immutable by default.

``` rust
let price = 42.17;
price = 42.18;
```

will fail.

Use:

``` rust
let mut price = 42.17;
price = 42.18;
```

This is intentional.

Rust makes mutation explicit.

## Learn

Understand:

-   `let`
-   `mut`
-   type inference
-   scalar types
-   tuples
-   arrays
-   expressions versus statements
-   functions
-   blocks

## OpenCry exercise

Create:

``` rust
fn price_change(open: f64, current: f64) -> f64
```

that returns percentage price change.

Then create:

``` rust
fn is_high_activity(volume: f64, average_volume: f64) -> bool
```

Write tests for both.

------------------------------------------------------------------------

# 6. Stage 2 --- Structs

Structs model domain objects.

Example:

``` rust
struct Instrument {
    symbol: String,
    name: String,
    price: f64,
}
```

Instantiate:

``` rust
let silver = Instrument {
    symbol: String::from("XAGUSD"),
    name: String::from("Silver"),
    price: 42.17,
};
```

Methods are implemented separately:

``` rust
impl Instrument {
    fn update_price(&mut self, new_price: f64) {
        self.price = new_price;
    }
}
```

Notice:

``` rust
&mut self
```

This means the method borrows the object mutably.

That distinction becomes extremely important.

## Exercise

Create:

``` rust
struct Candle {
    open: f64,
    high: f64,
    low: f64,
    close: f64,
    volume: f64,
}
```

Implement:

``` rust
impl Candle {
    fn range(&self) -> f64
    fn body_size(&self) -> f64
    fn is_bullish(&self) -> bool
}
```

Write tests.

------------------------------------------------------------------------

# 7. Stage 3 --- Ownership

This is where learning Rust actually begins.

Consider:

``` rust
let symbol = String::from("XAGUSD");
let another = symbol;
```

You might expect both variables to contain the string.

They do not.

Ownership has moved to `another`.

Using:

``` rust
println!("{symbol}");
```

after the move fails.

Why?

Because Rust wants exactly one owner responsible for releasing the heap
allocation.

Conceptually:

``` text
symbol
   |
   v
"XAGUSD" on heap

then:

another
   |
   v
"XAGUSD" on heap

symbol no longer owns it
```

There is no garbage collector deciding later when this memory can
disappear.

When `another` leaves scope, Rust automatically drops the `String`.

This is one of the central reasons Rust can provide memory safety
without garbage collection.

------------------------------------------------------------------------

# 8. Copy Versus Move

This works:

``` rust
let price = 42.17;
let another_price = price;

println!("{price}");
println!("{another_price}");
```

because `f64` implements `Copy`.

The value is cheap to duplicate.

A heap-backed `String` does not automatically copy its underlying
allocation.

Understand the distinction between:

-   move;
-   copy;
-   clone.

You can explicitly duplicate a `String`:

``` rust
let symbol = String::from("XAGUSD");
let another = symbol.clone();
```

But do not learn to fix ownership problems by spraying `.clone()`
everywhere.

That hides the design problem instead of teaching you ownership.

------------------------------------------------------------------------

# 9. Stage 4 --- Borrowing

Usually you do not want a function to take ownership.

Bad for many cases:

``` rust
fn print_symbol(symbol: String) {
    println!("{symbol}");
}
```

Calling it transfers ownership.

Instead:

``` rust
fn print_symbol(symbol: &String) {
    println!("{symbol}");
}
```

The function borrows the value.

Better still for text:

``` rust
fn print_symbol(symbol: &str) {
    println!("{symbol}");
}
```

Now the caller keeps ownership.

Conceptually:

``` text
owner
  |
  v
String
  ^
  |
borrower
```

The borrower may use the data but does not own it.

------------------------------------------------------------------------

# 10. Mutable Borrowing

To modify borrowed data:

``` rust
fn update_price(instrument: &mut Instrument, new_price: f64) {
    instrument.price = new_price;
}
```

The caller must also allow mutation:

``` rust
let mut silver = Instrument { ... };

update_price(&mut silver, 42.50);
```

The crucial rule is roughly:

> You may have many immutable references OR one mutable reference to a
> value at a time.

This prevents data races.

For example, Rust prevents one piece of code from modifying a collection
while another part is relying on a reference into it.

## Critical exercise

Write a program that:

1.  creates a `Vec<Candle>`;
2.  borrows the final candle;
3.  attempts to push another candle;
4.  tries to use the original borrowed candle afterwards.

Observe the compiler error.

Do not immediately fix it.

Explain to yourself **why allowing this could be unsafe** if the vector
reallocates its backing memory.

This exercise gets to the heart of Rust.

------------------------------------------------------------------------

# 11. Stage 5 --- Slices

You frequently do not need ownership of an entire collection.

Use slices:

``` rust
fn average_close(candles: &[Candle]) -> f64 {
    // ...
}
```

The function can inspect candles without owning the `Vec`.

This is exactly the kind of API OpenCry should use.

For example:

``` rust
fn calculate_volatility(candles: &[Candle]) -> f64
```

rather than transferring ownership of historical market data.

## Exercise

Implement:

``` rust
fn highest_high(candles: &[Candle]) -> Option<f64>
fn lowest_low(candles: &[Candle]) -> Option<f64>
fn average_volume(candles: &[Candle]) -> Option<f64>
```

Notice the use of `Option`.

That leads to the next important Rust concept.

------------------------------------------------------------------------

# 12. Stage 6 --- Option

Rust does not use null in the conventional way.

A value that might not exist is represented explicitly:

``` rust
Option<T>
```

which is:

``` rust
Some(value)
```

or:

``` rust
None
```

Example:

``` rust
fn latest_price(prices: &[f64]) -> Option<f64> {
    prices.last().copied()
}
```

Now callers are forced to deal with absence.

This is powerful for OpenCry.

A trader might have:

``` rust
position: Option<Position>
```

rather than a nullable position.

A market may have:

``` rust
last_trade: Option<Trade>
```

Invalid states become harder to represent accidentally.

------------------------------------------------------------------------

# 13. Stage 7 --- Enums

Rust enums are much more powerful than enums in many languages.

Example:

``` rust
enum Decision {
    Buy,
    Sell,
    Hold,
    Exit,
}
```

But variants can also carry data:

``` rust
enum Decision {
    Buy { quantity: f64 },
    Sell { quantity: f64 },
    Hold,
    Exit,
}
```

This is extremely useful for modelling OpenCry.

Another example:

``` rust
enum MarketEvent {
    Trade(TradeEvent),
    Quote(QuoteEvent),
    Candle(CandleEvent),
}
```

This says:

> A MarketEvent is exactly one of these valid forms.

That is far better than passing loosely structured maps or strings
around the application.

------------------------------------------------------------------------

# 14. Pattern Matching

Enums become particularly powerful with `match`.

``` rust
match decision {
    Decision::Buy { quantity } => {
        println!("Buying {quantity}");
    }
    Decision::Sell { quantity } => {
        println!("Selling {quantity}");
    }
    Decision::Hold => {
        println!("Holding");
    }
    Decision::Exit => {
        println!("Exit position");
    }
}
```

Rust checks exhaustiveness.

If you later add:

``` rust
Decision::Reduce
```

the compiler can identify code that no longer handles every possible
decision.

This is enormously useful in a large stateful system.

## Exercise

Create:

``` rust
enum MarketEvent {
    Trade { price: f64, quantity: f64 },
    Quote { bid: f64, ask: f64 },
    SessionOpened,
    SessionClosed,
}
```

Write:

``` rust
fn describe_event(event: &MarketEvent) -> String
```

using `match`.

Then add another variant and observe what the compiler tells you.

------------------------------------------------------------------------

# 15. Stage 8 --- Result and Error Handling

Rust makes recoverable errors explicit.

A function may return:

``` rust
Result<T, E>
```

which is:

``` rust
Ok(value)
```

or:

``` rust
Err(error)
```

Example:

``` rust
fn parse_price(value: &str) -> Result<f64, ParseFloatError> {
    value.parse::<f64>()
}
```

The `?` operator propagates errors:

``` rust
fn process_price(value: &str) -> Result<f64, ParseFloatError> {
    let price = value.parse::<f64>()?;
    Ok(price * 2.0)
}
```

Learn:

-   `Result`;
-   `?`;
-   custom error enums;
-   when to use `panic!`;
-   why application errors should usually not panic.

OpenCry's market feed disconnecting is not a reason to crash the
process.

It is a recoverable operational event.

------------------------------------------------------------------------

# 16. Stage 9 --- Collections

Learn the standard collections you will actually use.

## Vec

``` rust
Vec<T>
```

Dynamic contiguous array.

Likely everywhere in analytics.

## VecDeque

``` rust
VecDeque<T>
```

Useful for rolling market windows.

Example:

``` text
oldest -> [ candle candle candle candle ] <- newest
```

Push newest, discard oldest.

Excellent for:

-   last 100 candles;
-   recent trades;
-   rolling volatility windows.

## HashMap

``` rust
HashMap<K, V>
```

Useful for:

``` text
InstrumentId -> MarketState
FirmId       -> FirmState
```

## Exercise

Create a rolling candle buffer with maximum size 100.

When candle 101 arrives, discard candle 1.

Calculate:

-   average close;
-   average volume;
-   high;
-   low.

This is directly relevant to OpenCry.

------------------------------------------------------------------------

# 17. Stage 10 --- Iterators

Rust makes heavy use of iterators.

Instead of manually indexing:

``` rust
for candle in candles.iter() {
    println!("{}", candle.close);
}
```

You can transform data:

``` rust
let average =
    candles
        .iter()
        .map(|c| c.close)
        .sum::<f64>()
        / candles.len() as f64;
```

Learn:

-   `.iter()`;
-   `.iter_mut()`;
-   `.into_iter()`;
-   `.map()`;
-   `.filter()`;
-   `.filter_map()`;
-   `.fold()`;
-   `.collect()`;
-   `.enumerate()`.

Most importantly, understand how the three iterator forms relate to
ownership.

Ask:

> Does this iterator borrow, mutably borrow, or consume the collection?

That is more important than memorising the methods.

------------------------------------------------------------------------

# 18. Stage 11 --- Traits

Traits define shared behaviour.

This will be fundamental to OpenCry.

Example:

``` rust
trait Strategy {
    fn evaluate(&mut self, market: &MarketState) -> Decision;
}
```

Then:

``` rust
struct MomentumStrategy;
struct MeanReversionStrategy;
```

Both can implement `Strategy`.

``` rust
impl Strategy for MomentumStrategy {
    fn evaluate(&mut self, market: &MarketState) -> Decision {
        // ...
    }
}
```

Traits are somewhat analogous to interfaces, but Rust traits are more
powerful.

OpenCry will likely use traits for:

-   market-data sources;
-   strategies;
-   AI decision models;
-   persistence;
-   research evaluators.

## Exercise

Implement:

``` rust
trait Strategy {
    fn name(&self) -> &str;
    fn evaluate(&mut self, candles: &[Candle]) -> Decision;
}
```

Create:

-   `MomentumStrategy`;
-   `MeanReversionStrategy`.

Do not make them sophisticated.

The purpose is understanding polymorphism.

------------------------------------------------------------------------

# 19. Stage 12 --- Generics

Generics allow reusable strongly typed code.

Example:

``` rust
struct RollingBuffer<T> {
    values: VecDeque<T>,
    capacity: usize,
}
```

Now it can store:

``` rust
RollingBuffer<Candle>
RollingBuffer<Trade>
RollingBuffer<f64>
```

Implement:

``` rust
impl<T> RollingBuffer<T> {
    fn push(&mut self, value: T) {
        // ...
    }
}
```

This is a useful OpenCry learning exercise because it combines:

-   ownership;
-   collections;
-   structs;
-   generics.

------------------------------------------------------------------------

# 20. Stage 13 --- Lifetimes

Do not start by trying to memorise lifetime syntax.

First understand the problem lifetimes solve.

Consider:

``` rust
fn latest(candles: &[Candle]) -> Option<&Candle>
```

The returned reference must not outlive the collection it points into.

Rust usually infers this.

Conceptually:

``` text
Vec<Candle>
+-------------------+
| C1 | C2 | C3 | C4 |
+-------------------+
              ^
              |
         returned &Candle
```

If the vector disappears, that reference cannot remain valid.

Rust uses lifetimes to prove this at compile time.

Only start writing explicit lifetime annotations when the compiler
requires them.

Example:

``` rust
fn choose<'a>(a: &'a Candle, b: &'a Candle) -> &'a Candle
```

The important mental model:

> A lifetime describes relationships between references. It does not
> manually control how long memory lives.

## Exercise

Write:

``` rust
fn highest_candle(candles: &[Candle]) -> Option<&Candle>
```

Then try to write a function that returns a reference to a `Candle`
created locally inside the function.

Observe why Rust refuses.

------------------------------------------------------------------------

# 21. Stage 14 --- Heap Ownership: Box

`Box<T>` stores a value on the heap and owns it.

Example:

``` rust
let strategy: Box<dyn Strategy> =
    Box::new(MomentumStrategy);
```

This becomes useful when you want runtime polymorphism.

For example:

``` rust
Vec<Box<dyn Strategy>>
```

could contain different strategy implementations.

Learn why the `Box` is required for trait objects.

Do not obsess over smart pointers yet.

------------------------------------------------------------------------

# 22. Stage 15 --- Shared Ownership: Rc and Arc

Sometimes multiple things genuinely need ownership of the same data.

Single-threaded:

``` rust
Rc<T>
```

Thread-safe:

``` rust
Arc<T>
```

`Arc` means atomically reference counted.

Conceptually:

``` text
Task A ----\
           \
Task B -----> Shared Market Configuration
           /
Task C ----/
```

When the final `Arc` disappears, the value is dropped.

For OpenCry, immutable shared configuration may naturally use `Arc`.

Do not use `Arc` simply to make ownership errors disappear.

------------------------------------------------------------------------

# 23. Stage 16 --- Mutex and RwLock

If shared data must be mutated across threads, Rust requires
synchronisation.

Example:

``` rust
Arc<Mutex<Portfolio>>
```

Multiple tasks share ownership through `Arc`.

Only one may mutate/access the protected state at a time through the
mutex.

`RwLock` allows:

-   multiple readers;
-   or one writer.

However:

> Do not design OpenCry as one enormous `Arc<Mutex<World>>`.

That technically compiles but defeats much of the architectural benefit.

Prefer clear ownership and message passing.

------------------------------------------------------------------------

# 24. Stage 17 --- Threads and Send/Sync

Rust prevents unsafe cross-thread sharing at compile time.

Learn the concepts:

## Send

A type can safely be transferred to another thread.

## Sync

References to a type can safely be shared between threads.

You do not normally implement these manually.

The compiler derives them based on the fields in your types.

This is one of the major CV-worthy aspects of the project:

OpenCry can process concurrent market feeds and simulation work while
Rust statically prevents many classes of data race.

------------------------------------------------------------------------

# 25. Stage 18 --- Channels

Channels are likely to be more useful to OpenCry than shared mutable
state.

Conceptually:

``` text
Market Feed
     |
     | MarketEvent
     v
  CHANNEL
     |
     v
Simulation
```

The producer owns an event and sends it.

Ownership transfers through the channel.

That maps beautifully onto Rust.

Then:

``` text
Feed -> Normaliser -> Recorder
                  \
                   -> Simulation -> Analytics
```

Learn:

-   sender;
-   receiver;
-   bounded channels;
-   backpressure.

The important architectural question becomes:

> Who owns this event now?

Rust makes the answer concrete.

------------------------------------------------------------------------

# 26. Stage 19 --- Async Rust

OpenCry will consume WebSockets and other I/O.

You therefore need async Rust.

Learn:

``` rust
async fn
.await
```

Then learn Tokio.

Conceptually:

``` rust
async fn receive_market_data() {
    loop {
        let event = websocket.next().await;
        // ...
    }
}
```

Async is particularly appropriate for waiting on:

-   WebSockets;
-   HTTP;
-   AI responses;
-   file/network I/O.

Do not confuse async with CPU parallelism.

Async helps efficiently manage waiting.

CPU-heavy analytics may need threads/parallel execution instead.

------------------------------------------------------------------------

# 27. Stage 20 --- Tokio

Learn enough Tokio to understand:

``` rust
#[tokio::main]
async fn main()
```

and:

``` rust
tokio::spawn(...)
```

plus Tokio channels and timers.

## OpenCry exercise

Build a tiny fake market feed.

Task 1 generates a random-ish price event every 250 ms.

Task 2 receives events through a channel.

Task 3 records/prints them.

Do not connect to a real exchange yet.

The objective is understanding asynchronous ownership transfer.

------------------------------------------------------------------------

# 28. Stage 21 --- Modules and Crates

Before OpenCry becomes large, understand Rust's code organisation:

-   package;
-   crate;
-   module;
-   workspace;
-   `pub`;
-   `use`;
-   `mod`.

Create a workspace:

``` text
rust-market-lab/
|
|-- Cargo.toml
`-- crates/
    |-- market-types/
    |-- strategies/
    `-- simulator/
```

Have one crate depend on another.

This prepares you for OpenCry's eventual workspace.

------------------------------------------------------------------------

# 29. Stage 22 --- Serde

OpenCry will serialise:

-   market events;
-   configuration;
-   recordings;
-   experiment results.

Learn Serde.

Example:

``` rust
#[derive(Serialize, Deserialize)]
struct TradeEvent {
    symbol: String,
    price: f64,
    quantity: f64,
}
```

Serialise a few events to JSON.

Read them back.

Later, OpenCry may use a more efficient recording format, but JSON is
excellent for learning.

------------------------------------------------------------------------

# 30. Stage 23 --- Testing

Rust has excellent built-in testing.

Example:

``` rust
#[test]
fn bullish_candle_is_detected() {
    let candle = Candle {
        open: 10.0,
        high: 12.0,
        low: 9.0,
        close: 11.0,
        volume: 100.0,
    };

    assert!(candle.is_bullish());
}
```

For OpenCry, financial state should be heavily tested.

Particularly:

-   positions;
-   P&L;
-   risk;
-   event ordering;
-   strategies;
-   replay determinism.

Get into the habit now.

------------------------------------------------------------------------

# 31. Stage 24 --- Bevy ECS

Only begin Bevy after the earlier Rust concepts make sense.

Traditional OOP might model:

``` text
Trader
  |- position
  |- firm
  |- strategy
  |- animation
  |- location
```

ECS instead separates components:

``` text
Entity 123

Trader
Firm(Atlas)
Position(Long)
Strategy(Momentum)
Transform(...)
AnimationState(Running)
```

Systems query combinations of components.

Conceptually:

``` rust
fn update_traders(
    query: Query<(&Trader, &mut Position)>
) {
    // ...
}
```

Bevy and Rust work well together because systems declare what data they
read and mutate.

That enables safe parallel scheduling.

A system requesting:

``` text
&Position
```

only reads.

A system requesting:

``` text
&mut Position
```

mutates.

Bevy can reason about which systems can safely execute concurrently.

This will make far more sense after you understand ordinary Rust
borrowing.

------------------------------------------------------------------------

# 32. Stage 25 --- First Bevy Exercise

Do not begin with OpenCry.

Create:

**100 moving circles.**

Each entity has:

``` text
Trader
Velocity
Transform
```

Systems:

``` text
spawn_traders
move_traders
bounce_at_edges
```

Then increase:

``` text
100
1,000
10,000
```

Observe performance.

This teaches:

-   entities;
-   components;
-   resources;
-   systems;
-   queries;
-   commands.

Then add:

``` text
Firm::Atlas
Firm::Redwood
Firm::Northstar
```

Now you have the primitive ancestor of the OpenCry floor.

------------------------------------------------------------------------

# 33. Your Learning Project

Before starting the real OpenCry repository, build a small project
called:

# Rust Market Lab

This is disposable educational code.

It should contain no production pressure.

Build it yourself.

## Version 1

Create:

``` text
Instrument
Candle
Trade
```

Learn structs and ownership.

## Version 2

Create:

``` text
MarketEvent
```

as an enum.

Learn matching.

## Version 3

Create a:

``` text
RollingBuffer<Candle>
```

Learn generics and collections.

## Version 4

Implement:

``` text
Strategy
  |- Momentum
  `- MeanReversion
```

Learn traits.

## Version 5

Create:

``` text
Trader
Position
Decision
```

Learn domain modelling.

## Version 6

Run 100 traders over fake candles.

Learn borrowing and state mutation.

## Version 7

Send fake market events over a channel.

Learn ownership transfer.

## Version 8

Generate events asynchronously.

Learn Tokio.

## Version 9

Record events with Serde.

Learn persistence.

## Version 10

Replay exactly the same events.

Learn deterministic architecture.

## Version 11

Render traders in Bevy.

Learn ECS.

At that point:

**start OpenCry.**

You will already understand most of the architectural primitives the
coding agent is going to use.

------------------------------------------------------------------------

# 34. A Suggested Four-Week Path

This does not need to take months.

Because you already understand software engineering, approximately four
focused weeks should give you a strong foundation.

## Week 1 --- Rust's mental model

Focus entirely on:

-   variables/types;
-   structs;
-   ownership;
-   moves;
-   borrowing;
-   mutable borrowing;
-   slices;
-   `Option`;
-   enums;
-   `match`;
-   `Result`.

Build small market-domain exercises.

Do not touch Bevy.

### Exit criterion

You can explain why this does or does not compile without guessing:

``` rust
let mut candles = Vec::new();

candles.push(candle);

let latest = &candles[0];

candles.push(another_candle);

println!("{}", latest.close);
```

If you cannot explain it, remain on ownership/borrowing.

------------------------------------------------------------------------

## Week 2 --- Idiomatic Rust

Learn:

-   collections;
-   iterators;
-   traits;
-   generics;
-   lifetimes;
-   `Box`;
-   modules;
-   crates;
-   testing;
-   Serde.

Build the first half of Rust Market Lab.

### Exit criterion

You can design:

``` rust
trait Strategy
```

and implement multiple strategies without AI deciding the architecture
for you.

------------------------------------------------------------------------

## Week 3 --- Concurrent Rust

Learn:

-   `Arc`;
-   `Mutex`;
-   `RwLock`;
-   `Send`;
-   `Sync`;
-   channels;
-   async/await;
-   Tokio;
-   tasks;
-   error handling in async systems.

Build:

``` text
Fake Feed
    |
    v
Channel
    |
    +----> Recorder
    |
    `----> Simulation
```

### Exit criterion

You can explain where each `MarketEvent` is owned at every stage.

------------------------------------------------------------------------

## Week 4 --- Bevy

Learn:

-   App;
-   plugins;
-   entities;
-   components;
-   resources;
-   systems;
-   queries;
-   events;
-   schedules;
-   transforms;
-   camera;
-   basic rendering.

Build the 100/1,000/10,000 trader experiment.

Then connect your fake market feed.

### Exit criterion

You can explain why Bevy ECS is a sensible architecture for OpenCry and
how its query borrowing relates to ordinary Rust borrowing.

------------------------------------------------------------------------

# 35. Things You Should Be Able to Explain Before AI Builds OpenCry

Before handing major implementation work to a coding agent, make sure
you can answer these in your own words.

### Ownership

What does it mean for a value to have an owner?

### Move

Why does assigning a `String` normally move it?

### Copy

Why does assigning an `f64` normally leave the original usable?

### Clone

Why is `.clone()` different from borrowing?

### Borrow

What is `&T`?

### Mutable borrow

What is `&mut T`?

### Borrowing rule

Why can't arbitrary mutable and immutable references coexist?

### Slice

Why would a strategy accept `&[Candle]` instead of `Vec<Candle>`?

### Option

Why is `Option<Position>` preferable to a nullable position?

### Result

Why should a market-feed disconnect produce `Result` rather than a
panic?

### Enum

Why is `MarketEvent` naturally an enum?

### Match

What does exhaustive matching buy you?

### Trait

Why should market-data providers implement a common trait?

### Generic

What does `RollingBuffer<T>` buy you?

### Lifetime

What problem do lifetimes solve?

### Box

Why might `Box<dyn Strategy>` be useful?

### Arc

When is shared ownership actually necessary?

### Mutex

Why isn't `Arc<Mutex<Everything>>` a good architecture?

### Channel

How does sending a value through a channel interact with ownership?

### Async

Why is async useful for market WebSockets?

### Send / Sync

What guarantees do these marker traits represent?

### ECS

Why are OpenCry traders better represented as entities/components than
giant mutable objects?

If you can comfortably explain these, you have enough Rust knowledge to
start reviewing AI-generated OpenCry code critically.

------------------------------------------------------------------------

# 36. How to Use AI Without Letting It Steal the Learning

A good workflow:

### Step 1

Write the code yourself.

### Step 2

Run:

``` bash
cargo check
```

### Step 3

Read the compiler error yourself.

### Step 4

Try to explain the error.

### Step 5

Attempt a fix.

### Step 6

Only then ask AI:

> Explain why the compiler rejected my original version. Do not rewrite
> it for me.

After fixing it, ask:

> Is this idiomatic Rust? Point out problems but let me make the
> changes.

For unfamiliar APIs, using AI more heavily is fine.

The important thing is that AI should not repeatedly make ownership and
architectural decisions that you do not understand.

------------------------------------------------------------------------

# 37. Anti-Patterns to Watch for in AI-Generated Rust

AI can produce Rust that compiles while avoiding good Rust design.

Be suspicious of excessive:

``` rust
.clone()
```

It may be avoiding ownership reasoning.

Be suspicious of:

``` rust
Arc<Mutex<...>>
```

everywhere.

It may be converting Rust into a globally locked object graph.

Be suspicious of unnecessary:

``` rust
String
```

where borrowed:

``` rust
&str
```

would suffice.

Be suspicious of:

``` rust
unwrap()
expect()
```

throughout operational code.

Be suspicious of huge structs containing the entire application state.

Be very suspicious of:

``` rust
unsafe
```

without measured need.

And be suspicious when the coding agent says:

> "This is necessary because of Rust's borrow checker."

Sometimes it is.

Sometimes it simply chose a poor ownership model.

You should eventually be able to tell the difference.

------------------------------------------------------------------------

# 38. What Not to Learn Yet

Do not delay OpenCry because you have not mastered every advanced Rust
topic.

You do not initially need:

-   unsafe Rust;
-   procedural macros;
-   advanced macro_rules;
-   pinning internals;
-   custom allocators;
-   compiler internals;
-   advanced FFI;
-   no_std;
-   manual SIMD;
-   lock-free data structures;
-   advanced lifetime wizardry.

Learn those only if the project creates a genuine need.

------------------------------------------------------------------------

# 39. Core Mental Model

When reading Rust, continually ask four questions:

## 1. Who owns this?

``` text
Who is responsible for this value?
```

## 2. Who is borrowing this?

``` text
&T or &mut T?
```

## 3. How long must the reference remain valid?

``` text
What owns the thing it points to?
```

## 4. What happens when ownership ends?

``` text
When is the value dropped?
```

Then, in concurrent code, add:

## 5. Which task/thread owns this now?

That mental model will take you much further than memorising syntax.

------------------------------------------------------------------------

# 40. The Point Where Rust Should Click

Initially Rust can feel as though the compiler is obstructing perfectly
reasonable code.

The important transition is when you stop thinking:

> "How do I make the borrow checker accept this?"

and start thinking:

> "What ownership model am I actually trying to express?"

For OpenCry, that question appears everywhere:

``` text
Who owns market state?

Who owns an incoming MarketEvent?

Does a strategy need ownership or merely a borrow?

Who can mutate a trader's position?

Should two systems share state or exchange messages?

How does a replay event move through the pipeline?

Who owns the AI result?

Can this work happen concurrently?
```

Those are not Rust-specific annoyances.

They are real architectural questions that Rust forces you to answer
explicitly.

That is precisely why OpenCry is a good project for learning the
language.

------------------------------------------------------------------------

# 41. First Practical Session

Do this before any OpenCry implementation.

Create:

``` bash
cargo new rust-market-lab
cd rust-market-lab
```

Implement only:

``` rust
struct Candle {
    open: f64,
    high: f64,
    low: f64,
    close: f64,
    volume: f64,
}
```

Then implement:

``` rust
impl Candle {
    fn range(&self) -> f64;
    fn body_size(&self) -> f64;
    fn is_bullish(&self) -> bool;
}
```

Then create:

``` rust
fn average_close(candles: &[Candle]) -> Option<f64>
```

Then deliberately explore ownership:

1.  Put several candles in a `Vec<Candle>`.
2.  Borrow one.
3.  Mutate the vector.
4.  Observe compiler behaviour.
5.  Move the vector into a function.
6.  Observe that the caller loses ownership.
7.  Change the function to borrow the vector.
8.  Observe the difference.
9.  Create a mutable borrow.
10. Try creating conflicting borrows.

Do not rush this exercise.

It teaches more useful Rust for OpenCry than immediately rendering an
isometric trading floor.

------------------------------------------------------------------------

# 42. Final Learning Milestone

You are ready to move from **Rust Market Lab** to **OpenCry** when you
can build, largely yourself:

``` text
Fake async market feed
        |
        v
typed MarketEvent
        |
        v
bounded channel
        |
        +----------+
        |          |
        v          v
    Recorder    Simulation
                   |
                   v
               Strategies
                   |
                   v
                Traders
                   |
                   v
              Bevy entities
```

and explain the ownership of the data at every arrow.

At that point, let the coding agent accelerate you.

But keep reviewing its Rust.

OpenCry should not merely become an application written in Rust.

By the end of the project, **you should be a Rust programmer**.
