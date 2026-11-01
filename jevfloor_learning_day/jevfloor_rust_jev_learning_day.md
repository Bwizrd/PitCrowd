# JevFloor: one-day Rust → Jev interactive learning brief

## Instructions to the coding tutor

Teach an experienced senior software engineer the Rust foundations needed to understand a future JevFloor project, followed by TypeSafe AI Jev/System One concepts. **This is a learning day, not production coding. Do not start building JevFloor. The learner must personally type every exercise and run it.** Use fake market data only; none of the examples imply a real market prediction or trading strategy.

Work one lesson at a time. Do not dump the curriculum's solutions into the workspace. Do not implement exercises on the learner's behalf. You may help with tool installation and explain commands, but let the learner create the learning programs. Familiarity with software engineering does not imply familiarity with Rust ownership.

For every lesson, follow this protocol:

1. Explain the concept briefly, connecting it to an engineering concept the learner knows. Ask their primary language once and adapt analogies.
2. Give the small task below, relevant signatures or syntax hints, and its observable acceptance criteria. Give no completed implementation initially.
3. Ask the learner to predict the output or compiler response, then **wait for their attempt**.
4. Have them personally run the command. Ask for the actual output, error, or relevant code; never pretend an unobserved run succeeded.
5. Diagnose before fixing: identify what the compiler protects, ask a guiding question, then offer progressively stronger hints. Use deliberate compiler failures as teaching material. Do not teach gratuitous cloning, `unsafe`, or broad lifetime annotations as escape hatches.
6. Give a full solution only after an attempt or an explicit request. Even then, have the learner type/adapt it and run a small variation themselves.
7. End with a successful runnable payoff and the explain-back checkpoint. **Do not advance until the learner has run the exercise and can explain the checkpoint.** If stuck, shrink the exercise and repeat rather than silently skipping it.

All lessons below, including Jev lessons, require personally written runnable code. Expected output specifies meaningful lines or assertions, not Cargo's version-dependent build messages. Intentional compile failures must be repaired before the lesson closes.

## One-day pace and setup

Aim for roughly eight focused hours plus breaks: Rust lessons 1–12 about three hours; concurrency/async/ECS lessons 13–17 about ninety minutes; Jev lessons 18–25 about two hours; capstone and review about ninety minutes. This is ambitious. Keep exercises tiny and explanations short, check pace after each block, and use a second sitting if the learner needs it. Never sacrifice the runnable checkpoints to meet a clock. Optional live API access is the first thing to omit.

Use one disposable Cargo package, `jevfloor_learning_day`, with independent files under `examples/`: `l01_toolchain.rs` through `l25_replay.rs`, plus `capstone.rs`. Each example can have its own small types and `main`; do not spend the day designing shared modules. Tests can live in each example under `#[cfg(test)]`.

Command pattern: `cargo run --example NAME`; tests: `cargo test --example NAME`. Create only today's example when its lesson begins. Keep deliberately broken code in the current example until repaired, not in an additional Cargo target. Prefer standard-library tools until the async/ECS lessons. Use stable Rust and edition 2024 when the installed toolchain supports it. Check the actual installed versions; do not guess them.

Use these recurring teaching types, introducing fields as needed:

- `Candle`: integer `open`, `high`, `low`, `close`, plus `volume: u64`. Integers are toy price units, avoiding unnecessary decimal discussions.
- `MarketEvent`: an enum such as `Tick { symbol: String, price: i64 }` and `Closed(Candle)`.
- `RollingBuffer`: a small bounded sequence of prices.
- `Strategy`: a trait returning a local deterministic decision.
- `VisualBehaviour`: `Hesitate`, `Walk`, `Run`; these are printed labels, not animations or trade orders.

## Rust lessons

### 1. Cargo and the toolchain — make the feedback loop work (10 min)

**Explain:** `rustc` compiles; Cargo manages packages, dependencies, targets and tests. Distinguish `check`, `build`, `run`, and `test`.

**Learner task:** Run `rustc --version` and `cargo --version`; create the package with `cargo new jevfloor_learning_day`, enter it, and create `examples/l01_toolchain.rs`. Write a `main` printing `JevFloor learning day: fake data only`, plus a test asserting a toy price delta of `103 - 100 == 3`.

**Run/payoff:** `cargo check --example l01_toolchain`, `cargo run --example l01_toolchain`, `cargo test --example l01_toolchain`. Observe the exact banner and one passing test. If installation is needed, consult official Rust setup instructions for the learner's OS.

**Explain back:** Which command checks without running? Where does execution begin? What does Cargo manage?

### 2. Structs, enums and exhaustive `match` (10 min)

**Explain:** Structs group data; enum variants represent alternatives with their own payloads. `match` forces coverage.

**Learner task:** Define `Candle` and `MarketEvent`, construct one tick and one closed candle, and format them using `match`. Add a `Disconnected` variant and see what an incomplete match rejects, then handle it.

**Run/payoff:** `cargo run --example l02_types` prints `tick SILVER 103`, `candle close=103`, and `disconnected` for the three events.

**Explain back:** Why is an enum preferable to a string tag and several optional fields here?

### 3. Ownership and moves (15 min)

**Explain:** A value has an owner; passing an owned non-`Copy` value transfers ownership. Moving a `String` does not mean copying its whole allocation.

**Learner task:** Pass a `MarketEvent` containing `String` into `consume(event)`. Try to inspect the same event afterward. Read the moved-value error. Repair it by returning the event from the function and rebinding it in the caller; borrowing comes later.

**Run/payoff:** First `cargo check --example l03_moves` rejects the use after move. After repair, `cargo run --example l03_moves` prints `consumer saw SILVER` then `caller owns SILVER again`.

**Explain back:** Who owns the event at each step, and when would its string be dropped?

### 4. `Copy` versus `Clone` (10 min)

**Explain:** `Copy` permits implicit value duplication; `Clone` is explicit and type-specific. A type with an owned `String` cannot derive `Copy`. Cloning `Arc` later will mean something different from cloning a `String`.

**Learner task:** Copy an integer price and change the copy. Clone a symbol string and append `-SIM` only to the clone. Attempt to derive `Copy` for a string-owning event, inspect the error, then remove that derive.

**Run/payoff:** `cargo run --example l04_copy_clone` prints `prices original=100 copy=101` and `symbols original=SILVER clone=SILVER-SIM`.

**Explain back:** Why are both originals usable, and which duplication may allocate?

### 5. Shared borrowing (15 min)

**Explain:** `&T` grants shared access without transferring ownership. References cannot outlive their referents.

**Learner task:** Write `describe(event: &MarketEvent)`, call it twice, and then consume the owned event. Do not clone it.

**Run/payoff:** `cargo run --example l05_borrow` prints the description twice followed by `consumed SILVER`.

**Explain back:** Why can the caller still move the event after the shared borrows' last use?

### 6. Mutable borrowing (15 min)

**Explain:** `&mut T` provides exclusive access while that borrow is active; shared aliases cannot simultaneously be used. Borrow duration often ends at last use rather than at the closing brace.

**Learner task:** Write `adjust_close(candle: &mut Candle, delta: i64)`. Hold a shared reference to the candle, call the mutator, and use the shared reference afterward to trigger an error. Rearrange uses/scopes to fix it without cloning.

**Run/payoff:** After the deliberate `cargo check` failure, `cargo run --example l06_mut_borrow` prints `before=100` and `after=103`.

**Explain back:** Which overlapping accesses conflicted? Why did changing their order help?

### 7. `Vec` growth and reference invalidation (15 min)

**Explain:** `Vec` stores elements in a growable allocation. Growth can relocate them. Rust disallows conflicting borrows even when current capacity happens to be enough; this is not a runtime prediction about a particular push.

**Learner task:** Begin a `RollingBuffer` with a vector containing `100`. Borrow its first element, push `101`, then print the borrowed element. Diagnose the error. Repair by copying the integer before the push or reborrowing after it. Print length/capacity before and after several pushes.

**Run/payoff:** `cargo check --example l07_vec` initially fails; the repaired `cargo run --example l07_vec` prints `first=100` and a larger final length. Capacity values may vary; never assert a particular growth factor or address change.

**Explain back:** Why would spare capacity not make the original overlapping accesses legal?

### 8. Slices and views (10 min)

**Explain:** `&[T]` is a borrowed contiguous view; `&mut [T]` grants exclusive access to a view. A slice does not own or resize the collection.

**Learner task:** Write `sum_prices(prices: &[i64]) -> i64`; use `[100, 101, 103]`, its last two elements, and an empty slice. Add an `increment(&mut [i64])` helper and mutate just the first two elements.

**Run/payoff:** `cargo run --example l08_slices` prints `all=304 tail=204 empty=0` and `changed=[101, 102, 103]`.

**Explain back:** Why can the same summing function accept both arrays and vectors through slices?

### 9. `Option`, `Result` and `?` (15 min)

**Explain:** Absence differs from failure. Match recoverable cases explicitly; `?` propagates a compatible error or absence from a function rather than handling it magically.

**Learner task:** Implement `RollingBuffer::last() -> Option<i64>` and parse a price into `Result<i64, std::num::ParseIntError>`. Use `?` inside a helper; handle its result in `main`. Test the empty buffer, `"103"`, and `"oops"` without panicking.

**Run/payoff:** `cargo run --example l09_results` prints `empty=None`, `price=103`, `parse failed`. `cargo test --example l09_results` passes three corresponding tests.

**Explain back:** What does `?` return from, and why is an empty buffer not necessarily an error?

### 10. `iter`, `iter_mut`, `into_iter` (15 min)

**Explain:** On an owned vector, these provide shared references, exclusive references, and owned elements respectively. Iterator adapters are lazy until consumed.

**Learner task:** Use `iter()` to sum `[100, 101, 103]`, `iter_mut()` to increment each item, then `into_iter()` to collect owned prices above `101`. Inspect an attempted use of the vector after consumption and repair it. Use a vector explicitly to avoid array-edition distractions.

**Run/payoff:** `cargo run --example l10_iterators` prints `sum=304`, `updated=[101, 102, 104]`, `selected=[102, 104]`.

**Explain back:** Name the item type yielded at each stage and explain why the final vector move matters.

### 11. Traits and generics (20 min)

**Explain:** Traits describe capabilities. A generic bound asks for those capabilities with static dispatch; it does not mean inheritance.

**Learner task:** Define `Strategy` with a method accepting a candle reference and returning a small `Decision` enum. Implement `PositiveClose` and `AlwaysWait`; write `evaluate<S: Strategy>(strategy: &S, candle: &Candle)`. For a candle opening at `100` and closing at `103`, the first strategy watches and the second waits.

**Run/payoff:** `cargo run --example l11_traits` prints `PositiveClose=Watch` and `AlwaysWait=Wait`; `cargo test --example l11_traits` verifies both.

**Explain back:** What can `evaluate` assume about `S`? What determines which method implementation runs?

### 12. Practical lifetimes (20 min)

**Explain:** Lifetimes describe relationships between borrows; annotations do not extend storage duration. Prefer owned returns when that is the honest design, and use elision when it already expresses the relationship.

**Learner task:** Write `latest(prices: &[i64]) -> Option<&i64>` using elision. Then write a helper choosing one of two borrowed candles with signature `fn higher_close<'a>(a: &'a Candle, b: &'a Candle) -> &'a Candle`. Try returning a reference to a locally created candle; diagnose and remove that invalid helper.

**Run/payoff:** After inspecting the invalid-reference error, `cargo run --example l12_lifetimes` prints `latest=103` and `higher=105` from caller-owned data.

**Explain back:** Why does `'a` not keep a local candle alive? What restricts how long the returned borrow can be used?

### 13. `Box`, `Arc` and `Mutex` — brief practical pass (15 min)

**Explain:** `Box<T>` owns heap storage; `Arc<T>` shares ownership using an atomic reference count; `Mutex<T>` coordinates exclusive access. Shared ownership alone does not permit mutation. A poisoned lock is a distinct error condition.

**Learner task:** Put one candle in a box and print its close. Share `Arc<Mutex<u64>>` between two threads; each increments once. Clone the `Arc`, join both threads, and inspect the counter. Keep lock guards short. An `expect` is acceptable here if explicitly identified as a learning-only error policy.

**Run/payoff:** `cargo run --example l13_shared` prints `boxed close=103` and, after joining, `processed=2`.

**Explain back:** Does cloning the `Arc` duplicate the counter? Which object owns storage, and which protects mutation?

### 14. `Send` and `Sync` (15 min)

**Explain:** `Send` means a value may be transferred between threads; `Sync` means shared references to it may cross threads safely. `Arc` does not automatically make its payload thread-safe. These marker traits are generally inferred; do not manually implement unsafe traits today.

**Learner task:** Write `assert_send<T: Send>()` and `assert_sync<T: Sync>()`. Check a string-owning `MarketEvent` and `Arc<Mutex<u64>>`. Try moving an `Rc` clone into `thread::spawn`; read the failure and replace it with a suitable `Arc`. Briefly inspect why `Arc<RefCell<_>>` is not a substitute for synchronization.

**Run/payoff:** Repaired `cargo run --example l14_send_sync` prints `worker saw SILVER`; static assertion calls compile.

**Explain back:** Explain the difference between moving an owner and sharing a reference across threads.

### 15. Channels and ownership transfer (15 min)

**Explain:** A channel transfers messages between producers and consumers; disconnect and bounded capacity are part of its behaviour.

**Learner task:** Use `std::sync::mpsc::sync_channel(2)`. A producer thread sends three owned `MarketEvent` ticks with prices `100`, `101`, `103` in that order. The main thread receives until all senders drop, then joins the producer. Do not wait to join before receiving from the bounded channel.

**Run/payoff:** `cargo run --example l15_channels` prints the three ticks in producer order followed by `stream closed; received=3`, then terminates.

**Explain back:** Why can an extra live sender prevent termination? Why can joining too early deadlock this exercise?

### 16. Async and Tokio (25 min)

**Explain:** A future represents work that advances when polled. `.await` can yield to the runtime; async is not automatically CPU parallelism. Spawned tasks can require owned, `Send + 'static` data. Do not block the executor or hold a standard mutex guard across `.await`.

**Learner task:** Check official Tokio documentation for the installed version. Add Tokio with `macros`, `rt-multi-thread`, `time`, and `sync` features (for example `cargo add tokio --features macros,rt-multi-thread,time,sync`). Write an async `main` using `#[tokio::main]`. Spawn one producer that sleeps asynchronously for a small interval before each of three ticks, sends through a bounded Tokio channel, and drops its sender. Receive and print all ticks in the main task; await the task handle. Wrap receiving in a generous timeout so bugs do not hang indefinitely.

**Run/payoff:** `cargo run --example l16_async` prints `async tick=100`, `async tick=101`, `async tick=103`, `async complete`. Exact timing is not asserted. Add a test or alternate mode with a deliberately slow producer and short timeout that prints `timeout -> Hesitate`; abort and await the unfinished task in that path.

**Explain back:** What runs during a sleep's `.await`? How do timeout, cancellation and a thread join differ?

### 17. Bevy ECS bridge without a game (20 min)

**Explain:** An entity is an identity; components are attached data; systems operate on data selected by queries; resources are shared world-level data. ECS access declarations communicate borrowing requirements to scheduling. Runtime ECS access validation is related to, but not identical to, Rust's compile-time checks.

**Learner task:** Prefer the lightweight `bevy_ecs` crate and inspect official documentation for its installed version before selecting APIs. Create a headless `World` and `Schedule`. Spawn one entity with `Trader` and `Activity(u8)` components, initially activity `0`. Insert a `MarketPulse(3)` resource. Write a system using `Res<MarketPulse>` and `Query<&mut Activity, With<Trader>>`; run the schedule twice and read the result after each run.

**Run/payoff:** `cargo run --example l17_ecs` prints `activity=3` then `activity=6`. No window, rendering assets or JevFloor scaffolding. If dependency access is unavailable, write and run a tiny local entity/component store applying the same update and print those lines; explicitly label it an ECS analogy, not a Bevy implementation, and defer the real Bevy run.

**Explain back:** Identify the entity, components, resource, system and query in your code. Why would two overlapping mutable queries need disjoint filters or a different access pattern?

## Jev grounding and boundaries

Primary reading: [TypeSafe AI, Introducing System One Models & Jev](https://typesafe.ai/blog/introducing-system-one-models-and-jev), published 15 September 2026; checked 26 September 2026. For any live integration, consult [current TypeSafe documentation](https://docs.typesafe.ai/) at lesson time.

The article presents System One as fast structured inference, contrasting it with slower deliberative System Two reasoning. It describes Jev as accepting state and returning predefined typed outputs with probabilities and confidence, and advocates decomposed questions embedded in ordinary software. TypeSafe claims schema guarantees, calibrated outputs and performance benefits; these are vendor claims, not independently verified guarantees for this project. Schema validity does not establish factual correctness, market predictiveness or suitability of an action. The article's benchmark comparisons have caveats; do not turn them into application latency or cost promises.

**Everything below is an original local teaching design, not Jev's API contract.** Rust structs, field names, fixtures, confidence values, thresholds and envelopes are illustrative. Do not imply an official Rust SDK exists or invent endpoints, model identifiers, authentication, wire formats, or probability semantics. Mock outputs demonstrate consumption and control flow; they do not demonstrate real model intelligence or calibration. Rust calculation of a score is not Jev inference.

Use a local `Interpretation` with `p_persistent: f64`, `p_active: f64`, and an illustrative `confidence: f64`. Wrap it in an envelope carrying `snapshot_id`, `observed_at_ms`, and `schema_version`. These probabilities answer separate binary questions and therefore need not sum to one. If a later exercise uses mutually exclusive exhaustive alternatives instead, validate their distribution appropriately. Keep confidence distinct from event probability and consult official definitions before mapping real provider fields.

### 18. System One versus System Two: where interpretation fits (10 min)

**Explain:** Use the reading's distinction to discuss fast classification versus deliberate multi-step reasoning. A model-family label does not guarantee correctness or response speed on an arbitrary task.

**Learner task:** Write a local mock function accepting a description of a fake snapshot and returning one of two fixed typed interpretations. Separately write a deterministic function computing a price delta. Print the feature and mock interpretation. No remote request and no simulated reasoning transcript are needed.

**Run/payoff:** `cargo run --example l18_system_one` prints `delta=3`, `source=MOCK`, `p_persistent=0.85`. Change the fixture selector and observe `p_persistent=0.35`.

**Explain back:** Which result is calculated, which is a stand-in for inference, and what would the real provider replace?

### 19. Predefined schemas and typed probabilistic outputs (15 min)

**Explain:** Define possible answers before asking the model. Distinguish the internal Rust type, a wire schema, and validation at the provider boundary.

**Learner task:** Define the local interpretation and a `validate` function. Reject non-finite numbers and probabilities/confidence outside `[0,1]`; reject an unsupported schema version. Construct fixtures containing `0.85`, `1.2`, `NaN`, and a wrong version. Avoid JSON dependencies unless useful; direct typed fixtures suffice today.

**Run/payoff:** `cargo run --example l19_schema` prints `valid accepted`, `out-of-range rejected`, `NaN rejected`, `schema rejected`. `cargo test --example l19_schema` checks all four outcomes.

**Explain back:** Why can a valid `f64` be an invalid probability? Why does accepting a schema not establish truth?

### 20. Probability, confidence and uncertainty (15 min)

**Explain:** An estimated event probability is not a certainty or a promised observed frequency in a tiny sample. Confidence must have an explicit meaning; never assume every provider uses the same definition. Calibration requires labelled outcomes across examples.

**Learner task:** Consume three mock fixtures: `(p=0.85, confidence=0.90)`, `(0.85, 0.40)`, and `(0.50, 0.90)`. Write a local display policy: confidence below `0.60` yields `uncertain`; otherwise `p >= 0.80` yields `strong`, and other values yield `mixed`. Test boundary values `0.60` and `0.80`.

**Run/payoff:** `cargo run --example l20_uncertainty` prints `strong`, `uncertain`, `mixed`; `cargo test --example l20_uncertainty` passes the boundary assertions.

**Explain back:** Why do the first two cases share an event estimate yet produce different display labels? What evidence would calibration require?

### 21. Decompose interpretation into small questions (15 min)

**Explain:** Ask understandable questions about supplied state, not an unconstrained command to make a trade. For example: “Does this supplied toy sequence appear persistent?” and “Does the supplied activity description suggest high activity?” Questions need definitions and context.

**Learner task:** Represent these two questions as separate typed fields. Use fixtures `(0.85,0.90)`, `(0.85,0.30)`, `(0.30,0.90)`. Label them `both strong`, `persistence only`, and `activity only` using a threshold of `0.80`.

**Run/payoff:** `cargo run --example l21_questions` prints those three labels. Add a test proving the two probabilities are not forced to sum to one.

**Explain back:** Why should these outputs neither be summed into a categorical distribution nor multiplied as independent evidence without justification?

### 22. Deterministic features and Rust control flow (20 min)

**Explain:** Rust owns arithmetic, validation and the final policy. The probabilistic boundary supplies bounded interpretation. Jev is not a BUY/SELL oracle; the toy output controls a printed character behaviour.

**Learner task:** Calculate `delta = last - first` and count positive adjacent changes from `[100,101,103]`. Consume a mock interpretation. Implement this explicit toy policy: invalid/unavailable/stale or confidence `< 0.60` → `Hesitate`; otherwise positive delta with both probabilities `>= 0.80` → `Run`; otherwise positive delta → `Walk`; otherwise → `Hesitate`. Test strong, mixed, low-confidence and nonpositive cases.

**Run/payoff:** `cargo run --example l22_policy` prints `delta=3 positive_changes=2`, then `strong=Run mixed=Walk uncertain=Hesitate nonpositive=Hesitate`. `cargo test --example l22_policy` verifies all branches. Thresholds are arbitrary teaching constants.

**Explain back:** Which policy remains deterministic if the provider changes? Why does a model output not authorize a trade?

### 23. Shared versus per-agent inference; latency and cost (15 min)

**Explain:** Sharing may reduce duplicate work when inputs/questions match. Different agent context may require distinct requests. Do not reuse results merely because a symbol matches. Include snapshot, schema, questions, provider/model configuration and relevant context in a cache identity.

**Learner task:** Write a counting mock provider with a configured synthetic latency of `25 ms`. Three agents use the same snapshot: compare three independent mock calls with one shared call and three local consumers. Then change one agent's context and require a separate call. Compute illustrative cost using a clearly labelled fake rate of `2 units/call`; distinguish serial latency estimates from measured or concurrent latency.

**Run/payoff:** `cargo run --example l23_sharing` prints `per-agent calls=3 cost=6`, `shared calls=1 cost=2`, `different-context calls=2 cost=4`. Label any timing as simulated; do not assert exact elapsed milliseconds.

**Explain back:** When is reuse semantically valid? How would concurrent calls change wall-clock latency without reducing request count?

### 24. Provider-neutral boundary, failures and stale results (20 min)

**Explain:** Business code should consume application types, not vendor response objects. Freshness refers to the observed snapshot, not merely when a response arrived. Timeouts and errors need an explicit safe display fallback.

**Learner task:** Define `trait Interpreter { fn interpret(&self, snapshot: &Snapshot) -> Result<Envelope, InferenceError>; }` for a synchronous mock. Implement success and error providers without changing decision code. Use logical time for repeatable tests: current snapshot `7`, now `1000 ms`, maximum age `200 ms`; accept age exactly `200`, reject older data, future timestamps and wrong snapshot IDs. Reject unsupported schemas and invalid probabilities. All rejected/error paths yield `Hesitate` plus a reason. Discuss how a real async adapter would preserve this conceptual boundary without promising this exact synchronous trait would suit network I/O.

**Run/payoff:** `cargo test --example l24_boundary` covers fresh, age-boundary, stale, future, mismatched snapshot, malformed payload, timeout and provider-error cases. `cargo run --example l24_boundary` prints `fresh=Run`, `stale=Hesitate`, `provider-error=Hesitate` with reason labels.

**Explain back:** Why can a newly arrived response already be stale? Which checks must be repeated before applying an asynchronous result?

### 25. Replay and optional live request (15 min; live extension optional)

**Explain:** Replay uses recorded inputs and recorded interpretation, not a new stochastic request. Record enough policy, schema and timing context to reproduce a decision; do not promise a live model will reproduce an earlier answer.

**Learner task:** Make a local replay record containing the fake snapshot, deterministic features, response/error, source=`MOCK`, logical times, schema version and policy version. A hardcoded typed record is enough today. Replay it twice through the same pure policy and count provider calls. Add a stale record and a record from an unsupported policy version; explicitly refuse the unsupported version.

**Run/payoff:** `cargo run --example l25_replay` prints `replay1=Run replay2=Run provider_calls=0`, `stale replay=Hesitate`, `unsupported policy rejected`. Tests confirm identical replay decisions and no provider calls.

**Explain back:** Why is rerunning inference not replay? What must be preserved when a policy changes?

**Optional live extension:** Only if credentials and API access are already available, inspect current official docs and have the learner write one minimal request using the documented API. Use fake inputs, keep credentials in environment configuration, and never print or commit secrets. Confirm actual response semantics and adapt them into the local types; label output `source=REAL`. Apply validation, freshness and timeout handling. If Rust access is undocumented, use a documented request mechanism behind the boundary rather than inventing an SDK. Skip this extension if access is unavailable; the mock exercise above still completes this lesson. One real response demonstrates connectivity, not calibration, predictive accuracy or performance guarantees.

## Runnable capstone — assemble only what you have learned (45–60 min)

The learner types `examples/capstone.rs`, reusing their own small functions where helpful. The tutor gives signatures and acceptance tests as prompts, not a finished program. Keep this a terminal exercise with local fixtures; do not add networking, a renderer, live data, order execution, persistent infrastructure or the actual JevFloor project.

```text
Fake Market State
       ↓
Deterministic Rust Features
       ↓
Typed System One Interpretation (explicit MOCK or optional REAL)
       ↓
Deterministic Rust Decision
       ↓
Printed Visual Behaviour: Hesitate / Walk / Run
```

**Learner implementation requirements:**

1. Create `SILVER` tick events with prices `100`, `101`, `103`. Send them over a channel into a `RollingBuffer` of capacity three. On a fourth insertion, evict the oldest item; test this separately. Ensure the channel closes and the program exits.
2. Borrow the final prices as a slice to calculate `delta=3` and `positive_changes=2`. Empty or insufficient history should yield an explicit unavailable outcome, not a panic.
3. Construct snapshot ID `7`, observed time `1000 ms`, schema version `1`. Ask a local provider for a fixture with `p_persistent=0.85`, `p_active=0.90`, `confidence=0.90`. Label it MOCK; these values are invented.
4. Validate and evaluate using lesson 22's policy and lesson 24's freshness rules, with now=`1100 ms` and maximum age=`200 ms`. Keep feature calculation, interpretation acquisition and decision code separate.
5. Print a compact result. Terminal colour or graphics are unnecessary. Add a tiny `Strategy` implementation around the deterministic policy if it helps consolidate lesson 11; avoid an elaborate framework.
6. Provide local scenarios `strong`, `mixed`, `uncertain`, `stale`, `error`, and `empty`, selected by an argument. `mixed` sets `p_persistent=0.65`; `uncertain` sets confidence=`0.40`; `stale` sets now=`1301`; `error` returns a provider error; `empty` supplies no ticks. Preserve other fixture values.
7. Replay one recorded result twice without making provider calls. Keep logical time and policy version explicit.

**Run and observable payoff:**

`cargo run --example capstone -- strong` should print the following values (layout may differ):

```text
source=MOCK symbol=SILVER snapshot=7
prices=[100, 101, 103]
delta=3 positive_changes=2
p_persistent=0.85 p_active=0.90 confidence=0.90
age_ms=100 valid=true
decision=Investigate
visual=Run
```

Use a deterministic display mapping: `Run → Investigate`, `Walk → Observe`, `Hesitate → Wait`. These labels are not order instructions.

The learner personally runs all remaining modes with `cargo run --example capstone -- MODE`:

| Mode | Expected visual | Observable reason |
|---|---|---|
| `mixed` | `Walk` | Valid positive delta; persistence below strong threshold |
| `uncertain` | `Hesitate` | Confidence below threshold |
| `stale` | `Hesitate` | Snapshot age exceeds maximum |
| `error` | `Hesitate` | Provider unavailable |
| `empty` | `Hesitate` | Insufficient data |

`cargo test --example capstone` must pass tests for rolling-buffer eviction, empty input, feature arithmetic, valid strong/mixed policy branches, low confidence, age exactly at the limit, stale data, snapshot mismatch, invalid probability (including NaN), provider failure, and deterministic replay without provider calls. The tutor should ask the learner to choose inputs and write assertions rather than paste a finished test suite.

**Explain-back checkpoint:** Walk through one event's ownership across the channel, one slice borrow during feature calculation, the provider boundary, validation and freshness decisions, and the final deterministic branch. Change one threshold or fixture, predict the consequence, rerun, and explain the observed difference.

## Exit criteria

The learning day is complete when the learner can demonstrate all of these, using their own runnable work:

- Create, check, run and test a Cargo example and locate the relevant compiler diagnostic.
- Explain moves, `Copy`, explicit cloning, shared/exclusive borrowing, vector growth hazards, slices and practical reference lifetimes.
- Handle absent/error values, choose the correct iterator ownership mode, and write a small generic function constrained by a trait.
- Explain `Box`, `Arc`, `Mutex`, `Send`, `Sync`, channel closure and basic async scheduling; run the channel and Tokio payoffs.
- Identify the ECS concepts in the headless Bevy example, or explicitly record that only the fallback analogy ran and Bevy remains to be verified.
- Distinguish deterministic features, typed probabilistic interpretation and deterministic application policy; explain why type safety does not mean truth.
- Describe predefined schemas, separate questions, probability versus confidence, inference reuse conditions and the limits of latency/cost estimates.
- Run fallback, stale-result and replay cases through a provider-neutral local boundary.
- Run every capstone scenario and explain the entire pipeline without the tutor supplying the explanation.

At the end, ask for a two-minute verbal or written architecture explanation, then one small unscripted change implemented and run by the learner. Summarize demonstrated understanding, remaining confusions and any deferred live API/Bevy setup. Do not claim production readiness or begin JevFloor automatically.

## Tutor reference links

Use primary documentation as needed, checking current versions before prescribing dependency-specific APIs:

- [The Rust Book](https://doc.rust-lang.org/book/)
- [Cargo Book](https://doc.rust-lang.org/cargo/)
- [Rust standard library](https://doc.rust-lang.org/std/)
- [Tokio tutorial](https://tokio.rs/tokio/tutorial)
- [Bevy ECS documentation](https://docs.rs/bevy_ecs/latest/bevy_ecs/)
- [TypeSafe AI introduction](https://typesafe.ai/blog/introducing-system-one-models-and-jev)
- [TypeSafe documentation](https://docs.typesafe.ai/)
