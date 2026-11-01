# JevFloor Learning Day — Lesson Progress Tracker

Full notes, mystery box analogies, and takeaways are documented in [LESSON_NOTES.md](LESSON_NOTES.md).

---

## Phase 1: Core Rust Foundations (100% Complete! 🎉)

- [x] **Lesson 1 — Cargo & Toolchain** (`examples/l01_toolchain.rs`)  
  `check`, `build`, `run`, `test`, feedback loop.
- [x] **Lesson 2 — Structs, Enums & Match** (`examples/l02_types.rs`)  
  Tagged unions, payloads, exhaustive pattern matching.
- [x] **Lesson 3 — Ownership & Moves** (`examples/l03_moves.rs`)  
  Single owner, value hand-offs, scope, and drops.
- [x] **Lesson 4 — `Copy` vs `Clone`** (`examples/l04_copy_clone.rs`)  
  Implicit stack duplication vs explicit heap cloning.
- [x] **Lesson 5 — Shared Borrowing** (`examples/l05_borrow.rs`)  
  Looking without taking (`&T`), read-only views.
- [x] **Lesson 6 — Mutable Borrowing** (`examples/l06_mut_borrow.rs`)  
  Exclusive read/write access (`&mut T`), preventing aliasing bugs.
- [x] **Lesson 7 — `Vec` Growth & Invalidation** (`examples/l07_vec.rs`)  
  Heap reallocations, capacity, and preventing dangling pointers.
- [x] **Lesson 8 — Slices & Views** (`examples/l08_slices.rs`)  
  `&[T]` fat pointers (address + length) without copying memory.
- [x] **Lesson 9 — `Option`, `Result` & `?`** (`examples/l09_results.rs`)  
  Absence (`None`) vs failure (`Err`), propagating with `?`.
- [x] **Lesson 10 — Iterators** (`examples/l10_iterators.rs`)  
  `iter()`, `iter_mut()`, `into_iter()`, lazy pipelines.
- [x] **Lesson 11 — Traits & Generics** (`examples/l11_traits.rs`)  
  Capabilities, contracts, static dispatch (monomorphization).
- [x] **Lesson 12 — Practical Lifetimes (`'a`)** (`examples/l12_lifetimes.rs`)  
  Connecting borrow durations, lifetime elision, and avoiding dangling references.

---

## Phase 2: Concurrency, Async & Architecture (100% Complete! 🎉)

- [x] **Lesson 13 — `Box`, `Arc` & `Mutex`** (`examples/l13_shared.rs`)  
  Heap allocation, atomic ref-counting, thread-safe shared mutation.
- [x] **[Practical Detour] — RxJS Streams & Tokio** (`examples/rx_streams.rs`)  
  Real Coinbase API calls, `tokio::join!` (`forkJoin`), stream `map` + `then` (`switchMap`), and `async move`.
- [x] **Lesson 14 — `Send` & `Sync`** (`examples/l14_send_sync.rs`)  
  Compile-time thread safety markers (why `Rc` cannot cross threads).
- [x] **Lesson 15 — Channels & Ownership Transfer** (`examples/l15_channels.rs`)  
  MPSC message passing, bounded capacity, backpressure, and graceful termination.
- [x] **Lesson 16 — Async & Tokio** (`examples/l16_async.rs`)  
  Futures, cooperative multitasking, `.await`, timeouts, and task cancellation.
- [x] **Lesson 17 — Bevy ECS Bridge** (`examples/l17_ecs.rs`)  
  Entities, components, resources, and queries without game rendering.

---

## Phase 3: Jev & System One Integration (Lessons 18–25)

- [x] **Lesson 18 — System One vs System Two** (`examples/l18_system_one.rs`)  
  Fast structured inference vs slow deliberation.
- [x] **Lesson 19 — Predefined Schemas** (`examples/l19_schema.rs`)  
  Typed probabilistic boundaries and strict schema validation.
- [x] **Lesson 20 — Probability, Confidence & Uncertainty** (`examples/l20_uncertainty.rs`)  
  Separating event probabilities from model confidence.
- [x] **Lesson 21 — Decomposed Questions** (`examples/l21_questions.rs`)  
  Breaking complex market state into small, verifiable binary questions.
- [x] **Lesson 22 — Deterministic Policy Control** (`examples/l22_policy.rs`)  
  Pure Rust arithmetic and logic controlling behavior from probabilistic inputs.
- [x] **Lesson 23 — Shared vs Per-Agent Inference** (`examples/l23_sharing.rs`)  
  Cache identity, latency tradeoffs, and simulated API costs.
- [x] **Lesson 24 — Provider-Neutral Boundary** (`examples/l24_boundary.rs`)  
  Decoupling business logic from vendor APIs, freshness, and timeouts.
- [x] **Lesson 25 — Replay & Determinism** (`examples/l25_replay.rs`)  
  Replaying recorded decisions without invoking live models.

---

## Phase 4: Simple Jev Project (100% Complete! 🎉)

- [x] **Apple Notes Classifier**: a personal side project, not included in the public copy.


## Phase 5: Capstone Project (100% Complete! 🎉)

- [x] **Capstone** (`examples/capstone.rs`)  
  End-to-end pipeline:
  ```text
  Market Event Stream (Channel)
         ↓
  Rolling Buffer & Slice Feature Calculation
         ↓
  Typed System One Interpretation & Freshness Gate
         ↓
  Deterministic Policy
         ↓
  Visual Character Behaviour: Hesitate / Walk / Run
  ```
