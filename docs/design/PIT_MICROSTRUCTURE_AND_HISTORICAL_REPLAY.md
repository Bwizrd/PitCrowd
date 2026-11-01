# OpenCry: Pit Microstructure, Behavioural Model & Historical Replay

> **Status (2026-10-20).** The participant engine described in §3–§5 now exists as
> `opencry-visual/src/pit_engine.rs` and drives both the 2D floor and the 3D pit: per-slot
> two-sided quoting, broker order decks with market/limit orders, block sweeps across several
> locals, human-noise allocation with misses and out-trades, inventory-skewed locals who back
> away in fast markets, a reporter who prints only changes, and derived signals (`sound`,
> `pressure`, `imbalance`, `unfilled_interest`, `vacuum`, `fast_market`). Scenarios are flow
> presets (`preset_for`). **Replay and live data (§6) are deliberately deferred**: Paul's
> direction is to get the pit mechanics believable first, then wire data and Jev.


**Status:** Design, 2026-10-20. Supersedes the mechanics and scenario sections of
`TRADING_PIT_DYNAMICS_AND_SCENARIOS.md` (sections 3 to 6) and
`opencry_authentic_pit_mechanics.md` (sections 4 to 7). Corrections to those
documents are listed in Appendix A. It is consistent with, and fills in, the
product spec's sections 6 (data), 7 (replay), 9 (agents) and 17 (time controls).

**One-line goal:** make the pit a faithful *rendering of an order-flow state
machine*, so that the same engine can be driven by a live feed, a historical
record, or a synthetic generator, and so that "what would the S&P pit have felt
like at 09:30 on 19 October 1987" is a replay you press play on, not a scene
someone hand-animated.

---

## 0. Findings on the current model

These are the gaps between what the code does today and what the vision
documents promise. They are the reasons the floor currently reads as a stage set
rather than a market.

1. **The market is three scalars per pit.** `activity`, `delta` and
   `volume_multiplier`, hand-set by four scripted scenarios. There is no price
   process, no bid, no offer, no last print, no inventory, no order. Nothing a
   trader could be *wrong* about.
2. **Everyone agrees.** Both the 2D gestures and the 3D clips read the single
   pit-wide `delta`, so every trader in a pit signals the same side. A real pit
   is a two-sided auction at every instant. Imbalance showed in *who got filled*,
   *where the print landed*, and *who backed away*, not in a room full of
   identical palms. The one thing the visual must communicate is encoded in the
   one way that never happened.
3. **Choreography is a clock.** The 3D deals are selected by `t % 4.0` and the
   scenario enum. The 2D bursts are random draws gated by `delta`. Neither is
   caused by anything; there is no event a viewer could trace back to a cause.
4. **Six channels encode one variable.** Crowd count, floor glow, label text,
   decibel bars, broker badge and pulpit colour all render `activity × delta`.
   Redundant channels cost attention and carry no extra information. Each
   channel should carry a distinct signal (section 5).
5. **The price never moves.** The pulpit shows `base × (1 + 0.015·delta)`, a
   static function. The wall ticker is a string literal.
6. **The documented information channels are missing.** The literature on pits
   is specific about what carried information: sound level *led* volatility and
   depth withdrawal (Coval & Shumway 2001); brokers on the top step reacted
   first and locals lower down keyed off them; locals pulling bids was the
   visible form of depth evaporating. None of these mechanisms exist in the
   model.
7. **Hand-signal facts are wrong in two documents** (Appendix A). Price and
   quantity conventions are swapped and the tens/hundreds signals are
   misdescribed.
8. **The roster is three different rosters.** Twenty named participants in the
   dynamics document, eighteen unnamed slots in `pit_3d.rs` with comments that
   contradict the document's numbering, and a 2D crowd that assigns roles by
   spawn index.
9. **Replay does not exist**, although the product spec calls it first-class and
   the Jev addendum designs recorded-inference replay around it. There is no
   event bus, no clock abstraction and no data adapter, so every historical
   feature is blocked.
10. **Jev is not available yet** (lesson notes: waitlisted). Every design below
    must run with a deterministic stand-in and accept Jev later without touching
    the pit.

---

## 1. Governing principle

> The pit is a presentation layer for a typed event stream. Every visible or
> audible thing is the consequence of a `PitEvent`. Every `PitEvent` is produced
> by one of three sources behind one interface: a live feed, a historical
> record, or a synthetic generator. The HUD always states which.

Corollaries:

- Agents never move the real price. In live and replay modes the price path is
  exogenous truth; agents are the *explanation* of that path (spec section 9:
  "they do not rewrite history").
- Fidelity is labelled, never implied. Trade-only data is not depth. Bar data is
  reconstructed, and says so (spec section 7).
- Determinism: same events + same seed = same floor, frame for frame.

---

## 2. What actually happened in a pit

This section is the behavioural ground truth the engine models. Where a claim
comes from a source it is cited; where it is a design choice it says so.

### 2.1 Geography is hierarchy

The CBOT and CME pits were octagonal, stepped amphitheatres, and position on the
steps reflected standing. Orders entered via clerks and runners at the rail and
went to brokers on the **top step**, who had the sightline to their desks. Locals
on lower steps could not see the runners; they watched the top-step brokers, and
"if those brokers suddenly become active, brokers further down the steps usually
pick up on that fact and act accordingly." Traders in the same delivery month
stood together on the same step so a broker could find that market quickly
(sources: Appendix B, pit structure).

Design consequence: the pit has an explicit **attention graph**. Information
enters at the rail, hits the top step first, and propagates downward with
latency. The visual ripple down the tiers is not decoration; it *is* the
mechanism.

Working tier assignment for OpenCry (a design choice consistent with the above):

| Tier | Who | Faces | Reacts to |
|---|---|---|---|
| Rail / concourse | Runners, arb clerks, flashers | Desks | Desk orders (external) |
| Top step | Institutional brokers with decks | Outward to desks, then into pit | Runners and flashers |
| Middle steps | Locals, scalpers | Inward, eyes on brokers | Top-step brokers' hands |
| Bottom / well | Heavy market makers, spreaders, front month | Across the well | Everything, slowest to panic |
| Pulpit | Price reporters | The crowd | Fills at new prices |

### 2.2 The auction was two-sided and many-to-one

- Bids and offers had to be shouted and shown; private deals were illegal.
- Convention: a bid is *price then quantity* ("five for ten": bid 5 for 10
  lots); an offer is *quantity then price* ("ten at five": 10 lots offered at
  5). Signal prices showed only the last digit.
- A broker broadcasting size was answered by several locals at once; the broker
  allocated by pointing and eye contact; each party wrote the fill on a card
  with the counterparty's badge; mismatches became **out-trades** resolved later.
- Locals quoted both sides continuously and lived on the edge. After absorbing a
  block they were long and bid up to scratch, which produced the characteristic
  rebound. When a broker's deck looked large they pulled their bids, which is
  what a liquidity vacuum looked like: a broker shouting into silence while the
  price gapped.

### 2.3 Hand signals (CME convention, corrected)

| Meaning | Signal |
|---|---|
| Buying (bidding) | Palms **toward** the body |
| Selling (offering) | Palms **away** from the body |
| Price | Hand held **out in front**; fingers up for 1 to 5, sideways for 6 to 9, closed fist for 0 or "even" |
| Quantity 1 to 9 | Fingers touched to the **chin** |
| Quantity in tens | Fingers touched to the **forehead** |
| Quantity in hundreds | Price signal flashed near the forehead, then a **fist to the forehead** (500 = open hand then fist) |

Conventions varied by exchange; OpenCry standardises on the above and labels it
"CME-style" in the role key. Sources: Appendix B, hand signals.

Design consequence: the UniMate rig already has wrist, fingers and thumb joints.
Quantity at the face and price out front are two more clips, and they are the
*information-bearing* clips. Fractional price fingers remain out of scope, as the
evolution plan says.

### 2.4 Sound led price

Coval & Shumway (Journal of Finance, 2001) measured ambient sound in the CBOT
30-year Treasury pit and found that, controlling for lagged returns, volume and
announcements, rising sound level **preceded** higher volatility, lower depth,
higher transaction costs and the arrival of block trades and customer-driven
flow.

Design consequence: the decibel channel must be driven by *arrival intensity and
unfilled interest*, not by realised volatility. If the sound rises after the
candle moves, the floor is a lagging indicator and the trader's eye rejects it.
This is the single most important mapping to get right.

### 2.5 Big days were not always loud

The S&P 500 pit on 19 October 1987 opened on time with a crowd that was
"smaller than usual" and "subdued". The first print was about 7% down and the
pit stood in shocked silence; some traders left. Portfolio insurers sold more
than 3,000 contracts in the first half hour regardless of what locals did
(Henriques, Financial History 124). The scenario documents assume crashes are
roars. A vacuum is quiet, sparse and slow, and that is the most frightening
state a pit could be in. "The silent crash" is a distinct scenario (section 6.3).

### 2.6 What floor traders actually watched

The brokers' hands, the lean of the crowd toward one side of the pit, the speed
of the runners, and the sound. The wall boards lagged. The engine must therefore
expose *pre-print* state (interest shouted but not yet filled) as a first-class
quantity, because that is what the pit showed that a chart cannot.

---

## 3. The microstructure engine

### 3.1 Event vocabulary

```rust
pub enum PitEvent {
    SessionOpen { pit, ts },
    SessionClose { pit, ts },
    NewsPause { pit, ts, until },                 // scheduled release: the freeze
    OrderArrival { pit, ts, side, qty, limit: Option<Price>, urgency, origin: FirmId },
    Quote { pit, ts, agent, side, price, qty },   // a local shouting a bid or offer
    Withdraw { pit, ts, agent, side },            // a local pulling a quote
    Broadcast { pit, ts, broker, side, price, qty },
    Respond { pit, ts, agent, to: broker, qty },  // many-to-one contestation
    Fill { pit, ts, buyer, seller, qty, price },
    Missed { pit, ts, agent, to: broker },        // responded, not allocated
    Print { pit, ts, price, direction },          // reporter keys a new price
    FastMarket { pit, ts, on: bool },
    OutTrade { pit, ts, a, b },
}
```

Every event carries source timestamp, ingestion timestamp, sequence and a
`Fidelity` tag (`Tick`, `Reconstructed`, `Synthetic`).

### 3.2 Participants

Each participant is an ECS entity with:

| Component | Meaning |
|---|---|
| `Role` | Broker, Local, MarketMaker, Spreader, Runner, Reporter, Official |
| `Tier` | Rail, Top, Middle, Well, Pulpit (drives attention latency) |
| `Inventory` | signed lots, with a soft and hard limit |
| `QuotePolicy` | half-spread, inventory skew, widen-on-toxicity coefficients |
| `Aggressiveness` | propensity to hit or lift rather than quote |
| `Deck` | brokers only: queue of client orders with urgency and size |
| `Attention` | what this agent has seen, with per-tier delay |
| `Fatigue` | reaction time degrades with sustained intensity |
| `Personality` | the roster traits (perma-bear, nervous, contrarian) as parameter offsets |

Locals run an inventory-skewed market-making rule (Avellaneda-Stoikov in spirit:
reservation price shifts against inventory, spread widens with volatility and
with the size visible on brokers' decks). Brokers work their decks with a simple
execution policy: market orders broadcast immediately, limits rest as quotes,
large orders are sliced with urgency-dependent pace. Spreaders quote the spread
between two pits and are the natural carrier of cross-pit contagion.

### 3.3 The shouted book

The pit's book is a list of standing `Quote`s with owner, time and loudness.
Matching is price-time **with human noise**: allocation among responders depends
on proximity to the broadcaster, recent eye contact, and a loudness term. The
noise is what produces `Missed` and `OutTrade` events, and those are what make
the crowd look like people rather than a queue.

### 3.4 Price is exogenous; participants explain it

In live and replay modes the engine receives a real price path and must produce
a sequence of `Fill`s whose prints track it within tolerance (one tick on tick
data, the bar's OHLC envelope on bar data). The **flow inference** layer turns
the real series into `OrderArrival`s; the participant layer then clears them
against the shouted book, with the book's reservation prices anchored to the
real path so the pit cannot drift. Agents may be wrong-footed, long into a drop,
or short into a squeeze, because *their inventory* is free even though the
price is not. That is where the drama comes from, legitimately.

### 3.5 Flow inference tiers

| Tier | Input | Aggressor side | Intensity | Depth | Label |
|---|---|---|---|---|---|
| A | Ticks with bid/ask (live cTrader spot, cTrader tick history) plus DOM where offered | Tick rule against prevailing quote | Tick arrival rate | DOM levels | `Tick` |
| B | OHLCV bars (bridge history, vendor CSV) | Bulk Volume Classification per bar, cross-checked with body-to-range ratio; disagreement widens uncertainty | Volume vs rolling percentile; range expansion | Inferred from spread proxy and the number of locals still quoting | `Reconstructed` |
| C | None | Generator parameters | Generator | Generator | `Synthetic` |

Tier B also needs a **within-bar synthesiser**: a Brownian bridge constrained to
visit the bar's open, high, low and close in a plausible order, producing the
sub-bar timestamps at which arrivals are scheduled. BVC's accuracy is contested
in the literature (Andersen & Bondarenko argue the tick rule beats it where
ticks exist), which is exactly why the tier label matters: Tier B is a
reconstruction, and the HUD says so.

The current four scenarios become **presets of the Tier C generator**: arrival
rate curves, side bias, block-size distribution, news-pause schedule. They stop
being direct writes to a `delta` array.

### 3.6 Flow allocation and the attention cascade

- Block arrivals (heavy-tailed sizes) go to brokers' decks via runners with
  travel time. Background two-way flow goes to locals as quotes.
- When a broker broadcasts, the event reaches each tier after its latency (top
  step immediately, middle step 300 to 800 ms, well 1 to 2 s, modulated by
  fatigue and by sound level, which raises everyone's alertness).
- The visible consequence is the ripple: hands rise on the top step, then heads
  turn on the middle step, then the well leans. This replaces the current
  everyone-at-once burst.

### 3.7 Derived pit signals (the viewer's dashboard, and Jev's input)

| Signal | Definition | What it looks like |
|---|---|---|
| `arrival_intensity` | arrivals per second, percentile vs session | Pace of everything |
| `unfilled_interest` | shouted qty not yet filled, by side | Hands up without endorsing; the vacuum |
| `imbalance` | signed aggressor volume over a short window | Which side's palms dominate *among those getting filled* |
| `participation` | locals currently quoting / locals present | Crowd size and tightness of the ring |
| `depth_proxy` | sum of quoted qty within k ticks of the last print | How close people stand to the broker |
| `inventory_stress` | Σ|inventory| / Σ limits | Agitation, disputes, scratch trades |
| `sound` | f(arrival_intensity, unfilled_interest, participation) | The roar, and the silence |
| `fast_market` | reporter flag when prints outrun keying | Red strobe on the pulpit, officials step in |

These feed the `DecisionModel` (section 7) and are what the HUD shows when a pit
is inspected. They are also what a perceptual test asks the viewer to estimate
(section 6.5).

---

## 4. Behavioural state machines

Timer choreography is replaced by per-role finite state machines driven by
events. The existing `UnimateClip` set is kept as the *output alphabet*; the FSM
chooses the clip.

### Broker

```
Idle(deck empty) --OrderArrival--> Working
Working --decide slice--> Broadcasting(side, price, qty)   clip: Buy/Sell + QuantitySignal
Broadcasting --Respond(s)--> Allocating(targets)            clip: PointDeal, eye-lock to each
Allocating --Fill(s)--> Endorsing                           clip: Endorse
Endorsing --> Reporting(runner dispatched) --> Working | Idle
Broadcasting --no Respond within T--> Dropping price        clip: PriceSignal lower, louder
```

### Local / market maker

```
Scanning --Broadcast seen (after tier latency)--> Deciding
Deciding --inventory and edge ok--> Responding               clip: Buy/Sell counter-shout
Deciding --deck looks big / toxicity high--> BackingAway     clip: new: BackingAway (step back, hands down)
Responding --Fill--> Endorsing --> InventoryManage
Responding --Missed--> Scanning                             clip: hands drop, head shake
InventoryManage --|inventory| > soft--> Scratching           clip: Buy/Sell on own account
Scratching --Fill--> Scanning
any --NewsPause--> Freeze                                   clip: new: Freeze (eyes to board)
any --OutTrade--> Dispute
```

### Reporter

```
Watching --Fill at new price--> Keying                       clip: Pulpit
Keying --prints outrun keying--> FastMarketFlag
```

### Runner

```
AtDesk --OrderArrival(block)--> Sprint(to broker) --> Handoff --> Return
```

New clips needed beyond the current set: `QuantityChin`, `QuantityForehead`,
`PriceOutFront`, `BackingAway`, `Freeze`, `Missed`. Everything else maps to
existing clips.

The many-to-one contestation is explicit: a `Broadcast` triggers k `Respond`s
with staggered latency; the broker allocates a subset; the rest get `Missed`.
Two visibly different outcomes for the same gesture is what makes the crowd
legible.

---

## 5. Sensory mapping

Rule: one channel, one signal. No channel repeats another.

| Channel | Signal | Replaces |
|---|---|---|
| Crowd count in pit | `participation` | activity-driven spawn count |
| Lean of the crowd toward a rail | `unfilled_interest` by side | delta-driven phalanx |
| Palm colour per agent | that agent's side **this instant** | pit-wide delta |
| Hand height per agent | that agent's urgency (market order, deck size) | flash timer |
| Quantity gesture | that agent's qty bucket | none |
| Sound | `sound` (leads price) | decibel bars reading activity |
| Pulpit screen | last `Print`, tick direction, `fast_market` strobe | delta-coloured static price |
| Runner traffic on concourse | institutional `OrderArrival` rate | fixed 8 runners |
| Locals stepping back | `Withdraw` events | none |
| Scrum around a broker | `Fill` burst in progress | none |
| Cross-pit ripple | spreaders' `Broadcast` in the correlated pit within Δt | none |
| Whole-pit freeze | `NewsPause` | none |
| Pit floor glow | reserved for Jev / stand-in confidence only | activity×delta |

The 2D floor overview shows the pit-level signals; the 3D pit shows the agent-
level ones. Both consume the same events.

---

## 6. Historical replay

### 6.1 Architecture

```
MarketDataSource (trait, spec 6.4)
  ├── LiveCTraderSource        ticks + DOM           Tier A
  ├── CTraderTickHistorySource tick history, paged   Tier A (trades+quotes, no depth)
  ├── BridgeBarSource          /getDataFromDB JSON   Tier B
  ├── FileBarSource            CSV / Parquet         Tier B
  └── SyntheticSource          generator presets     Tier C
        │
        ▼
FlowInference ──► PitEvent stream ──► Recorder (append-only log + periodic agent snapshots)
                                   └► ReplayClock (play, pause, 0.25x..10x, step, seek to snapshot)
                                              │
                                              ▼
                                   Participant engine ──► 2D floor, 3D pit, audio, HUD
```

Seek is implemented by restoring the nearest agent snapshot and fast-forwarding
events silently. Recorded-inference mode (Jev addendum section 19) stores the
`DecisionModel` outputs alongside the event log so replays are exact.

### 6.2 Data actually available today

Measured against the local bridge on 2026-10-20:

| Timeframe | Earliest | Note |
|---|---|---|
| 1 minute | 2026-05-06 | Enough for every FOMC, CPI and NFP since May 2026 |
| 1 hour | 2021-04-14 | Covers 2022 rate shock, 2023 bank stress, 2024-2026 events |
| 1 day | 2024-06-02 | Shorter than 1 hour; looks like a retention setting |

No ticks or depth are stored. The cTrader Open API serves tick history on demand
in paged requests (per-request caps, `hasMore` pagination), so Tier A replays of
recent sessions are obtainable without a new vendor. For pre-2021 and for
futures-pit instruments, see 6.3.

### 6.3 Event catalogue

Pits closed progressively: CME shut most futures pits in July 2015 and closed the
floor in March 2020. Anything after 2015 is an explicit counterfactual ("as if
the pit still existed") and the HUD should say so. Earlier events are
reconstructions of a pit that did exist.

| Date | Event | Pit that existed | Best practical data | Tier | What the viewer should feel |
|---|---|---|---|---|---|
| 1980-03-27 | Silver Thursday (Hunt brothers) | COMEX silver ring, NY | Daily only | C, scripted on daily path | Margin-call selling into no bids; ring thins |
| 1987-10-19 | Black Monday | CME S&P 500 pit | CME DataMine Time & Sales (from 1982, paid) or daily | A (trades only) or C | **Quiet.** Small subdued crowd, 7% down on first print, silence, people leaving; brief rally near 11:00 NY, then renewed fall |
| 1992-09-16 | Black Wednesday | IMM currency pit, LIFFE short sterling | Vendor intraday or daily | B or C | Steady one-way desk flow, brokers never idle, locals step aside |
| 2008-09-15 | Lehman | CME S&P pit, CBOT Treasury pit | Vendor 1-minute | B | Attrition: sustained high intensity, inventory stress, disputes |
| 2010-05-06 | Flash Crash | S&P big pit still open | Vendor 1-minute (Databento GLBX starts 2010-06, just misses) | B | Vacuum then violent rebound in minutes; the doc's Scenario 3 on real data |
| 2011-05-01 | Silver collapse | COMEX ring still open | Vendor 1-minute | B | Night session; margin hikes; cascading |
| 2015-01-15 | SNB floor removed | Pits largely gone | cTrader tick history possible | A, counterfactual | Total absence of bids; the purest vacuum on record |
| 2016-06-24 | Brexit | Counterfactual | cTrader tick history | A, counterfactual | Overnight one-way flow, cross-pit contagion GBP to indices |
| 2020-03-09 to 16 | Covid crash | Floor closed 03-13 | 1-hour (bridge from 2021 misses; vendor) | B | Limit-down pauses as `NewsPause`-like freezes |
| 2022-06-15 | 75 bp FOMC | Counterfactual | Bridge 1-hour | B | Freeze then two-way whipsaw; the doc's Scenario 5 on real data |
| 2026-05 onward | Any FOMC, CPI, NFP | Counterfactual | Bridge 1-minute, all nine instruments | B | **First replay to build**: multi-pit, real lead-lag, zero new data cost |

The first milestone is the last row. It needs no purchase, exercises every tier
of the design, and gives cross-pit contagion for free because nine instruments
share one clock.

### 6.4 Replay details that bite

- Session calendars and time zones per pit; pit hours versus 24-hour CFDs.
- Scheduled releases as `NewsPause` events from a calendar, so the freeze happens
  before the move, as it did.
- Multi-pit replay must share one `ReplayClock`; lead-lag emerges from data, not
  from a contagion animation.
- Bar-reconstructed replays must never render sub-bar detail with more
  confidence than the data has: show a faint "reconstructed" watermark in 3D.

### 6.5 Validation

Three tests, pre-registered the way the live strategies are:

1. **Tape fidelity (unit test).** Over a replay, the pit's `Print`s reproduce
   every bar's OHLC within one tick, and total filled volume matches bar volume
   within 5%.
2. **Behavioural fidelity (checklist).** For each catalogued event, a short list
   of documented qualitative facts the replay must show. For 1987: crowd below
   normal, first print far below prior close, sound below baseline for the first
   minutes, net exodus, rally and fade near 11:00 NY.
3. **Perceptual test (the one that matters).** Blind A/B. Show ten seconds of
   pit with the chart hidden; ask for direction, intensity bucket and regime;
   compare accuracy and response time against the same ten seconds as a DOM
   ladder and as a candle chart. The floor earns its place only if it wins or
   ties on accuracy and wins clearly on time. Run it on yourself first, then on
   anyone who will sit still for twenty minutes.

---

## 7. Jev's place

Jev is waitlisted, so the boundary is built first and filled later.

- `DecisionModel` trait (Jev addendum section 6) with a
  `DeterministicDecisionModel` that maps the section 3.7 signals to the same
  `MarketInterpretation` type Jev will return.
- Jev's outputs modulate **behavioural parameters**, never positions: locals'
  quote width and hesitation, phalanx coherence, reaction latency, the
  probability a local backs away. The pit floor glow is reserved for model
  confidence so the viewer can see when the model is unsure.
- Recorded-inference mode persists observation hash, model version and typed
  output beside the event log; replays are exact. Re-inference mode is for
  model comparison and is labelled as such.

---

## 8. Roadmap mapped to the existing code

Each step is demoable on its own and leaves the visuals working.

1. **Event bus and clock.** New `events.rs` with `PitEvent`, `Fidelity`,
   `MarketClock`. `MarketActivitySim` becomes a consumer that derives the
   section 3.7 signals from events. The four scenarios become
   `SyntheticSource` presets. Visuals unchanged, now caused.
2. **Bar replay.** `flow_inference.rs` (BVC plus body-to-range, intensity
   percentiles, OHLC-constrained bridge), `BridgeBarSource` reading the existing
   `/getDataFromDB`, `ReplayClock` with play, pause, speed, step, seek. HUD time
   controls. **Demo: the most recent FOMC day at 1-minute across all nine pits.**
3. **Participant engine.** Decks, inventory market making, shouted book,
   allocation with noise, FSMs from section 4. Two-sided palms. One roster table
   shared by 2D and 3D. Crowd count from `participation`.
4. **Sound.** Layered crowd beds driven by `sound`, plus silence for `NewsPause`
   and vacuum. Verify in a replay that sound rises before the bar moves.
5. **Tick tier.** `CTraderTickHistorySource` with paging, then
   `LiveCTraderSource` with DOM. Fidelity label flips to `Tick`.
6. **Event catalogue.** Presets with calendars, narrative overlays stating what
   to notice, and the behavioural checklists from 6.5.
7. **Perceptual harness.** The A/B test as a mode of the app with logging.

Steps 1 and 2 are two to three weeks of focused work on top of the current code
and do not depend on Jev, on new data, or on finishing the 3D rig.

---

## Appendix A: Corrections to existing documents

| Document | Claim | Correction |
|---|---|---|
| `TRADING_PIT_DYNAMICS_AND_SCENARIOS.md` 3, `opencry_authentic_pit_mechanics.md` 4 | Quantity 1 to 5 shown with fingers vertical; 10 = index finger to chin or fist to chest; price fractions horizontal | Price is shown out in front (up for 1 to 5, sideways for 6 to 9, fist for 0). Quantity is shown at the face: chin for units, forehead for tens, fist to forehead for hundreds |
| `TRADING_PIT_DYNAMICS_AND_SCENARIOS.md` Scenario 5 | Price reporter "stands on his desk stamping his foot to establish pit priority and clear the dispute" | Reporters recorded prices. Disputes were resolved between the parties and by floor officials or the pit committee. Give this action to an Official |
| Both scenario documents | Crashes are described as roars at 90 to 110 dB | 1987 shows a crash pit can be small, subdued and silent. Add "the silent crash" as its own scenario. Decibel figures are unsourced; treat as relative levels |
| `TRADING_PIT_DYNAMICS_AND_SCENARIOS.md` 2 vs `pit_3d.rs` | Twenty named participants with IDs; code has eighteen slots and comments that use different IDs for the same names | One roster table in code, read by both views |
| `opencry_market_data_strategy.md` 6 | "The data problem is solved" | True for live Tier A. Stored history is bars only (section 6.2). Pre-2021 and futures-pit instruments need a vendor: CME DataMine Time & Sales from 1982, Databento Globex from June 2010 |
| `opencry_jev_realtime_feel_and_architecture.md`, `opencry_the_gallery_vision.md` | "1930s stock exchange" | The mechanics modelled are Chicago futures pits, roughly 1980 to 2015. NYSE posts with specialists worked differently. Pick the era and say it |
| `opencry_pit_evolution_plan.md` 2 | Exclude fractional finger codes | Agree. Keep quantity signals (chin, forehead, fist) because size is the information |
| `opencry_pit_evolution_plan.md` 2 | Out-trades have no modern equivalent | They do (trade busts, self-match prevention). Still fine to omit visually, but it is a modelling choice, not a fact |

## Appendix B: Sources

Pit structure and hierarchy:
- Manifold / University of Minnesota Press, sonic practices of finance (CBOT pit as stepped octagon, position reflects power): https://manifold.umn.edu/read/d2d2b7fa-0974-45f3-879c-f1276c6e449e/section/2613f2ee-0364-4fdb-b9ae-c163c2e79204
- Simulated pit trading system patent (top-step brokers see runners; lower steps key off them; same delivery month on the same step): https://patents.google.com/patent/WO1990010910A1
- Daily Speculations, cliques around brokers (anecdotal): https://dailyspeculations.com/wordpress/?p=11849

Hand signals:
- Wikipedia, hand signaling in open outcry: https://en.wikipedia.org/wiki/Hand_signaling_(open_outcry)
- StoneX, trading floor hand signals: https://futures.stonex.com/blog/trading-floor-hand-signals-the-sign-language-of-futures-trading and https://futures.stonex.com/futures-options-101/hand-signals

Sound as information:
- Coval & Shumway, "Is Sound Just Noise?", Journal of Finance 56(5), 2001: https://ideas.repec.org/a/bla/jfinan/v56y2001i5p1887-1910.html and https://deepblue.lib.umich.edu/bitstream/handle/2027.42/71876/0022-1082.00393.pdf

19 October 1987:
- Henriques, Financial History 124 (Winter 2018), pp. 24 to 26: https://fhmagazine.org/financial-history-issue-124-winter-2018/0695841001519735985/p24
- CFTC final staff report on the October 1987 market break: https://test.sechistorical.org/collection/papers/1980/1988_0201_CFTCMarketBreak.pdf

Historical data:
- CME DataMine FAQ, Time & Sales from 1 January 1982, depth from 2005: https://cmegroupclientsite.atlassian.net/wiki/x/AYD4H
- Databento GLBX.MDP3, coverage since June 2010: https://databento.com/datasets/GLBX.MDP3/futures/GC
- cTrader Open API tick history limits and paging: https://community.ctrader.com/forum/connect-api-support/32156/ and https://docs.spotware.com/reporting-api/historical-tick-data

Flow inference:
- Easley, López de Prado, O'Hara, "Discerning Information from Trade Data" (bulk volume classification): https://papers.ssrn.com/abstract=1989555
- Andersen & Bondarenko, critique of BVC and VPIN: https://repec.econ.au.dk/repec/creates/rp/13/rp13_43.pdf
