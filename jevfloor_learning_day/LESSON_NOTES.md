# JevFloor learning day — tutor notes

Package: `jevfloor_learning_day/`, one file per lesson in `examples/`.
Run: `cargo run --example NAME` · Test: `cargo test --example NAME` · Fast check: `cargo check --example NAME`

---

## Lesson 1 — Cargo and the toolchain ✅ DONE

**Concept**
- `rustc` = the compiler. Cargo = package manager + build tool (dependencies, targets, profiles, tests).
- `cargo check` — runs the compiler front end (parse, types, **borrow checker**) but skips codegen + linking. Fastest loop; every ownership/borrow error today shows up here.
- `cargo build` — full compile to a binary in `target/`.
- `cargo run` — build then execute.
- `cargo test` — separate `test`-profile build including `#[cfg(test)]` code; runs every `#[test]`.

**Test module (reference)**
```rust
#[cfg(test)]
mod tests {
    #[test]
    fn price_delta_is_three() {
        let open = 100;
        let close = 103;
        assert_eq!(close - open, 3);
    }
}
```
- `#[cfg(test)]` — compiled only for `cargo test`, absent from the `cargo run` binary.
- `mod tests` — inline module → test is reported as `tests::price_delta_is_three`.
- `#[test]` — marks a no-arg function as a test.
- `assert_eq!(left, right)` — panics on mismatch (panic = test failure) and prints both values.

**Checkpoint answers**
1. `cargo check` checks without running (no codegen/link → fastest).
2. Execution begins at `main`.
3. Cargo manages dependencies, project structure, targets, profiles, tests; `rustc` just compiles.

---

## Lesson 2 — Structs, enums and exhaustive `match` ✅ DONE

**Concept:** `struct` groups named fields. `enum` = a set of alternatives, each variant can carry its own data (a tagged union, not C-style integers). `match` must cover every variant — add a variant later and the compiler shows you every `match` that doesn't handle it.

**Task** — `examples/l02_types.rs`
1. `Candle`: `open`, `high`, `low`, `close` as `i64`; `volume: u64`.
2. `MarketEvent`: `Tick { symbol: String, price: i64 }` and `Closed(Candle)`.
3. `fn describe(event: MarketEvent) -> String` using `match` → `tick SILVER 103` / `candle close=103`.
4. In `main`: build one of each, put them in a `Vec`, loop, print `describe(...)`.
5. Then add a `Disconnected` variant (no data). `cargo check` **before** updating the `match` → read the error → handle it so it prints `disconnected`.

**Syntax hints**
```rust
MarketEvent::Tick { symbol, price } => ...   // variant with named fields
MarketEvent::Closed(candle) => ...           // variant with one unnamed value; then candle.close
format!("tick {symbol} {price}")             // build a String
"SILVER".to_string()                         // String from a literal
for event in events { ... }                  // fine today; lesson 10 explains what it does to the Vec
```

**Target output**
```
tick SILVER 103
candle close=103
disconnected
```

**Predict before step 5:** warning or error? Does it point at the enum or at the `match`?

**Checkpoint:** Why is an enum better here than a string tag plus several optional fields?

**Takeaways**
- Unhandled variant = hard error **E0004 non-exhaustive patterns**, pointing at the `match` (and noting where the variant is defined).
- Don't silence it with `_ =>` — that disables the protection for future variants.
- Enum vs tag + `Option`s: impossible states unrepresentable (a `Tick` always has symbol+price, never a candle); typos and new kinds caught at compile time instead of runtime.

---

## Lesson 3 — Ownership and moves ✅ DONE

### 🧸 Part A — The mystery box
- **One owner at a time.** Every box has exactly one kid's name on it.
- **Giving it away = a move.** Hand your box to Sam → Mum crosses your name off, writes Sam's.
- **Can't open a box you gave away.** "You gave that to Sam, it's not yours any more." ← the compiler error.
- **Owner goes home → box goes in the bin** (end of function = **drop**). Nobody has to remember to tidy up.
- **Getting it back.** Sam hands it back when finished → your name goes back on it.
- **Moving doesn't copy the toys.** Same box, new hands.

### 📈 Part B — The market
| Box world | Market world |
|---|---|
| The box | `MarketEvent::Tick { symbol: "SILVER", price: 103 }` |
| You | `main` |
| Sam | `fn consume(event: MarketEvent)` |
| Handing it over | calling `consume(event)` |
| "Not yours any more" | error **E0382: use of moved value** |
| Sam hands it back | `consume` returns the event: `-> MarketEvent` |

A tick moves because it holds a `String` (owns heap memory); two owners would mean a double free. A move copies only the `String` header (ptr, len, cap), not the text.

**Task** — `examples/l03_moves.rs`
1. `MarketEvent` with just `Tick { symbol: String, price: i64 }`.
2. `fn consume(event: MarketEvent)` prints `consumer saw SILVER`.
3. `main`: create a SILVER tick, `consume(event);`, then try to use `event` again.
4. Predict: what does the compiler say — points at the hand-over line, the peek line, or both?
5. `cargo check --example l03_moves`.
6. Repair, **no `.clone()`**: `consume(...) -> MarketEvent { ...; event }` and `let event = consume(event);` → print `caller owns SILVER again`.

```rust
if let MarketEvent::Tick { symbol, .. } = &event {   // one-arm match; `&` = look without taking (lesson 5)
    println!("caller owns {symbol} again");
}
```

**Target output**
```
consumer saw SILVER
caller owns SILVER again
```

**Checkpoint:** Who owns the tick at each step, and when does `"SILVER"` get dropped?

**What we actually did (simplified)**
- Broken: `consume(tick); consume(tick);` → **E0382 use of moved value: `tick`** (giving Sam the same box twice).
- Fixed: `let tick = consume(tick); consume(tick);` → catch the box Sam hands back, then give it again. Prints `consumer saw SILVER` twice.

**Takeaways**
- The move happens **at the call** `consume(tick)` — your hands are empty immediately. Sam handing it back only helps if you **catch** it with a new `let`.
- A value is dropped when its **last owner lets go**: end of the owner's scope, or straight away at the `;` if a returned value is caught by nobody.
- In the fixed version `"SILVER"` is dropped at the `;` of the last `consume(tick);` — before the end of `main`.

**Extra (building the "complicated" line one piece at a time)**
- `match tick { Tick { symbol, price } => ... }` — `{ symbol, price }` **destructures** (TS `const { symbol, price } = event`); it doesn't check the fields exist. Comma separates arms.
- `price` unused → warning → `{ symbol, .. }` ignores the other fields.
- `match tick` moved the `String` out → **E0382 use of *partially* moved value** (box missing its name tag). Fix: `match &tick` = peek, don't take.
- One-arm `match` ≡ `if let PATTERN = VALUE { ... }` (TS `if (event.kind === "tick")`).
- With only one enum variant, `if let` warns **irrefutable** (always matches). Add a second variant (`Disconnected`) and the warning goes.

---

## Lesson 4 — `Copy` vs `Clone` ✅ DONE

### 🧸 Part A — The mystery box
- **Number on a card** ("103") → scribble a new card, instant, automatic → **`Copy`**.
- **A teddy** → can't photocopy it; someone must *build another*, so you must ask out loud → **`Clone`** (explicit, may cost).
- Don't ask → the teddy just **moves** (lesson 3).
- A `String` is a **claim ticket** for a name tag stored in the cupboard. Photocopy the ticket → two kids bin the same tag → forbidden. So anything holding a `String` can't be `Copy`.

### 📈 Part B — The market
| Box world | Market world |
|---|---|
| Number on a card | `price: i64` — integers are `Copy` |
| Teddy / claim ticket | `symbol: String` — owns heap memory, not `Copy` |
| "Please make another teddy" | `.clone()` — explicit, allocates |
| Two kids binning one tag | double free (prevented) |

TS: numbers copy (same as Rust); objects are *shared* in TS, but in Rust a `String` **moves** unless you `.clone()` (≈ `structuredClone`).

**New syntax:** `let mut x = 5; x = 6;` — variables are immutable unless `mut`.

**Task** — `examples/l04_copy_clone.rs`
1. `let price = 100; let mut copy = price;` → set `copy` to 101 → print `prices original=100 copy=101`. Predict: is `price` still usable?
2. `let symbol = "SILVER".to_string(); let mut clone = symbol.clone(); clone.push_str("-SIM");` → print `symbols original=SILVER clone=SILVER-SIM`. Predict: what if you drop `.clone()` and print `symbol`?
3. Put `#[derive(Clone, Copy)]` above `enum MarketEvent` (string-owning). `cargo check` → read error → remove just `Copy`.

**Target output**
```
prices original=100 copy=101
symbols original=SILVER clone=SILVER-SIM
```

**Checkpoint:** Why are both originals still usable, and which duplication may allocate?

**Takeaways**
- `#[derive(Clone, Copy)]` on a `String`-owning enum → **E0204 the trait `Copy` cannot be implemented**, pointing at the offending field. A type is `Copy` only if **every** field is `Copy`.
- `let x = y;` looks the same for both, but: `Copy` types copy automatically; everything else **moves** unless you `.clone()`.
- `.clone()` of a `String` allocates new heap memory; copying an `i64` just duplicates 8 bytes.
- `println!` is in the prelude — no `use` needed. `copy += 1` works like TS.

---

## Lesson 5 — Shared borrowing (`&`) ✅ DONE

### 🧸 Part A — The mystery box
- **Letting a friend look:** you hold the box open, Sam peeks — can't take it home, can't change anything. Still **your** box.
- **Lots of friends can look at once.**
- **When everyone's finished looking**, you can do anything with it — including give it away.
- **Rule:** a peek can't outlast the box. Mum checks.

### 📈 Part B — The market
| Box world | Market world |
|---|---|
| Letting Sam peek | passing `&event` — a **shared borrow** |
| Sam's hands | a parameter of type `&MarketEvent` |
| Still your box | `main` still owns `event` |
| Give it away afterwards | `consume(event)` — allowed once peeking is over |

TS: objects are shared references anyone can mutate. Rust `&T` = **read-only** reference guaranteed not to outlive the value.

**New syntax:** `fn describe(event: &MarketEvent)` (look only) · `describe(&event)` (lend a look).

**Task** — `examples/l05_borrow.rs`
1. `MarketEvent` (just `Tick`) + `consume(event: MarketEvent)` printing `consumed SILVER` (no return needed).
2. `fn describe(event: &MarketEvent)` → `match` → print `tick SILVER 103`.
3. `main`: create tick, `describe(&event);` twice, then `consume(event);`. **No `.clone()`.**
4. Bonus: move `consume(event);` before the 2nd `describe` → predict + read the error.

**Target output**
```
tick SILVER 103
tick SILVER 103
consumed SILVER
```

**Checkpoint:** Why can the caller still move the event after the shared borrows' last use?

**Takeaways**
- Signature puts `&` on the **type** (`event: &MarketEvent` — "what I accept: a peek"); the call puts `&` on the **value** (`describe(&tick)` — "what I hand over: a peek"). `MarketEvent` and `&MarketEvent` are different types (TS analogy: `Readonly<T>`).
- `::` = path into a type/module (`MarketEvent::Tick`); `.` = field of a value (`candle.close`). `event::symbol` → misleading "unresolved module" error.
- Can't do `event.symbol` on an enum — fields belong to a **variant**; unpack with `match` / `if let` first.
- Field shorthand: `MarketEvent::Tick { symbol, price }` when variable names match (like TS).
- Inside `describe(event: &MarketEvent)`, just `match event` — `match &event` is a peek at a peek (works, unnecessary).
- `describe(&tick); consume(tick); describe(&tick);` → **E0382 borrow of moved value**. Peeks end when the call returns, so you still own it and can give it away afterwards.

---

## Lesson 6 — Mutable borrowing (`&mut`) ✅ DONE

### 🧸 Part A — The mystery box
- Letting Sam **fix** the name tag — still your box.
- **Lots of peekers OR exactly one fixer — never both** (a peeker could read a half-changed tag).
- When Sam's finished, everyone can peek and sees the new tag.
- A peek only counts while it's **still being used** — finished peekers don't block Sam.

### 📈 Part B — The market
| Box world | Market world |
|---|---|
| Letting Sam fix things | `&mut candle` — mutable borrow |
| Sam's hands | parameter `&mut Candle` |
| Many peekers OR one fixer | many `&T` **or** one `&mut T` |
| Peek "still being used" | a borrow lives until its **last use** |

TS: anyone holding a reference can mutate anytime. Rust: mutation is explicit (`&mut`) and exclusive.

**New syntax (one at a time)**
1. `let mut candle = Candle { ... };` — owner must be `mut`.
2. `fn adjust_close(candle: &mut Candle, delta: i64) { candle.close += delta; }`
3. `adjust_close(&mut candle, 3);`
4. `let view = &candle;` — a peek stored in a variable, used later as `view.close`.

**Task** — `examples/l06_mut_borrow.rs`
1. Make it work: print `before=100`, `adjust_close(&mut candle, 3)`, print `after=103`.
2. Break it: `let view = &candle; adjust_close(&mut candle, 3); println!("before={}", view.close);` → predict + `cargo check`.
3. Fix by **reordering only** (no `.clone()`).

**Target output**
```
before=100
after=103
```

**Checkpoint:** Which overlapping accesses conflicted? Why did changing their order help?

**Side track — owned vs borrowed, and where `mut` goes**

| Signature | Owned or borrowed? | Can the function change it? | Caller writes |
|---|---|---|---|
| `candle: Candle` | owned | no | `f(candle)` |
| `mut candle: Candle` | owned | **yes** | `f(candle)` |
| `candle: &Candle` | borrowed | no | `f(&candle)` |
| `candle: &mut Candle` | borrowed | **yes** | `f(&mut candle)` |

- **Left of the colon = about the function's own variable** (like `let` / `let mut`). **Right of the colon = what the caller hands in.**
- Owned: `mut` before the **name** is a *private rule* — the caller writes the same call either way; it gave the candle away.
- Borrowed: `&mut` before the **type** — the caller still owns it, so must agree by writing `&mut` at the call.
- `mut` alone never goes before a type (`candle: mut Candle` ✗), and never in a call (`f(mut candle)` ✗).
- If an owned param is changed and nothing reads it afterwards → warning "assigned to, but never used" (like ESLint `no-unused-vars`): *used* means *read*. Read red errors first; warnings are often knock-ons.

**Mindset shift from TypeScript:** the signature is a contract declaring what the function will do with the data (look `&T` / change `&mut T` / keep `T`). Decide that first; the caller then matches it. Ask for the least you need.

**Lesson 6 takeaways**
- Borrowed + changed: **nothing to give back** — the function changed the caller's value directly; the borrow just ends.
- Semicolons: Rust never inserts them (unlike TS). Every statement needs `;` except the last line of a function when it's the value handed back.
- While a borrow is still going to be used:

| Borrow in use | Owner can read? | change? | give away? |
|---|---|---|---|
| `&` (look) | ✅ | ❌ | ❌ |
| `&mut` (change) | ❌ | ❌ | ❌ |

- A borrow lasts until its **last use** (nothing special about `println!` — it just happened to be the last use). Borrows are never returned; they just stop.
- Fix for "cannot borrow as mutable because it is also borrowed as immutable": move the last use of the look **before** the change.

---

## ▶ LESSONS RESUMED (2026-09-27) — Jev is waitlisted; playground script ready, needs `TYPESAFE_API_KEY` once off the waitlist

Detoured to see a real Jev call via the Python SDK: `~/Sites/Trading/PythonProjects/jev-playground/first_jev.py`.
Resume at **Lesson 7 — `Vec` growth and reference invalidation**. Plan: later call Jev directly over HTTP from Rust behind an `Interpreter` trait (lesson 24 design).

---

## Lesson 7 — `Vec` growth and reference invalidation ✅ DONE

### 🧸 Part A — The mystery box
- A **shelf** of boxes in a row, with some empty spaces at the end.
- Full shelf + new box → Mum moves **every** box to a bigger shelf in another room.
- Anyone pointing at "the first box on the old shelf" is now pointing at an empty wall.
- So while anyone is **looking** at a box on the shelf, Mum won't let anyone **add** to it — she doesn't check for space, she just says no.
- Fix: finish looking first, or write down what you saw ("it says 100") and stop pointing.

### 📈 Part B — The market
| Box world | Market world |
|---|---|
| The shelf | `Vec<i64>` of prices |
| Empty spaces at the end | spare **capacity** |
| Boxes on it | **length** |
| Moving to a bigger shelf | reallocation on `push` |
| Pointing at a box | `let first = &prices[0];` |
| Adding a box | `prices.push(101)` — needs `&mut prices` |
| Writing down what you saw | `let first = prices[0];` — copies the `i64` |

TS: arrays grow invisibly and references stay valid (GC). Rust `Vec` stores values inline, so growth can move them.

**New syntax (one at a time)**
1. `let mut prices = vec![100];` — a `Vec` (TS array) with one item. `vec!` is a macro.
2. `prices.push(101);` — add to the end (TS `push`). Needs `mut`.
3. `prices[0]` — first item (TS `prices[0]`).
4. `prices.len()` / `prices.capacity()` — boxes on the shelf / total spaces.

**Task** — `examples/l07_vec.rs`
1. Make `prices` = `vec![100]`, print `len` and `capacity`.
2. Break it: `let first = &prices[0]; prices.push(101); println!("first={}", first);` → predict + `cargo check`.
3. Fix it two ways: (a) copy — `let first = prices[0];` (no `&`); (b) reorder — push first, then borrow.
4. Push a few more prices, print `len` and `capacity` again. Capacity values may vary — never rely on them.

**Target:** prints `first=100` and a larger final length.

**Checkpoint:** Why doesn't spare capacity make the original borrow + push legal?

**Lesson 7 takeaways**
- `let first = &prices[0]; prices.push(..); use(first)` → **E0502 cannot borrow as mutable because it is also borrowed as immutable**. `push` needs `&mut prices` behind the scenes.
- Fix 1: `let first = prices[0];` — **copies** the `i64` (original stays in the Vec); nothing points at the shelf.
- For `Vec<String>`, `let s = names[0];` is an error — can't copy a `String`, and moving it out of the Vec isn't allowed. Use `&names[0]` or `names[0].clone()`.
- Copy isn't about stack vs heap (Vec items are all on the heap) — it's whether the value is **self-contained** (i64) or **owns something else** (String = claim ticket).
- Fix 2: reorder — finish using the look before the next `push`.
- Observed: `len=4 capacity=4` → push → `len=5 capacity=8` (the shelf moved). Never rely on exact capacity numbers.
- **Checkpoint:** spare capacity doesn't help because the borrow checker runs at **compile time** and only sees *kinds* of access (look in use + change), never runtime numbers like capacity. If it compiles, it's safe on every run.

---

## Lesson 8 — Slices and views ✅ DONE

### 🧸 Part A — The mystery box
- A **slice** = a **picture frame** held over part of the shelf (e.g. boxes 2–3).
- The frame doesn't own the boxes; it can't add/remove boxes.
- A **fixing frame** can change the boxes it shows — still can't add any.
- Works on any shelf — fixed-size or growing.

### 📈 Part B — The market
| Box world | Market world |
|---|---|
| Shelf | `Vec<i64>` or fixed array `[i64; 3]` |
| Frame (look) | `&[i64]` — slice |
| Fixing frame | `&mut [i64]` — change items, not length |
| Frame over boxes 2–3 | `&prices[1..]` |

TS: `prices.slice(1)` **copies** into a new array. A Rust slice copies nothing — it's a view (a borrow).

**New syntax**
1. `let prices = [100, 101, 103];` — fixed array (can't grow).
2. `&[i64]` — slice type.
3. Ranges: `&prices[..]` all · `&prices[1..]` from 1 to end · `&prices[..2]` up to (not incl.) 2.
4. `{:?}` prints a whole list: `println!("{:?}", prices);` → `[100, 101, 103]`.

**Task** — `examples/l08_slices.rs`
1. `fn sum_prices(prices: &[i64]) -> i64` — total starts 0, `for p in prices`, add, return. Print `all=304 tail=204 empty=0` using `&prices[..]`, `&prices[1..]`, `&[]`.
2. `fn increment(prices: &mut [i64])` — add 1 to each; call on `&mut prices[..2]` → print `changed=[101, 102, 103]`.

**Checkpoint:** Why can the same summing function accept both arrays and vectors through slices?

**Lesson 8 — the big insight (from fighting `from`)**
- `&prices[1..]` does **not** put `[101, 102]` into `from`. It stores an **address + length** (a "fat pointer", 16 bytes) pointing at the original items. `&` always means **point at**, never copy. The `&mut` permission exists only at compile time.
- TS `prices.slice(1)` = **copy** → Rust `prices[1..].to_vec()` (independent list; no borrow errors; changes don't flow either way).
- TS `int32arr.subarray(1)` = **view** → Rust `&prices[1..]` / `&mut prices[1..]`.
- Why a long-lived `from` borrow fails: with a `Vec`, `push` can move the items and free the old memory → `from` would point at freed memory (C/C++ "use-after-free / iterator invalidation"). Rust's one uniform rule — no change while a borrow is still in use — makes that impossible, for arrays too.
- A fixing frame (`&mut`) can also look (print, sum) — but while it's in use, work **only** through it; don't touch `prices` directly.
- **Idiomatic habit: store positions (or owners), not borrows; borrow at the moment of use.** `let from = 1;` then `&prices[from..]` / `&mut prices[from..]` wherever needed. (Same idea as ECS entity IDs later.)
- TS-style shared mutable data exists (`Cell`, `RefCell`, `Mutex`) — use deliberately, not by default.

### Lesson 8 — summary (the confusing bits, untangled)

**1. Every value is either an OWNER or a POINTER (a borrow).**
| You write | `from` is | TS equivalent |
|---|---|---|
| `prices[1..].to_vec()` | **owner** of a new copy `[101, 102]` | `prices.slice(1)` |
| `&prices[1..]` | **pointer** (look) at the originals | `int32arr.subarray(1)` |
| `&mut prices[1..]` | **pointer** (look + change) at the originals | `subarray`, writable |
| `&prices[1..].to_vec()` | pointer at an unnamed copy — **almost never what you want** | — |

**2. `&` always means "point at", never "copy".** To copy: `.to_vec()` / `.clone()` / plain `x = arr[i]` for numbers.

**3. The types to know**
| Type | Meaning |
|---|---|
| `i64` / `&i64` | one number / a look at one number |
| `[i64; 3]` | owned array of exactly 3 |
| `Vec<i64>` | owned growable list |
| `&[i64]` | **slice**: a look at any run of i64s (array, Vec, part, or empty) |
| `&mut [i64]` | a changing look — can change items, not the length |
| `[i64]` alone | unusable as a parameter — unknown size; slices only live behind `&` |

**4. Functions: ask for `&[i64]`, and callers lend with `&`.**
`sum_prices(&prices)` (array) · `sum_prices(&from)` (Vec) · `sum_prices(&prices[1..])` (part) · `sum_prices(&[])` (empty) — all the same function. That's the checkpoint answer: arrays and Vecs both turn into slices automatically.

**5. Owners lend with `&`; pointers pass as they are.**
If `from` is an owner (`Vec`) → `sum_prices(&from)`. If `from` is already a pointer → `sum_prices(from)`.

**6. A borrow can't stay in use across a change to what it points at.**
Why: with a Vec, `push` can move the items and free the old memory → the pointer would read freed memory (C/C++ use-after-free). Rust forbids it for all types, by one simple rule.

**7. How to reuse "a part of a list" (pick one)**
- **Position** — `let from = 1;` then `&prices[from..]` / `&mut prices[from..]` whenever needed. Most idiomatic.
- **Copy** — `let mut from = prices[1..].to_vec();` — independent; changes don't affect `prices`.
- **Short group** — `let from = &mut prices[1..];` then use only `from` until you're done with it.

**8. Syntax picked up:** `sum++` doesn't exist → `sum += 1`; `for i in 0..prices.len()`; `{:?}` prints lists; ranges `..`, `1..`, `..2`.

**The `&` exactly-once rule:** a function asking for `&[i64]` wants a pointer (a note saying where the boxes are). If you own the data (`from = prices[1..].to_vec()`), *make* the note at the call: `sum_prices(&from)`. If `from` is already a pointer (`from = &prices[1..]`), just hand it over: `sum_prices(from)`. The `&` appears once between the data and the function — at creation or at the call.

---

## Lesson 9 — `Option`, `Result` and `?` ✅ DONE

### 🧸 Part A — The mystery box
- **`Option`** = a box that might be empty. Mum makes you check before reaching in.
- **`Result`** = a parcel delivery: either the parcel (success) or a courier's note (why it failed).
- **`?`** = "pass the note up": if you get a note instead of a parcel, stop and take the note straight to whoever asked you; if it's the parcel, open it and carry on.
- **Empty ≠ failed.** Nothing-yet is normal; a note means something went wrong.

### 📈 Part B — The market
| Box world | Rust | TypeScript |
|---|---|---|
| Teddy / empty | `Option<i64>` — `Some(103)` / `None` | `number \| undefined` |
| Parcel / note | `Result<i64, E>` — `Ok(103)` / `Err(e)` | return / `throw` |
| Pass the note up | `?` | `if (err) return err;` |

Both are plain enums: `enum Option<T> { Some(T), None }`, `enum Result<T, E> { Ok(T), Err(E) }`. `<T>` = generics (like TS). Handle with `match`; exhaustiveness forces you to deal with `None` / `Err`.

Market: empty buffer = "no price yet" → `Option`. `"oops"` as a price = failure → `Result`.

**Task** — `examples/l09_results.rs`
1. `fn last_price(prices: &[i64]) -> Option<i64>` — `prices.is_empty()` → `None`, else `Some(prices[prices.len() - 1])`. `if c { None } else { Some(v) }` as last expression. Print `{:?}` of `last_price(&[])` and `last_price(&[100, 101, 103])`.
2. `Result`: parse `"103"` and `"oops"` into `i64`; `match` Ok/Err → `price=103`, `parse failed`.
3. `?`: a helper that uses `?` to pass a parse failure up; handle it in `main`.
4. Tests for empty buffer, `"103"`, `"oops"`.

**Target output:** `empty=None`, `price=103`, `parse failed`.

**Checkpoint:** What does `?` return from, and why is an empty buffer not necessarily an error?

**Lesson 9 takeaways**
- `-> Option<i64>` returns `Some(101)` / `None`, never a bare number — `{:?}` shows the box. Get the number out with `match` (`Some(price) => ...`, `None => ...`).
- Hard-coded `prices[2]` panics ("index out of bounds") on shorter lists → use `prices[prices.len() - 1]` after checking `is_empty()`.
- `&str` = a look at some text (like `&[i64]` for numbers). `text.parse::<i64>()` → `Result<i64, ParseIntError>`.
- `Err(_)` — `_` = "ignore the details".
- `?` = the 4-line `match { Ok(p) => p, Err(e) => return Err(e) }` in one character. Only works inside a function that returns `Result`. Success must be wrapped: `Ok(price + 1)`.
- Tests: `use super::*;` lets `mod tests` see the functions above. `assert!(x)` for true/false, `assert_eq!(a, b)` to compare, `.is_err()` checks for a failure. Tests run in parallel → output order varies.
- **Checkpoint:** `?` returns from the function it's written in, handing the `Err` to its caller. Empty = normal/expected ("no prices yet") → `Option`; tried-and-failed-with-a-reason → `Result`.

---

## Lesson 10 — Iterators (`iter`, `iter_mut`, `into_iter`) ✅ DONE

### 🧸 Part A — The mystery box
| Walk | At each box | Shelf afterwards |
|---|---|---|
| `iter()` | **point at** it, look | unchanged, still yours |
| `iter_mut()` | reach in and **fix** it | same boxes, changed contents |
| `into_iter()` | **take it off** and hand it on | **gone** |

**Lazy:** "walk along and pick the big ones" written on a note does nothing — it's a plan until someone sums / collects / loops.

### 📈 Part B — The market
| Walk | Each item | Earlier lesson |
|---|---|---|
| `prices.iter()` | `&i64` | L5 look |
| `prices.iter_mut()` | `&mut i64` | L6 change |
| `prices.into_iter()` | `i64` owned — Vec is **moved** | L3 move |

TS array methods are **eager** (build arrays immediately). Rust iterators are **lazy** (like generators) until `.sum()` / `.collect()` / `for`.

**Task** — `examples/l10_iterators.rs` (prices = `vec![100, 101, 103]`)
1. `let sum: i64 = prices.iter().sum();` → `sum=304`.
2. `iter_mut()` + `*p += 1` → `updated=[101, 102, 104]`.
3. `into_iter().filter(...).collect()` → `selected=[102, 104]`; try using `prices` afterwards → error → repair.

**Checkpoint answers**
1. Item types: `iter()` yields shared references (`&i64`), `iter_mut()` yields mutable references (`&mut i64`), and `into_iter()` yields owned values (`i64`).
2. Final move: `into_iter()` consumes/dismantles the original vector, transferring ownership of the elements. It avoids cloning allocations when transforming a collection, but means the original vector can no longer be used. To keep the original vector, borrow with `.iter().copied()` or explicitly `.clone()`.

**Takeaways**
- `filter(|p| *p > 101)`: `filter` passes items by reference to avoid consuming them during the test, so `p` is a pointer and `*p` reads the value.
- Lazy pipelines do nothing until consumed by `.sum()`, `.collect()`, or a `for` loop.
- `for p in &mut prices` is shorthand for `(&mut prices).into_iter()`, which calls `iter_mut()`.

---

## Lesson 11 — Traits and generics ✅ DONE

### 🧸 Part A — The mystery box
- **Trait** = a job description or skill badge (e.g. "Can make a trade decision").
- Any toy or box can earn the badge if it implements the required actions.
- **Generics with a trait bound** (`<S: Strategy>`): "I don't care what type you are, as long as you're wearing the `Strategy` badge."
- **Static dispatch (monomorphization):** The compiler generates a custom copy of the function for each concrete type that calls it. Zero runtime overhead, no vtable lookup.

### 📈 Part B — The market
| Concept | Rust | TypeScript / OOP |
|---|---|---|
| Interface / Contract | `trait Strategy { fn decide(&self, c: &Candle) -> Decision; }` | `interface Strategy { decide(c: Candle): Decision; }` |
| Implementation | `impl Strategy for PositiveClose { ... }` | `class PositiveClose implements Strategy { ... }` |
| Generic constraint | `fn evaluate<S: Strategy>(s: &S, c: &Candle) -> Decision` | `<T extends Strategy>(s: T, c: Candle): Decision` |

**Task** — `examples/l11_traits.rs`
1. Define `Candle` struct: `open: i64`, `close: i64`.
2. Define `Decision` enum: `Watch`, `Wait`. Derive `#[derive(Debug, PartialEq)]`.
3. Define `trait Strategy`:
   ```rust
   trait Strategy {
       fn decide(&self, candle: &Candle) -> Decision;
   }
   ```
4. Define two zero-sized structs: `struct PositiveClose;` and `struct AlwaysWait;`.
5. Implement `Strategy` for both:
   - `PositiveClose`: if `candle.close > candle.open`, return `Decision::Watch`, else `Decision::Wait`.
   - `AlwaysWait`: always return `Decision::Wait`.
6. Write generic function:
   ```rust
   fn evaluate<S: Strategy>(strategy: &S, candle: &Candle) -> Decision {
       strategy.decide(candle)
   }
   ```
7. In `main`:
   - Create a candle with `open: 100, close: 103`.
   - Evaluate with `&PositiveClose` and `&AlwaysWait`.
   - Print `PositiveClose={:?}` and `AlwaysWait={:?}`.
8. Add tests verifying both behaviors.

**Target output:**
```
PositiveClose=Watch
AlwaysWait=Wait
```

**Checkpoint answers**
1. What `evaluate` can assume about `S`: That any type passed as `S` implements the `Strategy` trait, guaranteeing it has a `decide(&self, &Candle) -> Decision` method.
2. What determines which implementation runs: The concrete struct type passed at the call site (e.g., `PositiveClose` vs `AlwaysWait`), resolved statically at compile time through monomorphization.

**Takeaways**
- Traits define capabilities/contracts; `impl Trait for Type` attaches the behavior to data.
- Unit structs (`struct PositiveClose;`) have no fields (0 bytes) and are useful for behavioral strategies.
- Monomorphization: generic functions are duplicated at compile time for each concrete type, giving high performance with zero runtime dynamic dispatch overhead.

---

## Lesson 12 — Practical lifetimes (`'a`) ✅ DONE

### 🧸 Part A — The mystery box
- A lifetime is just a **sticky note with an expiry date**.
- You can't write an expiry date later than the shelf it sits on.
- Putting a sticky note on a box does not stop Mum from binning it when the scope ends.

### 📈 Part B — The market
| Scenario | Rust signature | Lifetime handling |
|---|---|---|
| One input reference, one output | `fn latest(prices: &[i64]) -> Option<&i64>` | **Elision** (automatic; no annotations needed) |
| Two input references, one output | `fn higher_close<'a>(a: &'a Candle, b: &'a Candle) -> &'a Candle` | **Explicit `'a`** (output tied to the shorter-lived input) |
| Returning a reference to local data | `fn invalid() -> &Candle` | **Illegal!** Local data dropped at function exit. Return owned `Candle` instead. |

**Task** — `examples/l12_lifetimes.rs`
1. Define `#[derive(Debug)] struct Candle { open: i64, close: i64 }`.
2. Write `fn latest(prices: &[i64]) -> Option<&i64>` using lifetime elision (no `'a`).
   - If `prices.is_empty()`, return `None`, else `Some(&prices[prices.len() - 1])`.
3. Write `fn higher_close<'a>(a: &'a Candle, b: &'a Candle) -> &'a Candle`:
   - If `a.close >= b.close`, return `a`, else `b`.
4. Deliberate error test: try writing a function that creates a local candle and returns a reference to it:
   ```rust
   // fn invalid_local() -> &Candle {
   //     let c = Candle { open: 100, close: 105 };
   //     &c
   // }
   ```
   Inspect compiler error, then remove/comment it out.
5. In `main`:
   - `let prices = vec![100, 101, 103];`
   - `let c1 = Candle { open: 100, close: 105 };`
   - `let c2 = Candle { open: 100, close: 102 };`
   - Print `latest={}` and `higher={}`.
6. Verify output.

**Target output:**
```
latest=103
higher=105
```

**Checkpoint answers**
1. Why `'a` does not keep a local candle alive: Lifetime annotations are descriptive, not prescriptive. They do not allocate memory or change when drop occurs. A local variable created on the stack is deallocated when its enclosing scope exits, so returning a reference to it is always an illegal dangling pointer.
2. What restricts how long the returned borrow can be used: The lifetime `'a` in the signature connects the output to the inputs. The returned borrow can only be used as long as the *shortest-lived* input remains alive in the caller's scope.

**Takeaways**
- Lifetime elision: 1 input reference -> 1 output reference doesn't need annotations; Rust automatically infers the output borrows from the input.
- Multiple input references returning a reference require explicit annotations (e.g. `'a`) so the compiler knows which input lifetimes bound the output.
- "Start with ownership, borrow only when needed": Prefer returning owned values (`Copy`/`Clone`) rather than borrows from functions to avoid lifetimes entirely unless zero-copy performance is strictly required.

---

## Lesson 13 — `Box`, `Arc` and `Mutex` ✅ DONE

### 🧸 Part A — The mystery box
- **`Box<T>`**: Putting a toy in a secure storage locker on the heap. You still have one owner, just stored elsewhere.
- **`Arc<T>`**: Multiple kids holding claim tickets to the *same* toy locker. When a kid leaves, their ticket is torn up; when the last ticket is torn up, the locker is emptied.
- **`Mutex<T>`**: A keycard scanner on the locker door. Only one kid can enter at a time. To change the toy inside, you must hold the keycard (`lock()`).
- **`Arc<Mutex<T>>`**: Shared ownership (`Arc`) + exclusive mutation (`Mutex`).

### 📈 Part B — The market
| Tool | Meaning | Use case |
|---|---|---|
| `Box<T>` | Unique heap allocation | Fixed-size pointer to heap data |
| `Arc<T>` | Atomic Reference Counted pointer | Read-only shared ownership across threads |
| `Mutex<T>` | Mutual exclusion lock | Safe interior mutability across threads |
| `Arc<Mutex<T>>` | Shared ownership + safe mutation | State shared and updated across threads/tasks |

**Task** — `examples/l13_shared.rs`
1. Define `struct Candle { open: i64, close: i64 }`.
2. Heap-box a candle: `let boxed_candle = Box::new(Candle { open: 100, close: 103 });` and print `boxed close={}`.
3. Share a counter across threads:
   - `let counter = Arc::new(Mutex::new(0u64));`
   - Clone the Arc for thread 1 and thread 2.
   - Spawn thread 1: acquire `lock()`, increment counter (`*guard += 1`).
   - Spawn thread 2: acquire `lock()`, increment counter (`*guard += 1`).
   - Join both threads in `main`.
   - Inspect and print `processed={}`.

**Target output:**
```
boxed close=103
processed=2
```

**Checkpoint answers**
1. Cloning `Arc`: Does not duplicate the counter. It only increments an atomic reference count on the heap. Both clones point to the exact same memory.
2. Roles: `Arc` manages the shared heap storage and lifetime (dropping when count reaches 0); `Mutex` protects the inner data from concurrent mutation by enforcing mutual exclusion.

**Takeaways**
- `Arc<T>` gives shared ownership across threads, but is read-only.
- `Mutex<T>` allows interior mutability, but can only have one owner.
- Layering them as `Arc<Mutex<T>>` allows multiple threads to safely share and mutate state.
- `move || { ... }` in thread closures moves ownership of thread handles/variables into the spawned thread.
- `lock()` returns a `MutexGuard` that unlocks automatically when dropped (RAII).

---

## Lesson 14 — `Send` and `Sync` ✅ DONE

### 🧸 Part A — The mystery box
- **`Send`**: A toy that is safe to **mail** to another kid in a different house (thread).
- **`Sync`**: A toy in a display case that is safe for two kids in different houses to **look at at the same time** (`&T` is `Send`).
- **`Rc` vs `Arc`**: `Rc` uses a cheap non-atomic counter. Mailing `Rc` to another thread is forbidden because two threads updating the count at once would corrupt it. `Arc` uses atomic CPU instructions, so it earns the `Send` badge.

### 📈 Part B — The market
| Trait | Meaning | Rule of thumb |
|---|---|---|
| `Send` | Safe to transfer ownership to another thread | Most types are `Send`. Not `Send`: raw pointers, `Rc`. |
| `Sync` | Safe to share references (`&T`) between threads | `T` is `Sync` if `&T` is `Send`. `Mutex<T>` makes `T` `Sync`. `RefCell` is not `Sync`. |

**Task** — `examples/l14_send_sync.rs`
1. Define `MarketEvent::Tick { symbol: String, price: i64 }`.
2. Write compile-time type assertion helpers:
   ```rust
   fn assert_send<T: Send>() {}
   fn assert_sync<T: Sync>() {}
   ```
3. In `main`:
   - Assert `assert_send::<MarketEvent>();`
   - Assert `assert_send::<Arc<Mutex<u64>>>();`
   - Assert `assert_sync::<Arc<Mutex<u64>>>();`
4. Deliberate error test: try moving an `Rc<String>` into a `thread::spawn`:
   - Observe why the compiler blocks it (`Rc` does not implement `Send`).
   - Replace with `Arc<String>`.
5. Spawn a worker thread that receives an owned `MarketEvent` and prints `worker saw SILVER`.
6. Run and verify output.

**Target output:**
```
worker saw SILVER
```

**Checkpoint answers**
- Moving an owner across threads (`Send`): The sending thread gives up all access; ownership moves entirely to the receiving thread. Because only one thread has access at any given time, no locks or synchronization are needed to prevent data races.
- Sharing a reference across threads (`Sync`): Multiple threads hold pointers (`&T`) to the same memory concurrently. To mutate shared data safely across threads, synchronization (e.g. `Mutex<T>`) is required.

**Takeaways**
- `Send` and `Sync` are auto marker traits: if all fields of a struct are `Send`/`Sync`, the struct automatically implements them.
- `Rc` is not `Send` because its non-atomic counter would corrupt across threads; use `Arc` instead.
- `RefCell` is not `Sync` because its borrow check is not thread-safe; use `Mutex` or `RwLock` instead.

---

## Lesson 15 — Channels and ownership transfer ✅ DONE

### 🧸 Part A — The mystery box
- **Channel**: A pneumatic mail tube between rooms.
- Put a box in the tube (`send()`) $\rightarrow$ it vanishes from your hands (moved!).
- Take it out at the other end (`recv()`).
- **Bounded capacity (e.g. 2)**: The tube can only hold 2 boxes at once. If you try to push a 3rd box, the tube blocks you until someone empties a box from the other end (backpressure).
- **Closing**: When the sender drops their end of the tube, `recv()` returns `Err` (meaning the stream is closed / no more messages).

### 📈 Part B — The market
| Tool | Meaning | Use case |
|---|---|---|
| `sync_channel(bound)` | Bounded synchronous channel (MPSC: Multi-Producer, Single-Consumer) | Market tick pipelines with backpressure |
| `tx.send(msg)` | Moves ownership of `msg` into the channel buffer | Producer sending ticks |
| `rx.iter()` / `rx.recv()` | Blocks waiting for next item; terminates when all `tx` are dropped | Consumer loop |

**Task** — `examples/l15_channels.rs`
1. Define `#[derive(Debug)] enum MarketEvent { Tick { symbol: String, price: i64 } }`.
2. Use `std::sync::mpsc::sync_channel(2)` to create a sender (`tx`) and receiver (`rx`).
3. Spawn a producer thread with `tx`:
   - Send three ticks: `100`, `101`, `103`.
   - The thread completes and `tx` drops automatically.
4. In `main`:
   - Receive events using `for event in rx { ... }` or `while let Ok(event) = rx.recv()`.
   - Count received items.
   - Join the producer thread.
   - Print `stream closed; received=3`.

**Target output:**
```
tick SILVER 100
tick SILVER 101
tick SILVER 103
stream closed; received=3
```

**Checkpoint answers**
- Why an extra live sender prevents termination: The receiver loop (`for event in rx`) only terminates when *all* clones of `tx` have been dropped. If an unused sender clone remains in scope, `rx` will wait forever for more messages.
- Why joining too early causes a deadlock: In a bounded channel (`sync_channel(2)`), sending more items than the capacity blocks the producer until the consumer reads. If the consumer calls `producer.join()` before reading, the consumer is waiting on the producer, and the producer is waiting on the consumer—a classic deadlock.

**Takeaways**
- Bounded channels apply natural backpressure, preventing fast producers from exhausting memory.
- `rx` implements `IntoIterator`, reading until all senders drop.
- Always drain bounded channels concurrently with the producer to avoid deadlocks.

---

## Lesson 16 — Async and Tokio ✅ DONE

### 🧸 Part A — The mystery box
- **`async` function / block**: Writing a recipe on a card. It does **not** cook the meal immediately; it produces a `Future`.
- **`.await`**: "Put the pot on to simmer and step away from the stove." While waiting, the chef works on other tables' orders (cooperative multitasking).
- **Tokio channel (`tokio::sync::mpsc`)**: Same mail tube as Lesson 15, but instead of blocking the entire OS thread, `send().await` and `recv().await` pause only the async task.
- **`tokio::time::timeout`**: An egg timer. If the recipe takes too long, abandon it and report a timeout.

### 📈 Part B — The market
| Tool | Meaning | Use case |
|---|---|---|
| `#[tokio::main]` | Initializes the multi-threaded async runtime | Entry point for async apps |
| `tokio::spawn(async move { ... })` | Spawns a lightweight green task | Background producers, WebSocket listeners |
| `tokio::sync::mpsc::channel(bound)` | Asynchronous bounded MPSC channel | Passing market events between async tasks |
| `tokio::time::sleep(duration).await` | Non-blocking async timer | Rate-limiting, heartbeat, polling |
| `tokio::time::timeout(duration, future)` | Timeout wrapper | Guarding network calls against hanging |

**Task** — `examples/l16_async.rs`
1. Define `#[derive(Debug)] enum MarketEvent { Tick { symbol: String, price: i64 } }`.
2. Bounded async channel: `let (tx, mut rx) = tokio::sync::mpsc::channel::<MarketEvent>(2);`.
3. Spawn an async producer task with `tokio::spawn`:
   - Sleep 10ms before each of three ticks (`100`, `101`, `103`).
   - Send each tick asynchronously (`tx.send(event).await.unwrap()`).
   - Task finishes and drops `tx`.
4. In main async task:
   - Wrap the receive loop in a generous timeout (`tokio::time::timeout(Duration::from_millis(500), ...)`).
   - Receive ticks with `while let Some(event) = rx.recv().await`.
   - Print `async tick=...`.
   - Await the producer task handle.
   - Print `async complete`.

**Target output:**
```
async tick=100
async tick=101
async tick=103
async complete
```

**Checkpoint answers**
- What runs during a sleep's `.await`: The current task yields its thread to the Tokio scheduler. The underlying OS thread is free to execute other pending async tasks or I/O events while the timer counts down.
- Difference between timeout, cancellation, and thread join:
  - `join()` (thread): Blocks an entire OS thread until a background thread terminates.
  - Cancellation (async): In Rust, futures are canceled simply by dropping them. If you stop polling a future, it ceases to execute immediately and runs its destructors.
  - Timeout: Drops and cancels a future if it does not complete before a specified duration expires.

**Takeaways**
- Async is cooperative: tasks yield at `.await` points.
- Never execute long-running blocking CPU work or hold standard OS mutex locks across an `.await`.
- Async channels (`tokio::sync::mpsc`) pause tasks instead of blocking OS threads.

---

## Lesson 17 — Bevy ECS bridge without a game ✅ DONE

### 🧸 Part A — The mystery box
- **ECS (Entity Component System)** is like an in-memory database:
  - **Entity**: A row ID / primary key (just a number like `Entity(0)`).
  - **Component**: Columns of data attached to that ID (e.g. `Activity(0)`, `Trader`).
  - **Resource**: A global singleton variable for the whole database (e.g. `MarketPulse(3)`).
  - **System**: A SQL query + function (e.g. `UPDATE entities SET activity = activity + pulse WHERE has(Trader)`).
  - **Schedule**: The scheduler that runs the systems in order.

### 📈 Part B — The market
| ECS Concept | Bevy Rust | Meaning |
|---|---|---|
| Entity | `Entity` ID | Unique identity for a market participant |
| Component | `#[derive(Component)]` | State attached to an entity (`Activity`, `Position`) |
| Resource | `#[derive(Resource)]` | Global world data (`MarketPulse`, `Clock`) |
| Query | `Query<&mut Activity, With<Trader>>` | Fetching matching components for systems |
| System | `fn update(pulse: Res<Pulse>, ...)` | Function operating on queried data |

**Task** — `examples/l17_ecs.rs`
1. Define components: `#[derive(Component)] struct Trader;` and `#[derive(Component)] struct Activity(u8);`.
2. Define resource: `#[derive(Resource)] struct MarketPulse(u8);`.
3. Write system:
   ```rust
   fn pulse_activity(pulse: Res<MarketPulse>, mut query: Query<&mut Activity, With<Trader>>) {
       for mut activity in &mut query {
           activity.0 += pulse.0;
       }
   }
   ```
4. In `main`:
   - Initialize `let mut world = World::new();`.
   - Insert resource `MarketPulse(3)`.
   - Spawn entity with `(Trader, Activity(0))`.
   - Create `Schedule::default()` and add system `pulse_activity`.
   - Run schedule once $\rightarrow$ print `activity=3`.
   - Run schedule second time $\rightarrow$ print `activity=6`.

**Target output:**
```
activity=3
activity=6
```

**Checkpoint answers**
- Identifying parts:
  - Entity: `trader_entity` (the unique ID returned by `world.spawn()`).
  - Components: `Trader` (marker component) and `Activity(u8)` (data component).
  - Resource: `MarketPulse(3)` (global singleton inserted into the world).
  - System: `pulse_activity` (the function that runs during schedule execution).
  - Query: `Query<&mut Activity, With<Trader>>` (the selector parameter asking for entities with specific components).
- Why overlapping mutable queries conflict: Rust's fundamental borrow checker rule says you cannot have two mutable references (`&mut`) to the same data simultaneously. If two queries in the same system could match the same entity's component mutably, Bevy catches it at runtime and panics to prevent data aliasing and race conditions. Disjoint filters (e.g. `Without<X>`) guarantee non-overlapping sets.

**Takeaways**
- ECS decouples state (Components) from behavior (Systems), running queries across dense arrays for cache locality.
- Headless Bevy ECS operates purely in memory with zero windowing or graphics overhead.
- Bevy systems declare their borrowing requirements (`Res`, `ResMut`, `&Component`, `&mut Component`), allowing the scheduler to parallelize systems automatically.

---

## Lesson 18 — System One versus System Two: where interpretation fits ✅ DONE

### 🧸 Part A — The mystery box
- **System One (Fast / Reflex)**: Instinctive pattern recognition. Given a picture, immediately say "cat" (or "high activity") with a probability and confidence. No chain-of-thought, no essay, just typed structured predictions.
- **System Two (Slow / Deliberative)**: Step-by-step reasoning, writing proofs, planning a vacation.
- **Deterministic Code (The Calculator)**: Pure Rust math (`103 - 100 = 3`). Never ask an AI model to subtract numbers; let Rust do math and let System One do perceptual categorization.

### 📈 Part B — The market
| Role | Mechanism | Example in JevFloor |
|---|---|---|
| Deterministic Features | Pure Rust calculation | `delta = candle.close - candle.open` (exact, instant) |
| System One Inference | Fast typed model (Jev) | `p_persistent: 0.85` (probabilistic pattern score) |
| System Two Deliberation | Multi-step reasoning | Macro analysis, strategy planning |
| Decision Policy | Rust logic | `if delta > 0 && p_persistent > 0.80 { Run }` |

**Task** — `examples/l18_system_one.rs`
1. Define typed output struct:
   ```rust
   #[derive(Debug)]
   struct Interpretation {
       p_persistent: f64,
       p_active: f64,
       confidence: f64,
   }
   ```
2. Write deterministic feature calculator:
   ```rust
   fn price_delta(open: i64, close: i64) -> i64 {
       close - open
   }
   ```
3. Write mock System One inference function:
   ```rust
   fn mock_system_one(scenario: &str) -> Interpretation {
       match scenario {
           "bullish" => Interpretation { p_persistent: 0.85, p_active: 0.90, confidence: 0.90 },
           _ => Interpretation { p_persistent: 0.35, p_active: 0.40, confidence: 0.70 },
       }
   }
   ```
4. In `main`:
   - Compute `delta = price_delta(100, 103)`.
   - Call `mock_system_one("bullish")`.
   - Print `delta=3`, `source=MOCK`, `p_persistent=0.85`.
   - (Change selector to `"bearish"` to observe `p_persistent=0.35`).

**Target output:**
```
delta=3 source=MOCK p_persistent=0.85
```

**Checkpoint answers**
- Which result is calculated: `delta = 3` (calculated purely deterministically by Rust arithmetic).
- Which is a stand-in for inference: `p_persistent = 0.85` in `Interpretation` (represents typed probabilistic model output).
- What the real provider replaces: A live model provider (e.g. TypeSafe AI / Jev) replaces the mock hardcoded return values with live inference calls returning structured JSON over HTTP/gRPC.

**Takeaways**
- Never delegate arithmetic or deterministic logic to AI models.
- System One provides fast, bounded, typed probabilities and confidence.
- Application logic combines deterministic metrics with probabilistic scores to make final decisions.

---

## Lesson 19 — Predefined schemas and typed probabilistic outputs ✅ DONE

### 🧸 Part A — The mystery box
- **The Wire vs The Validator**: Just because a box fits through the mail slot doesn't mean what's inside is allowed.
- A float (`f64`) can hold numbers like `1.5`, `-50.0`, or even `NaN` (garbage). A probability MUST strictly be between `0.0` and `1.0`.
- **Schema versioning**: If a delivery has an unknown schema badge (e.g. version 2 when we only know version 1), reject it immediately.

### 📈 Part B — The market
| Wire Value | Rust Type | Validation Outcome |
|---|---|---|
| `0.85` | `f64` | `Ok(())` — valid probability |
| `1.20` | `f64` | `Err(InvalidProbability)` — outside [0.0, 1.0] |
| `f64::NAN` | `f64` | `Err(NotANumber)` — invalid float |
| `version: 2` | `u32` | `Err(UnsupportedSchemaVersion)` — rejected |

**Task** — `examples/l19_schema.rs`
1. Define `Envelope` with `schema_version: u32`, `p_persistent: f64`, `p_active: f64`, `confidence: f64`.
2. Define `#[derive(Debug, PartialEq)] enum ValidationError { InvalidProbability, NotANumber, UnsupportedSchemaVersion }`.
3. Implement `fn validate(env: &Envelope) -> Result<(), ValidationError>`.
4. Test 4 fixtures:
   - Valid (`0.85`) $\rightarrow$ `valid accepted`
   - Out of range (`1.20`) $\rightarrow$ `out-of-range rejected`
   - NaN (`f64::NAN`) $\rightarrow$ `NaN rejected`
   - Wrong version (`99`) $\rightarrow$ `schema rejected`
5. Add unit tests for all 4 outcomes.

**Target output:**
```
valid accepted
out-of-range rejected
NaN rejected
schema rejected
```

**Checkpoint answers**
- Why a valid `f64` can be an invalid probability: An `f64` represents any 64-bit IEEE 754 floating-point number, including negative numbers, numbers greater than 1.0, infinity, and `NaN`. A probability, by definition, must be a finite number within the closed interval `[0.0, 1.0]`.
- Why accepting a schema does not establish truth: Schema validation only verifies structural and syntactical conformity (types, bounds, versions). It does not guarantee that the model's estimate is factually accurate, calibrated, or predictive of real-world market movements.

**Takeaways**
- Boundary validation is essential: parse into types, then validate business/domain invariants before passing data to core logic.
- Always check `.is_nan()` and range bounds for floating-point probabilities.
- Reject unsupported schema versions early to prevent silent misinterpretation.

---

## Lesson 20 — Probability, confidence and uncertainty ✅ DONE

### 🧸 Part A — The mystery box
- **Probability ($p$)**: "How likely is the event?" (e.g. 85% chance of persistent trend).
- **Confidence**: "How sure is the model about its own estimate?" (e.g. 90% confidence based on clean data vs 40% confidence on noisy/partial data).
- **A high probability with low confidence is NOT strong**: If someone says "There's an 85% chance it rains, but I'm only 40% confident because my radar is broken", you treat the prediction as **uncertain**.

### 📈 Part B — The market
| Scenario | Probability $p$ | Confidence | Policy Output | Reason |
|---|---|---|---|---|
| Case 1 | `0.85` | `0.90` | `strong` | High probability, high confidence |
| Case 2 | `0.85` | `0.40` | `uncertain` | High probability, but confidence < 0.60 |
| Case 3 | `0.50` | `0.90` | `mixed` | Confident, but probability < 0.80 |

**Task** — `examples/l20_uncertainty.rs`
1. Define `#[derive(Debug, PartialEq)] enum MarketStance { Strong, Mixed, Uncertain }`.
2. Write classification policy:
   ```rust
   fn classify(p: f64, confidence: f64) -> MarketStance {
       if confidence < 0.60 {
           MarketStance::Uncertain
       } else if p >= 0.80 {
           MarketStance::Strong
       } else {
           MarketStance::Mixed
       }
   }
   ```
3. In `main`:
   - Evaluate `(0.85, 0.90)` $\rightarrow$ print `strong`.
   - Evaluate `(0.85, 0.40)` $\rightarrow$ print `uncertain`.
   - Evaluate `(0.50, 0.90)` $\rightarrow$ print `mixed`.
4. Add unit tests for exact boundaries (`confidence == 0.60` and `p == 0.80`).

**Target output:**
```
strong
uncertain
mixed
```

**Checkpoint answers**
- Why the first two cases share an event estimate yet produce different labels: Both have $p = 0.85$, but Case 2 has low confidence ($0.40 < 0.60$). In our policy, low confidence acts as a gatekeeper: if the model is not confident in its inputs or internal state, the probability cannot be trusted, forcing the decision into `Uncertain`.
- What evidence calibration requires: Calibration requires tracking historical predictions against actual ground-truth outcomes across hundreds or thousands of instances (e.g., verifying that among all events assigned an 80% probability, approximately 80% of them actually occurred).

**Takeaways**
- Never evaluate probability in isolation; always gate on confidence first.
- Clear policy boundaries prevent garbage-in from triggering trading actions.

---

## Lesson 21 — Decompose interpretation into small questions ✅ DONE

### 🧸 Part A — The mystery box
- **The big question trap**: Asking "Should I buy?" is like asking an AI "Is this good?". It's an un-auditable black box.
- **Decomposed questions**: Instead, ask small, sharp, verifiable questions:
  1. "Is the trend persistent?" ($p_1$)
  2. "Is the market active?" ($p_2$)
- **Independent, not exclusive**: The answers do NOT sum to 1.0. Both can be 0.95, or both can be 0.05.

### 📈 Part B — The market
| Scenario | $p_\text{persistent}$ | $p_\text{active}$ | Threshold ($\ge 0.80$) | Label |
|---|---|---|---|---|
| Fixture 1 | `0.85` | `0.90` | Both $\ge 0.80$ | `both strong` |
| Fixture 2 | `0.85` | `0.30` | Only persistent $\ge 0.80$ | `persistence only` |
| Fixture 3 | `0.30` | `0.90` | Only active $\ge 0.80$ | `activity only` |

**Task** — `examples/l21_questions.rs`
1. Define a struct `DecomposedQuestions { p_persistent: f64, p_active: f64 }`.
2. Write a classifier using a `0.80` threshold returning `both strong`, `persistence only`, `activity only`, or `neither`.
3. In `main`: Evaluate and print the three fixtures.
4. Add a unit test demonstrating that the two probabilities do NOT need to sum to 1.0 (`0.85 + 0.90 = 1.75`).

**Target output:**
```
both strong
persistence only
activity only
```

**Checkpoint answers**
- Why outputs should not be summed: They are answers to distinct, non-mutually-exclusive questions (persistence vs activity), not outcomes of a single categorical random variable. A market can simultaneously be highly persistent and highly active.
- Why outputs should not be multiplied without justification: Multiplying probabilities assumes statistical independence ($P(A \land B) = P(A) \times P(B)$). In financial markets, persistence and activity are often correlated. Naive multiplication distorts the true joint probability.

**Takeaways**
- Decomposing complex decisions into orthogonal binary questions creates verifiable, auditable AI inputs.
- Distinct questions are evaluated against separate thresholds rather than combined into a single black-box score.

---

## Lesson 22 — Deterministic features and Rust control flow ✅ DONE

### 🧸 Part A — The mystery box
- **The Calculator & The Sensor**:
  - Rust is the calculator: it counts prices, calculates differences, and enforces the rules.
  - The AI model is a weather sensor: it reports probabilities and confidence.
  - The sensor does NOT drive the car. Rust reads the calculator + the sensor and decides whether to `Hesitate`, `Walk`, or `Run`.

### 📈 Part B — The market
| Inputs | Policy Branch | Result |
|---|---|---|
| Confidence < 0.60 | Low confidence gate | `Hesitate` |
| Delta > 0, both $p \ge 0.80$, Conf $\ge 0.60$ | Strong trend & activity | `Run` |
| Delta > 0, Conf $\ge 0.60$ (mixed $p$) | Positive trend only | `Walk` |
| Delta $\le 0$ | Non-positive price action | `Hesitate` |

**Task** — `examples/l22_policy.rs`
1. Calculate from `prices = [100, 101, 103]`:
   - `delta = last - first` (3)
   - `positive_changes = count of prices[i] > prices[i-1]` (2)
2. Define `#[derive(Debug, PartialEq)] enum VisualBehaviour { Hesitate, Walk, Run }`.
3. Implement the policy function.
4. Evaluate 4 cases: `strong`, `mixed`, `uncertain`, `nonpositive`.
5. Add unit tests for all 4 branches.

**Target output:**
```
delta=3 positive_changes=2
strong=Run mixed=Walk uncertain=Hesitate nonpositive=Hesitate
```

**Checkpoint answers**
- Which policy remains deterministic if the provider changes: The entire policy (`decide_behaviour`) and the feature calculation (`delta`, `positive_changes`) remain strictly deterministic in Rust. Given identical inputs, the outcome is guaranteed to be identical regardless of whether the model provider changes or is mocked.
- Why a model output does not authorize a trade: Model outputs are statistical estimates with uncertainty and confidence bounds, not legal or business commitments. The Rust application governor owns risk controls, portfolio constraints, circuit breakers, and final execution authority.

**Takeaways**
- Separate deterministic metric extraction from probabilistic model interpretation.
- Gate risky decisions behind strict confidence checks and deterministic conditions (`delta > 0`).
- Ensure safe default behavior (`Hesitate`) on missing, uncertain, or negative signals.

---

## Lesson 23 — Shared versus per-agent inference; latency and cost ✅ DONE

### 🧸 Part A — The mystery box
- **The Group Order**: If 3 kids all want the exact same pizza, order 1 large pizza and share it (1 call, cheap).
- **The Wrong Order**: If Kid 3 is allergic to gluten (different context), you CANNOT just give them a slice of Kid 1's pizza because "it's food". You must place a 2nd order.
- **Cache Key**: A cache key is not just `symbol`. It must be `(snapshot_id, schema_version, agent_context)`.

### 📈 Part B — The market
| Scenario | Agent 1 Context | Agent 2 Context | Agent 3 Context | Inference Calls | Cost (2 units/call) |
|---|---|---|---|---|---|
| Per-agent (no sharing) | Context A | Context A | Context A | 3 | 6 |
| Shared (same context) | Context A | Context A | Context A | 1 | 2 |
| Different context | Context A | Context A | Context B | 2 | 4 |

**Task** — `examples/l23_sharing.rs`
1. Define a mock provider that tracks call count and calculates cost (`calls * 2`).
2. Simulate:
   - 3 independent calls $\rightarrow$ `per-agent calls=3 cost=6`
   - 1 shared call across 3 consumers $\rightarrow$ `shared calls=1 cost=2`
   - 2 calls for differing contexts $\rightarrow$ `different-context calls=2 cost=4`

**Target output:**
```
per-agent calls=3 cost=6
shared calls=1 cost=2
different-context calls=2 cost=4
```

**Checkpoint answers**
- When reuse is semantically valid: Only when the inputs, schema version, snapshot ID, and agent question context are identical. Reusing results purely based on ticker symbol is a semantic bug.
- How concurrent calls affect latency vs cost: Running calls concurrently (e.g. `tokio::join!`) reduces wall-clock time from $N \times \text{latency}$ down to $\sim 1 \times \text{latency}$, but does NOT reduce request count or API financial costs. Caching/sharing eliminates both latency AND financial cost.

**Takeaways**
- Use `HashMap::entry().or_insert_with(...)` for clean memoization of inference results.
- Composite cache keys prevent false-sharing bugs across differing agent contexts.

---

## Lesson 24 — Provider-neutral boundary, failures and stale results ✅ DONE

### 🧸 Part A — The mystery box
- **Provider Neutrality**: Never weld your engine to one brand of spark plug. Use a trait (`Interpreter`) so changing AI vendors doesn't break a single line of trading logic.
- **The Stale Milk Trap**: The carton just arrived on your doorstep (response is new), but the milk was bottled 3 weeks ago (snapshot is old). Freshness is based on when the snapshot was observed, NOT when the packet arrived.
- **Fail-Safe**: If anything fails (timeout, network dropped, data too old), always drop back to `Hesitate`.

### 📈 Part B — The market
| Scenario | Observed Time | Current Time `now` | Age | Max Allowed Age | Outcome |
|---|---|---|---|---|---|
| Fresh | `900 ms` | `1000 ms` | `100 ms` | `200 ms` | `Ok(Envelope)` $\rightarrow$ `Run` |
| Exact Boundary | `800 ms` | `1000 ms` | `200 ms` | `200 ms` | `Ok(Envelope)` $\rightarrow$ `Run` |
| Stale | `750 ms` | `1000 ms` | `250 ms` | `200 ms` | `Err(StaleData)` $\rightarrow$ `Hesitate` |
| Provider Error | N/A | `1000 ms` | N/A | `200 ms` | `Err(ProviderError)` $\rightarrow$ `Hesitate` |

**Task** — `examples/l24_boundary.rs`
1. Define `Snapshot { id: u64, observed_at_ms: u64 }`.
2. Define `Envelope` and `InferenceError`.
3. Define `trait Interpreter`:
   ```rust
   trait Interpreter {
       fn interpret(&self, snapshot: &Snapshot) -> Result<Envelope, InferenceError>;
   }
   ```
4. Write freshness gate: given `now_ms: u64` and `max_age_ms: u64`, verify:
   - `snapshot.observed_at_ms <= now_ms` (reject future timestamps)
   - `now_ms - snapshot.observed_at_ms <= max_age_ms` (reject stale data)
5. Implement `SuccessProvider` and `ErrorProvider`.
6. Print:
   - `fresh=Run`
   - `stale=Hesitate`
   - `provider-error=Hesitate`
7. Add tests for fresh, exact boundary (`200ms`), stale (`>200ms`), and provider error.

**Target output:**
```
fresh=Run
stale=Hesitate
provider-error=Hesitate
```

**Checkpoint answers**
- Why a newly arrived response can already be stale: The market observation timestamp reflects when the market tick was recorded. If network latency, queuing, model batching, or inference processing took longer than the staleness threshold, the response is dead on arrival even if the HTTP packet just reached the socket.
- Which checks must be repeated before applying an asynchronous result:
  1. Freshness check against the current system time (`now - observed_at <= max_age`).
  2. Snapshot ID match (ensure market state hasn't moved past this snapshot or invalidated it).
  3. Schema version and payload integrity validation.

**Takeaways**
- Decouple application decision logic from model vendor objects via an `Interpreter` trait.
- Always validate freshness using observation time, not receipt time.
- Default to fail-safe behavior (`Hesitate`) on any validation, age, or transport failure.

---

## Lesson 25 — Replay and Determinism ✅ DONE

### 🧸 Part A — The mystery box
- **The Flight Recorder (Black Box)**: When air crash investigators analyze what happened, they don't put a new plane into the sky and hope the turbulence is identical. They play back the *exact* recorded cockpit audio and sensor readings.
- **Replay is NOT Re-running Inference**: AI models are stochastic black boxes. If you re-query an LLM during a backtest or post-mortem, you get slightly different probabilities and test a *different* universe. True replay feeds recorded inferences into pure Rust code. Zero model calls!
- **Policy Version Lock**: If your trading rules change to Version 2, but a historical log was stamped with Version 1 rules, the engine must either emulate Version 1 or refuse to evaluate it. Never silently apply new rules to old schemas without verification.

### 📈 Part B — The market
| Field in `ReplayRecord` | Purpose | Determinism Guarantee |
|---|---|---|
| `snapshot` | Ticker state and original observation time | Fixes market inputs |
| `features` | Precomputed deterministic metrics (`delta`, etc.) | Eliminates calculation drift |
| `interpretation` | Recorded probabilities from the model | Zero calls to stochastic provider (`provider_calls = 0`) |
| `logical_time` (`now_ms`) | Explicit simulated clock | No `SystemTime::now()` non-determinism |
| `schema_version` / `policy_version` | Schema & contract locks | Prevents silent mismatches |

**Task** — `examples/l25_replay.rs`
1. Define `ReplayRecord`:
   - `snapshot: Snapshot` (`id: u64`, `observed_at_ms: u64`)
   - `features: Features` (`delta: i64`, `positive_changes: usize`)
   - `interpretation: Option<InterpretationPayload>` (`p_persistent: f64`, `p_active: f64`, `confidence: f64`)
   - `source: String` (e.g. `"MOCK"`)
   - `now_ms: u64`
   - `schema_version: u32`
   - `policy_version: u32`
2. Define a provider call tracker (e.g., `AtomicUsize` or a mock struct with a counter) to prove the live model is NEVER called during replay.
3. Write `evaluate_replay(record: &ReplayRecord, current_policy_version: u32, max_age_ms: u64) -> Result<VisualBehaviour, ReplayError>`:
   - Check policy version: if `record.policy_version != current_policy_version`, return `Err(ReplayError::UnsupportedPolicyVersion(record.policy_version))`.
   - Check freshness: if `record.now_ms - record.snapshot.observed_at_ms > max_age_ms`, return `Ok(VisualBehaviour::Hesitate)`.
   - Apply policy:
     - If `interpretation` is `Some(interp)` and `features.delta > 0` and `interp.confidence >= 0.60`:
       - If `interp.p_persistent >= 0.80 && interp.p_active >= 0.80` $\rightarrow$ `VisualBehaviour::Run`
       - Otherwise $\rightarrow$ `VisualBehaviour::Walk`
     - Otherwise $\rightarrow$ `VisualBehaviour::Hesitate`
4. In `main`:
   - Construct a valid record (`observed_at_ms: 900, now_ms: 1000, max_age_ms: 200, policy_version: 1`).
   - Run replay evaluation twice. Verify both return `Run` and `provider_calls == 0`.
   - Construct a stale record (`observed_at_ms: 700, now_ms: 1000, max_age_ms: 200, policy_version: 1`).
   - Replay returns `Hesitate`.
   - Construct an unsupported policy record (`policy_version: 2` when current policy is 1).
   - Replay returns unsupported error.
5. Print:
   ```text
   replay1=Run replay2=Run provider_calls=0
   stale replay=Hesitate
   unsupported policy rejected
   ```
6. Add unit tests for determinism (replay 1 == replay 2), staleness, and policy version rejection.

**Checkpoint answers**
1. Why is rerunning inference not replay?
   Live model providers are stochastic (temperature, non-deterministic sampling, backend weight updates, provider API latency). Re-querying the model during a replay or backtest introduces randomness, testing an alternate reality rather than auditing or faithfully reproducing the original decision.
2. What must be preserved when a policy or schema changes?
   Explicit version stamps (`schema_version`, `policy_version`) and evaluation rules. A system must reject records stamped with unsupported policy versions or use version-specific dispatch logic rather than silently reinterpreting legacy records with modern rules.

**Takeaways**
- Replay is strictly an evaluation of *recorded* inputs and inferences through pure deterministic code.
- Always verify provider calls remain 0 during replays.
- Explicit schema and policy versions protect backtests and audits from silent semantic drift.

---

## Phase 4 — Apple Notes Classifier

A personal side project (classifying the author's own notes with Jev); not included in the public copy.

---

## Capstone Project — The Unified JevFloor Pipeline (`examples/capstone.rs`) 🎉 COMPLETE

### 🎯 Architecture Overview
The Capstone unites all 25 lessons into an end-to-end toy market decision pipeline:

```text
[Fake Market Tick Events]
       ↓ (Threaded MPSC Channel — L15)
[RollingBuffer: capacity 3, oldest evicted — L7]
       ↓ (Slice borrow &[f64] — L8)
[Deterministic Features: delta, positive_changes — L22]
       ↓ (Provider-neutral boundary — L24)
[System One Interpretation: p_persistent, p_active, confidence — L18, L19, L20, L21]
       ↓ (Freshness Gate + Schema Validation — L19, L24)
[Deterministic Policy Engine: Hesitate / Walk / Run — L20, L22]
       ↓ (Display Mapping: Wait / Observe / Investigate)
[Terminal Output + Deterministic Replay — L25]
```

---

### 📋 Full List of Tasks for `examples/capstone.rs`

#### Part 1: Event Channel & RollingBuffer (Lessons 7, 15) ✅ DONE
- [x] **Task 1.1 — MarketEvent:** Define `enum MarketEvent { Tick { symbol: String, price: f64 } }`.
- [x] **Task 1.2 — RollingBuffer:** Implement `RollingBuffer` struct with `capacity: usize` and `prices: Vec<f64>`.
  - `new(capacity: usize) -> Self`
  - `push(&mut self, price: f64)`: When full (`prices.len() == capacity`), evict oldest (`remove(0)`) before appending.
  - `as_slice(&self) -> &[f64]`: Expose an immutable slice view of the current buffer.
- [x] **Task 1.3 — Ingestion Channel:** Create bounded channel `sync_channel::<MarketEvent>(2)`. Spawns producer thread streaming curriculum ticks (`100.0, 101.0, 103.0`) by default, or live Silver bars from local cTrader bridge when `--live` is passed. Channel closes cleanly when producer finishes.
- [x] **Task 1.4 — Unit Tests:** `test_rolling_buffer_eviction`, `test_buffer_starts_empty`, and `test_synthetic_channel_ingestion`.

#### Part 2: Slices & Deterministic Feature Extraction (Lessons 8, 22) ✅ DONE
- [x] **Task 2.1 — Features Struct:** Define `struct Features { delta: f64, positive_changes: usize }`.
- [x] **Task 2.2 — Slice Calculation:** Implement `calculate_features(prices: &[f64]) -> Option<Features>`.
  - `delta = last - first` (e.g. `103.0 - 100.0 = 3.0`).
  - `positive_changes`: Count adjacent steps where `prices[i] > prices[i-1]`.
- [x] **Task 2.3 — Safe Handling:** Return `None` if buffer is empty or has fewer than 2 prices; never panic on insufficient history.
- [x] **Task 2.4 — Unit Tests:** Tests for normal feature calculation (`test_feature_arithmetic`), empty and single-element buffer safe handling (`test_empty_and_insufficient_features`), negative delta, flat prices, two elements, and buffer-to-features pipeline integration.

#### Part 3: Schema Validation & System One Interpretation (Lessons 18–21, 24) ✅ DONE
- [x] **Task 3.1 — Boundary Data Structures:**
  - `Snapshot { id: u64, observed_at_ms: u64 }`
  - `InterpretationPayload { p_persistent: f64, p_active: f64, confidence: f64 }`
  - `Envelope { schema_version: u32, snapshot_id: u64, interpretation: InterpretationPayload }`
  - `InferenceError { ProviderUnavailable, StaleData, InvalidSchema, ValidationFailed(String) }`
- [x] **Task 3.2 — Schema Validation:** Implement `validate_envelope(env: &Envelope, expected_snapshot_id: u64) -> Result<(), InferenceError>`.
  - Verify `schema_version == 1`.
  - Verify `snapshot_id == expected_snapshot_id`.
  - Verify all probabilities and confidence are finite (no `NaN`) and strictly within `[0.0, 1.0]`.
- [x] **Task 3.3 — Provider-Neutral Trait & Mock:**
  - `trait Interpreter { fn interpret(&self, snapshot: &Snapshot) -> Result<Envelope, InferenceError>; }`
  - Implement mock interpreter returning configurable fixtures.
- [x] **Task 3.4 — Unit Tests:** Tests rejecting `NaN`, out-of-bounds probabilities (`1.2`), wrong schema version, snapshot ID mismatch, and provider errors.

#### Part 4: Freshness Gate, Policy Engine & Strategy (Lessons 11, 20, 22, 24)
- [x] **Task 4.1 — Decision Types:**
  - `enum VisualBehaviour { Hesitate, Walk, Run }`
  - `enum DisplayDecision { Wait, Observe, Investigate }`
  - Display mapping: `Run -> Investigate`, `Walk -> Observe`, `Hesitate -> Wait`.
- [x] **Task 4.2 — Freshness Gate:** Verify `now_ms >= snapshot.observed_at_ms` (no future dates) and `now_ms - snapshot.observed_at_ms <= max_age_ms` (here: `200ms`).
- [x] **Task 4.3 — Policy Function:** Implement `decide_behaviour(features: Option<&Features>, envelope: Result<&Envelope, &InferenceError>, snapshot: &Snapshot, now_ms: u64, max_age_ms: u64) -> (VisualBehaviour, String)`.
  - Default fail-safe: any error, stale data, missing features, or `confidence < 0.60` -> `Hesitate`.
  - If features valid (`delta > 0`) and confidence $\ge 0.60$:
    - If `p_persistent >= 0.80 && p_active >= 0.80` -> `Run`.
    - Otherwise -> `Walk`.
  - Non-positive delta (`delta <= 0`) -> `Hesitate`.
- [x] **Task 4.4 — Strategy Trait:** Wrap decision logic in a lightweight `trait Strategy` consolidating Lesson 11.
- [x] **Task 4.5 — Unit Tests:** Tests for strong (`Run`), mixed (`Walk`), low-confidence (`Hesitate`), stale data (`Hesitate`), exact freshness boundary (`200ms`), and non-positive delta.

#### Part 5: Multi-Scenario CLI Runner
- [x] **Task 5.1 — Scenario Argument Parser:** Support command-line arguments: `cargo run --example capstone -- <MODE>`.
- [x] **Task 5.2 — Mode Matrix:**
  | Mode | Fixture Configuration | Expected Visual | Display Decision |
  |---|---|---|---|
  | `strong` | $p=(0.85, 0.90)$, conf=$0.90$, age=$100\text{ms}$ | `Run` | `Investigate` |
  | `mixed` | $p=(0.65, 0.90)$, conf=$0.90$, age=$100\text{ms}$ | `Walk` | `Observe` |
  | `uncertain` | $p=(0.85, 0.90)$, conf=$0.40$, age=$100\text{ms}$ | `Hesitate` | `Wait` |
  | `stale` | $p=(0.85, 0.90)$, conf=$0.90$, age=$301\text{ms}$ | `Hesitate` | `Wait` |
  | `error` | Provider returns error | `Hesitate` | `Wait` |
  | `empty` | 0 ticks supplied to buffer | `Hesitate` | `Wait` |
- [x] **Task 5.3 — Formatted Terminal Output:** Print source, symbol, snapshot ID, prices, delta, positive changes, probabilities, confidence, age, validity, decision, and visual behaviour.

#### Part 6: Deterministic Replay Engine (Lesson 25) ✅ DONE
- [x] **Task 6.1 — ReplayRecord Struct:** Bundle `snapshot`, `features`, recorded `interpretation`, `logical_time`, `schema_version`, and `policy_version`.
- [x] **Task 6.2 — Replay Evaluator:** Implement `evaluate_replay` that executes pure deterministic policy without calling the provider.
- [x] **Task 6.3 — Replay Verification:** Run recorded record twice in `main`, assert both return identical outputs, and assert provider call count remains strictly `0`.
- [x] **Task 6.4 — Unit Tests:** Tests for determinism (`replay1 == replay2`), stale replay, and unsupported policy version rejection.

#### Part 7: Configurable Policy Thresholds & Curriculum Exit Criteria (Lessons 11, 20, 22, 24, 25) ✅ DONE
- [x] **Task 7.1 — Configurable Policy Thresholds:** Implement `PolicyThresholds` struct with `Default` (`min_confidence: 0.60`, `min_p_persistent: 0.80`, `min_p_active: 0.80`) and `decide_behaviour_with_thresholds` allowing decoupled, parameterized policy evaluation.
- [x] **Task 7.2 — Unscripted Threshold Verification:** Implement `ConfigurableStrategy` implementing `Strategy` and verify threshold sensitivity:
  - Relaxing `min_confidence` to `0.35` turns `uncertain` scenario (`conf=0.40`) from `Hesitate` into `Run`.
  - Tightening `min_p_persistent` to `0.90` turns `strong` scenario (`p_persistent=0.85`) from `Run` into `Walk`.
- [x] **Task 7.3 — Curriculum Explain-Back Checkpoints:** Complete rigorous answers for all 4 curriculum explain-back checkpoints.
- [x] **Task 7.4 — Learning Day Exit Criteria:** All 55 automated tests passing, strictly zero orders/trading actions, end-to-end pipeline verified from ingestion to deterministic replay.

#### Part 8: Real End-to-End Jev Path (Task #6 Live Verification) ✅ DONE
- [x] **Task 8.1 — Real TypeSafe Jev Interpreter:** Implemented `RealJevInterpreter` posting to `https://api.typesafe.ai/v1/systemone` using `JEV_KEY` from `.env`.
- [x] **Task 8.2 — Opt-in CLI Flags:** Added `--jev real` and `--real-jev` flags alongside `--timeframe` and `--pair`, while preserving `MockInterpreter` as the offline default.
- [x] **Task 8.3 — Real Classification Decision Flow:** Real Jev scores flow through `features -> freshness -> decision -> visual`, showing `source=REAL` and model version `jev-1.13.0`.
- [x] **Task 8.4 — Fail-Fast Error Safety:** Fast exit with non-zero status on bad keys, connection refused, or malformed data; zero silent fallback to mock.
- [x] **Task 8.5 — Test Suite Expansion:** Added 7 unit tests (62 total passed) covering response parsing, malformed payloads, error status, and CLI parsing.

---

### 🧪 Complete Capstone Test Suite (62 Automated Tests)
1. `test_buffer_eviction` — Oldest item evicted when capacity exceeded.
2. `test_empty_and_insufficient_features` — Safe `None` return on `< 2` items without panic.
3. `test_feature_arithmetic` — Accurate `delta` and `positive_changes` calculation.
4. `test_policy_strong_branch` — Delta > 0, both probabilities $\ge 0.80$, confidence $\ge 0.60 \implies \text{Run}$.
5. `test_policy_mixed_branch` — Delta > 0, mixed probabilities, confidence $\ge 0.60 \implies \text{Walk}$.
6. `test_policy_low_confidence_gate` — High probability but confidence $< 0.60 \implies \text{Hesitate}$.
7. `test_freshness_exact_boundary` — Age == $200\text{ms}$ accepted as fresh.
8. `test_freshness_stale_rejection` — Age > $200\text{ms}$ rejected $\implies \text{Hesitate}$.
9. `test_snapshot_mismatch_rejection` — Mismatched snapshot ID rejected.
10. `test_schema_invalid_probabilities` — Probabilities $< 0.0$, $> 1.0$, or `NaN` rejected.
11. `test_provider_failure_fallback` — Provider error safely yields `Hesitate`.
12. `test_deterministic_replay_zero_calls` — Replay produces identical result with `provider_calls == 0`.

---

### 🎓 Explain-Back Checkpoints
1. Trace ownership of a `MarketEvent::Tick` from thread spawn to `buffer.push()`.
2. Why does `calculate_features` take `&[f64]` rather than `&RollingBuffer` or `Vec<f64>`?
3. Why must freshness be evaluated against `snapshot.observed_at_ms` rather than the packet arrival time?
4. Why is rerunning live inference fundamentally distinct from running deterministic replay?

**Checkpoint answers (Parts 1–7 Complete)**
1. **Tick Ownership Flow:**
   - In `spawn_tick_producer`, a new `MarketEvent::Tick` is constructed inside the worker closure. It owns its `symbol: String` and `price: f64`.
   - `tx.send(event)` moves ownership of the tick into the bounded channel queue.
   - `for event in rx` inside `main()` receives and takes ownership of each `MarketEvent::Tick`.
   - Pattern match `match event { MarketEvent::Tick { ref symbol, price } => ... }` borrows `symbol` for logging and copies `price` (`f64: Copy`).
   - `buffer.push(price)` copies the float into `buffer.prices: Vec<f64>`. When `event` goes out of scope at the end of the loop iteration, its owned `symbol: String` is cleanly dropped.
2. **Why `calculate_features` takes `&[f64]`:**
   - **Maximum Flexibility (Lesson 8):** A slice view `&[f64]` is a fat pointer (pointer + length). It can borrow directly from a `RollingBuffer` (`buffer.as_slice()`), a heap `Vec<f64>`, or a fixed array `[100.0, 101.0, 103.0]`.
   - **Decoupled Architecture:** `calculate_features` does not need to know or care how prices were collected or stored. It requires zero memory allocation, cannot modify buffer state, and does not depend on `RollingBuffer`'s internal data structure.
3. **Freshness against `snapshot.observed_at_ms` vs. Packet Arrival Time:**
   - **Physical Market Truth (Lessons 20 & 24):** `observed_at_ms` marks the timestamp when the market state was physically measured at the exchange/sensor. Packet arrival time only reflects when local client memory received the payload.
   - **Detecting Transport Latency & Queueing Delays:** If a network queue, WiFi lag, or OS context switch stalls packet transit by 500ms, the packet would appear "0ms old" relative to arrival time despite representing completely dead, stale market information. Measuring `now_ms - snapshot.observed_at_ms` guarantees the trading engine never acts on stale information regardless of latency source.
4. **Live Inference vs. Deterministic Replay:**
   - **Live Inference (Stochastic, Network-Bound, Paid):** Queries an external System One provider over network/IPC. It incurs latency, API consumption costs, and potential non-determinism across model updates.
   - **Deterministic Replay (Pure, Instant, Zero External Calls):** Feeds frozen, recorded payload structures (`ReplayRecord`) directly into pure deterministic Rust policy code (`evaluate_replay`). It guarantees $100\%$ byte-for-byte reproducible decisions offline, executes in sub-microsecond time, and asserts strictly zero provider calls (`calls_after - calls_before == 0`).

---

### 🔬 Part 2 Educational Deep Dive — Slices & Deterministic Features

#### 🧸 Part A — The mystery box
- **The Lens (`&[f64]`)**: You don't hand your whole heavy filing cabinet (`RollingBuffer`) to the accountant. You just let them look through a magnifying glass at the last few lines (`&[f64]`).
- **Two is the minimum**: You can't ask "did the temperature go up or down?" if you only have one thermometer reading. If you have fewer than 2 numbers, you say "I don't know yet" (`None`), you don't panic or pretend the change was zero.

#### 📈 Part B — The market
| Concept | Code | Lesson Origin |
|---|---|---|
| Slice view | `pub fn calculate_features(prices: &[f64]) -> Option<Features>` | Lesson 8 (Slices & views) |
| Insufficient history | `if prices.len() < 2 { return None; }` | Safe boundaries (no panic) |
| Total delta | `prices.last().unwrap() - prices.first().unwrap()` | Deterministic arithmetic |
| Directional changes | `prices.windows(2).filter(\|w\| w[1] > w[0]).count()` | Idiomatic window iterator |

#### 💻 Runnable Educational Examples
```rust
// Run with: cargo test --example capstone
use capstone::{calculate_features, Features, RollingBuffer};

// 1. Normal calculation over slice
let prices = [100.0, 101.0, 103.0];
let feats = calculate_features(&prices).unwrap();
assert_eq!(feats.delta, 3.0);
assert_eq!(feats.positive_changes, 2);

// 2. Safe None on insufficient history (never panics)
assert_eq!(calculate_features(&[]), None);
assert_eq!(calculate_features(&[100.0]), None);

// 3. Falling prices: negative delta, zero positive changes
let falling = [105.0, 102.0, 100.0];
let feats_falling = calculate_features(&falling).unwrap();
assert_eq!(feats_falling.delta, -5.0);
assert_eq!(feats_falling.positive_changes, 0);

// 4. Integrated with RollingBuffer: oldest item evicted before slice borrow
let mut buf = RollingBuffer::new(3);
buf.push(90.0);
buf.push(100.0);
buf.push(101.0);
buf.push(103.0); // 90.0 evicted!
let feats_buf = calculate_features(buf.as_slice()).unwrap();
assert_eq!(feats_buf.delta, 3.0);
assert_eq!(feats_buf.positive_changes, 2);
```

---

### 🛡️ Bridge Safety & Ingestion Contract (Pre-Part 3 Hardening)

#### Key Architectural Rules
1. **Pure Response Parsing (`parse_ctrader_response`)**:
   - Decoupled from network I/O (`json_str: &str -> Result<Vec<f64>, BridgeError>`).
   - Extracts the most recent `bars_count` candle closes in chronological order.
   - Distinguishes between valid responses with fewer than requested bars, empty data (`BridgeError::EmptyData`), API errors (`BridgeError::BridgeReportedError`), and broken syntax (`BridgeError::MalformedJson`).
2. **Fail-Fast in Live Mode (Zero Silent Fallbacks)**:
   - When `--live` is passed, any bridge failure (connection refused, non-200 HTTP, malformed JSON, empty data, timeout) aborts execution with a clear diagnostic message and a non-zero exit code (`exit(1)`).
   - Silently substituting synthetic fixtures for real market feeds is strictly prohibited in live mode.
   - Synthetic data remains the deterministic, zero-network default when `--live` is omitted.
3. **Offline Network Integration Tests**:
   - Ephemeral `TcpListener` servers simulate non-200 HTTP codes, malformed payloads, timeouts, and dropped connections locally, ensuring 100% deterministic test coverage without hitting live endpoints.

---

### 🧠 Part 3 Educational Deep Dive — Schema Validation & System One Boundary

#### 🧸 Part A — The mystery box
- **The Customs Gate (`validate_envelope`)**: When a package arrives from an outside warehouse (the AI provider), the guard checks the invoice format (`schema_version == 1`), makes sure the recipient tag matches the order number (`snapshot_id == expected_id`), and inspects the goods to ensure no poisoned or impossible items (`NaN` or numbers greater than 100%) were delivered. If anything fails inspection, the package is rejected right at the border.
- **The Decoupled Adapter (`Interpreter`)**: Your trading desk doesn't care whether answers come from Jev, an offline flight simulator (`MockInterpreter`), or a recorded flight recorder. It only asks for `interpret(&snapshot)` and expects a valid `Envelope`.

#### 📈 Part B — The market
| Concept | Code | Lesson Origin |
|---|---|---|
| Correlated Snapshot | `Snapshot { id: 7, observed_at_ms: 1000 }` | Lesson 24 (Boundary & Freshness) |
| Typed Probabilities | `InterpretationPayload { p_persistent, p_active, confidence }` | Lesson 19 & 21 (Decomposed questions) |
| Schema Lock | `if env.schema_version != 1 { return Err(InferenceError::InvalidSchema); }` | Lesson 19 (Predefined schemas) |
| Correlation Lock | `if env.snapshot_id != expected_snapshot_id` | Lesson 24 (Snapshot mismatch rejection) |
| Probability Bounds | `val.is_nan() || !(0.0..=1.0).contains(&val)` | Strict numeric validation |
| Provider Decoupling | `pub trait Interpreter { fn interpret(...) }` | Lesson 24 (Provider-neutral trait) |

#### 💻 Runnable Educational Examples
```rust
// Run with: cargo test --example capstone
use capstone::{validate_envelope, Envelope, InferenceError, InterpretationPayload, MockInterpreter, Interpreter, Snapshot};

let snapshot = Snapshot { id: 7, observed_at_ms: 1000 };
let interpreter = MockInterpreter::standard_fixture(snapshot.id);
let env = interpreter.interpret(&snapshot).unwrap();

// 1. Valid envelope passes validation
assert_eq!(validate_envelope(&env, 7), Ok(()));

// 2. Snapshot ID mismatch rejected
assert!(validate_envelope(&env, 99).is_err());

// 3. Out-of-bounds probability rejected
let mut bad_env = env.clone();
bad_env.interpretation.p_persistent = 1.25;
assert!(matches!(validate_envelope(&bad_env, 7), Err(InferenceError::ValidationFailed(_))));

// 4. Provider failure handled safely
let err_provider = MockInterpreter::new(Err(InferenceError::ProviderUnavailable));
assert_eq!(err_provider.interpret(&snapshot), Err(InferenceError::ProviderUnavailable));
```

---

### ⏱️ Part 4 Educational Deep Dive — Freshness Gate, Policy Engine & Strategy

#### 🧸 Part A — The mystery box
- **The Expiry Stamp (`check_freshness`)**: Even the most brilliant weather forecast or radar ping is useless if it tells you what the storm was doing 10 minutes ago, or if someone's clock accidentally claims it's from tomorrow. If the data is more than 200ms old or dated in the future, the system refuses to act on it and defaults to safety (`Hesitate`).
- **The Fail-Safe Circuit Breaker (`decide_behaviour`)**: The system defaults to standing still (`Hesitate`) on *any* anomaly — missing price points, non-positive price movements, low AI confidence (< 0.60), network hiccups, or expired timestamps. You only move forward (`Walk` or `Run`) when every condition is demonstrably verified.
- **The Strategy Pluggability (`Strategy` trait)**: Consolidating Lesson 11, the decision logic is decoupled behind a lightweight trait `evaluate(...)`. This lets you swap deterministic rules, test harness mock strategies, or future models without re-architecting your ingestion pipeline.
- **Display Semantics vs Orders (`DisplayDecision`)**: The labels `Investigate`, `Observe`, and `Wait` are informative terminal display states for the human operator. They are strictly decoupled from order generation — no trading instructions or order routing exists in this model.

#### 📈 Part B — The market
| Concept | Code | Lesson Origin |
|---|---|---|
| Freshness Gate | `check_freshness(&snapshot, now_ms, max_age_ms)` | Lesson 24 (Boundary & Freshness, 200ms limit) |
| Future Timestamp Rejection | `snapshot.observed_at_ms > now_ms` | Lesson 24 (Time travel prevention) |
| Fail-Safe Policy | `decide_behaviour(...) -> (VisualBehaviour, String)` | Lesson 22 (Deterministic policy, safe defaults) |
| Confidence Gate | `if env.interpretation.confidence < 0.60` | Lesson 22 (Confidence thresholding) |
| Positive Delta Requirement | `if feats.delta <= 0.0` | Lesson 20 & 22 (Feature polarity) |
| Pluggable Engine | `pub trait Strategy { fn evaluate(...) }` | Lesson 11 (Traits for strategy abstraction) |
| Visual to Display Mapping | `Run -> Investigate`, `Walk -> Observe`, `Hesitate -> Wait` | Capstone Spec (Deterministic display mapping) |

#### 💻 Runnable Educational Examples
```rust
// Run with: cargo test --example capstone
use capstone::{
    check_freshness, decide_behaviour, DeterministicStrategy, DisplayDecision,
    Features, Envelope, InterpretationPayload, Snapshot, Strategy, VisualBehaviour,
};

let snapshot = Snapshot { id: 7, observed_at_ms: 1000 };
let features = Features { delta: 3.0, positive_changes: 2 };
let env = Envelope {
    schema_version: 1,
    snapshot_id: 7,
    interpretation: InterpretationPayload {
        p_persistent: 0.85,
        p_active: 0.90,
        confidence: 0.90,
    },
};

// 1. Strong branch evaluation (Run -> Investigate)
let strategy = DeterministicStrategy;
let (visual, decision, reason) = strategy.evaluate(
    Some(&features),
    Ok(&env),
    &snapshot,
    1100, // age = 100ms <= 200ms
    200,
);
assert_eq!(visual, VisualBehaviour::Run);
assert_eq!(decision, DisplayDecision::Investigate);

// 2. Freshness rejection (age 201ms > 200ms -> Hesitate -> Wait)
let (visual_stale, decision_stale, _) = strategy.evaluate(
    Some(&features),
    Ok(&env),
    &snapshot,
    1201, // age = 201ms > 200ms
    200,
);
assert_eq!(visual_stale, VisualBehaviour::Hesitate);
assert_eq!(decision_stale, DisplayDecision::Wait);

// 3. Low confidence gate (confidence 0.40 < 0.60 -> Hesitate -> Wait)
let mut low_conf_env = env.clone();
low_conf_env.interpretation.confidence = 0.40;
let (visual_low_conf, _, _) = strategy.evaluate(
    Some(&features),
    Ok(&low_conf_env),
    &snapshot,
    1100,
    200,
);
assert_eq!(visual_low_conf, VisualBehaviour::Hesitate);
```

---

### 🕹️ Part 5 Educational Deep Dive — Multi-Scenario CLI Runner

#### 🧸 Part A — The mystery box
- **The Flight Simulator Matrix (`ScenarioMode`)**: Before putting real capital on the line, pilots spend hours in a full-motion simulator testing severe icing, engine failure, loss of radar, and emergency descents. The Multi-Scenario CLI Runner is our algorithmic flight simulator: each flag (`strong`, `mixed`, `uncertain`, `stale`, `error`, `empty`) simulates an exact operating regime deterministically.
- **Why Fake Data Matters First**: Production systems fail most catastrophically at boundary transitions — when an upstream API returns empty arrays, drops connections, or delivers an observation that expired 1 millisecond ago. Having repeatable local fixtures lets you prove your safety invariant (`Hesitate` / `Wait`) without waiting for a flash crash on live exchanges.

#### 📈 Part B — The market
| Scenario | CLI Invocation | Observable Outcome | Why it matters |
|---|---|---|---|
| `strong` | `cargo run --example capstone -- strong` | `Run` / `Investigate` | Proves bullish trend and strong model consensus triggers active branch |
| `mixed` | `cargo run --example capstone -- mixed` | `Walk` / `Observe` | Confirms moderate persistence doesn't over-commit |
| `uncertain` | `cargo run --example capstone -- uncertain` | `Hesitate` / `Wait` | Proves low confidence ($0.40 < 0.60$) triggers safety circuit breaker |
| `stale` | `cargo run --example capstone -- stale` | `Hesitate` / `Wait` | Proves latency / clock skew ($301\text{ms} > 200\text{ms}$) aborts execution |
| `error` | `cargo run --example capstone -- error` | `Hesitate` / `Wait` | Confirms provider outages degrade gracefully without panic |
| `empty` | `cargo run --example capstone -- empty` | `Hesitate` / `Wait` | Confirms zero data safely returns `delta=N/A` without slice panics |

#### 💻 Runnable Educational Examples
```rust
// Run with: cargo test --example capstone
use capstone::{run_pipeline, ScenarioMode, DisplayDecision, VisualBehaviour};

// 1. Run the 'strong' scenario
let strong_cfg = ScenarioMode::Strong.config();
let strong_res = run_pipeline(&strong_cfg, false).unwrap();
assert_eq!(strong_res.visual, VisualBehaviour::Run);
assert_eq!(strong_res.decision, DisplayDecision::Investigate);

// 2. Run the 'stale' scenario (301ms age)
let stale_cfg = ScenarioMode::Stale.config();
let stale_res = run_pipeline(&stale_cfg, false).unwrap();
assert_eq!(stale_res.visual, VisualBehaviour::Hesitate);
assert_eq!(stale_res.decision, DisplayDecision::Wait);
assert!(!stale_res.valid);

// 3. Run the 'empty' scenario (0 ticks)
let empty_cfg = ScenarioMode::Empty.config();
let empty_res = run_pipeline(&empty_cfg, false).unwrap();
assert!(empty_res.prices.is_empty());
assert_eq!(empty_res.visual, VisualBehaviour::Hesitate);
```

---

### 📼 Part 6 Educational Deep Dive — Deterministic Replay Engine

#### 🧸 Part A — The mystery box
- **The Flight Black Box (`ReplayRecord`)**: When an airplane lands (or encounters turbulence), the flight recorder doesn't re-run the physical flight; it captures every sensor reading, flight surface command, and clock timestamp exactly as they happened. In our trading engine, the `ReplayRecord` freezes the exact market snapshot, feature slice, model payload, schema version, policy version, and logical evaluation time.
- **Why Zero Provider Calls Matter**: If replaying past trading decisions required re-querying the AI model or live exchange bridge, every audit or backtest would suffer from network latency, API costs, and stochastic non-determinism. By isolating the deterministic policy into `evaluate_replay`, we guarantee $100\%$ byte-for-byte reproducibility with strictly $0$ external calls.
- **Policy Version Governance**: Trading algorithms evolve over time. If a historical audit record was evaluated under Policy v1, running it under an unverified Policy v2 must fail immediately (`ReplayError::UnsupportedPolicyVersion`) rather than silently producing invalid retrospective actions.

#### 📈 Part B — The market
| Concept | Code | Lesson Origin |
|---|---|---|
| Replay Record | `ReplayRecord { snapshot, features, interpretation, now_ms, ... }` | Lesson 25 (Audit logs & record serialization) |
| Offline Policy Evaluation | `evaluate_replay(&record, version, max_age_ms)` | Lesson 25 (Pure deterministic evaluation) |
| Zero External Calls | `calls_after - calls_before == 0` | Lesson 25 (Cost & latency elimination) |
| Policy Version Gate | `if record.policy_version != current_policy_version` | Lesson 25 (Version mismatch protection) |
| Schema Version Gate | `if record.schema_version != CURRENT_SCHEMA_VERSION` | Lesson 19 & 25 (Schema governance) |
| Replay Freshness Safety | Stale/future records evaluated via `decide_behaviour` | Lesson 24 (Consistent temporal rules in replay) |

#### 💻 Runnable Educational Examples
```rust
// Run with: cargo test --example capstone
use capstone::{
    evaluate_replay, Features, InterpretationPayload, MockInterpreter,
    ReplayError, ReplayRecord, Snapshot, VisualBehaviour, CURRENT_POLICY_VERSION,
};

let interpreter = MockInterpreter::standard_fixture(7);
let record = ReplayRecord {
    snapshot: Snapshot { id: 7, observed_at_ms: 1000 },
    features: Some(Features { delta: 3.0, positive_changes: 2 }),
    interpretation: Some(InterpretationPayload {
        p_persistent: 0.85,
        p_active: 0.90,
        confidence: 0.90,
    }),
    source: "recorded".to_string(),
    now_ms: 1100,
    schema_version: 1,
    policy_version: 1,
};

// 1. Replay twice: assert identical outputs & strictly 0 provider calls
let calls_before = interpreter.call_count();
let replay1 = evaluate_replay(&record, CURRENT_POLICY_VERSION, 200).unwrap();
let replay2 = evaluate_replay(&record, CURRENT_POLICY_VERSION, 200).unwrap();
let calls_after = interpreter.call_count();

assert_eq!(replay1, replay2);
assert_eq!(calls_after - calls_before, 0);

// 2. Unsupported policy version rejected immediately
let mut v2_record = record.clone();
v2_record.policy_version = 2;
assert_eq!(
    evaluate_replay(&v2_record, CURRENT_POLICY_VERSION, 200),
    Err(ReplayError::UnsupportedPolicyVersion(2))
);
```

---

### 🎛️ Part 7 Educational Deep Dive — Configurable Thresholds & Curriculum Exit

#### 🧸 Part A — The mystery box
- **The Tunable Thermostat (`PolicyThresholds`)**: A hardcoded thermostat set to $21^\circ\text{C}$ works fine in summer, but in freezing winter or an industrial cleanroom you need an interface to adjust setpoints without rebuilding the furnace. `PolicyThresholds` decouples the rules of decision-making from the numeric trigger levels.
- **Predict, Change, Verify**: The true test of system mastery is knowing what will happen before you run the code. If we relax our confidence requirement from $0.60$ down to $0.35$, an uncertain market observation ($0.40$) that previously triggered `Hesitate` / `Wait` immediately promotes to `Run` / `Investigate`.

#### 📈 Part B — The market
| Component | Implementation | Educational Purpose |
|---|---|---|
| Policy Thresholds | `PolicyThresholds { min_confidence, min_p_persistent, min_p_active }` | Lesson 20 & 22 (Decoupled risk parameters) |
| Parameterized Policy | `decide_behaviour_with_thresholds(...)` | Allows flexible simulation and backtesting |
| Strategy Trait Implementation | `ConfigurableStrategy { thresholds }` | Lesson 11 (Object-oriented / trait polymorphism) |
| Sensitivity Verification | `uncertain` under relaxed threshold $\to$ `Run` | Verifies deterministic predictability |

#### 💻 Runnable Educational Examples
```rust
// Run with: cargo test --example capstone
use capstone::{
    decide_behaviour_with_thresholds, ConfigurableStrategy, Features,
    InterpretationPayload, Envelope, Snapshot, PolicyThresholds,
    VisualBehaviour, DisplayDecision, Strategy,
};

let snapshot = Snapshot { id: 7, observed_at_ms: 1000 };
let features = Features { delta: 3.0, positive_changes: 2 };
let uncertain_env = Envelope {
    schema_version: 1,
    snapshot_id: 7,
    interpretation: InterpretationPayload {
        p_persistent: 0.85,
        p_active: 0.90,
        confidence: 0.40, // Below default 0.60
    },
};

// 1. Standard thresholds reject low confidence (Hesitate)
let standard = PolicyThresholds::default();
let (vis_std, _) = decide_behaviour_with_thresholds(
    Some(&features),
    Ok(&uncertain_env),
    &snapshot,
    1100,
    200,
    &standard,
);
assert_eq!(vis_std, VisualBehaviour::Hesitate);

// 2. Relaxed confidence threshold (0.35) promotes to Run
let relaxed = PolicyThresholds {
    min_confidence: 0.35,
    min_p_persistent: 0.80,
    min_p_active: 0.80,
};
let (vis_rel, _) = decide_behaviour_with_thresholds(
    Some(&features),
    Ok(&uncertain_env),
    &snapshot,
    1100,
    200,
    &relaxed,
);
assert_eq!(vis_rel, VisualBehaviour::Run);
```

