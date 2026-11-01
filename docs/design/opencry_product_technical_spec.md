# OpenCry --- Product & Technical Specification

**Working title:** OpenCry\
**Tagline:** *See the market move.*\
**Primary implementation:** Rust + Bevy\
**Document purpose:** Implementation handover for a coding agent\
**Status:** Initial product/architecture specification

------------------------------------------------------------------------

## 1. Executive Summary

OpenCry is a real-time, isometric market visualisation and
agent-simulation application inspired by historical open-outcry trading
floors.

Instead of presenting financial markets primarily as candlestick charts,
OpenCry turns multiple instruments into a living spatial environment.
Each instrument occupies its own trading pit on a large exchange floor.
Market activity is represented through crowds, movement, aggression,
congestion, trader behaviour, and other visual signals.

At the highest zoom level, the user should be able to survey the market
as a whole and immediately notice where activity is concentrated. For
example:

-   Silver may have a large, frantic crowd and rapidly changing
    activity.
-   GBP/USD may be comparatively quiet.
-   Gold may suddenly become active shortly after the US Dollar Index
    moves.
-   Equity-index pits may collectively become more volatile.
-   Related instruments may begin exhibiting unusual relationships.

The user can zoom progressively from the entire exchange, into an
individual pit, into brokerage firms, and ultimately into individual
simulated traders.

OpenCry combines four ideas:

1.  **Real market visualisation** --- live and historical market data
    drive the state of the world.
2.  **Agent-based simulation** --- fictional brokerage firms and traders
    make independent decisions against the same real market.
3.  **AI-assisted agents** --- fast probabilistic AI models such as Jev
    can make bounded, typed decisions for selected agents.
4.  **Research discovery** --- slower analytical/AI processes can
    propose new strategy hypotheses, which must then pass deterministic
    statistical testing before being surfaced as credible research.

The application is both:

-   a substantial Rust/Bevy engineering portfolio project; and
-   an experiment in whether spatial, behavioural visualisation can help
    a human trader develop better cross-market intuition.

OpenCry must never imply that simulated traders correspond to
identifiable real market participants unless the underlying data
genuinely provides that information. Agents are visual and analytical
abstractions derived from aggregate market data.

------------------------------------------------------------------------

# 2. Product Goals

## 2.1 Primary goals

OpenCry should:

-   Visualise many financial instruments simultaneously as individual
    trading pits.
-   Allow seamless isometric zooming from whole-market view to
    individual-agent view.
-   Convert abstract market information into intuitive visual behaviour.
-   Consume real-time market data.
-   Record market events for deterministic replay.
-   Support historical market sessions.
-   Run autonomous simulated brokerage firms against real market
    conditions.
-   Track positions, trades, risk, and P&L for simulated firms and
    traders.
-   Support multiple trading philosophies and strategies.
-   Provide a leaderboard of simulated brokerage performance.
-   Allow selected AI-driven traders to make bounded probabilistic
    decisions.
-   Detect potentially interesting cross-market patterns.
-   Generate research hypotheses without presenting unvalidated
    discoveries as trading edges.
-   Backtest proposed hypotheses deterministically.
-   Clearly separate in-sample discovery from out-of-sample validation.
-   Be performant enough to demonstrate why Rust is appropriate.
-   Be visually compelling enough to demonstrate in a short portfolio
    video.

## 2.2 Secondary goals

-   Make historical market sessions enjoyable to explore.
-   Allow accelerated replay.
-   Allow pausing and frame/time stepping.
-   Enable strategy tournaments against identical historical sessions.
-   Support simulation experiments with reproducible seeds.
-   Expose useful diagnostics and performance metrics.
-   Provide a clean modular architecture suitable for open-source
    development.

## 2.3 Non-goals for the initial version

The first versions should NOT attempt to:

-   execute real-money trades;
-   become a brokerage platform;
-   perfectly reconstruct historical open-outcry behaviour;
-   infer identities of real market participants;
-   create realistic 3D human models;
-   support every asset class or data vendor;
-   use generative AI for every simulated trader;
-   automatically deploy discovered strategies to live trading;
-   claim predictive power without statistical validation.

------------------------------------------------------------------------

# 3. Core Product Concept

## 3.1 The exchange floor

The application presents a large isometric exchange floor containing
multiple pits.

Example instruments:

### Precious metals

-   Gold / XAUUSD
-   Silver / XAGUSD

### Foreign exchange

-   GBP/USD
-   EUR/USD
-   USD/JPY
-   AUD/USD
-   USD/CAD

### Dollar / rates

-   US Dollar Index
-   selected Treasury instruments

### Equity indices

-   S&P 500
-   Nasdaq 100
-   Dow Jones
-   FTSE 100
-   DAX

### Energy

-   Crude Oil
-   Natural Gas

### Crypto

-   BTC/USD
-   ETH/USD

The exact set must be configuration-driven.

Each instrument has its own pit with a visible label and market status.

## 3.2 Whole-market view

At maximum zoom-out, the application should behave like a financial
"city".

The user should be able to notice:

-   which pits are busiest;
-   which are quiet;
-   where volatility is expanding;
-   where activity has suddenly changed;
-   which asset classes are collectively moving;
-   where unusual cross-market behaviour may be emerging.

Numbers should be deliberately minimised at this level. The goal is
pre-attentive visual recognition.

## 3.3 Pit view

Zooming into an instrument reveals:

-   buyer/seller distribution;
-   current price;
-   important price levels;
-   relative activity;
-   simulated brokerage participation;
-   aggressive versus passive behaviour;
-   selected order-flow/liquidity representations where supported by
    source data;
-   active strategies;
-   local P&L;
-   notable research signals.

## 3.4 Trader view

At close zoom, an individual simulated trader can be selected.

Example:

    Name: Tom
    Firm: Atlas Securities
    Role: Momentum Trader
    Instrument: XAGUSD
    Position: SHORT
    Entry: 42.17
    Current: 42.08
    Unrealised P&L: +0.9R
    Strategy: Momentum Exhaustion
    Decision confidence: 0.78

The UI should also be able to explain the bounded input features that
led to the current simulated action.

------------------------------------------------------------------------

# 4. Visual Language

A fundamental design principle is:

> Do not merely display market information when the world itself can
> behave like the information.

The simulation should develop a consistent visual vocabulary.

  -----------------------------------------------------------------------
  Market concept                      Possible visual representation
  ----------------------------------- -----------------------------------
  Volume/activity                     Crowd size

  Volatility                          Speed/intensity of movement

  Passive liquidity                   Traders holding/occupying an area

  Aggressive buying                   Buyers rapidly advancing

  Aggressive selling                  Sellers rapidly advancing

  Liquidity withdrawal                Crowd dispersing

  Absorption                          Large group holding despite
                                      repeated pressure

  Level failure                       Defending crowd breaks/disperses

  Uncertainty                         Hesitant movement / reduced
                                      animation conviction

  AI confidence                       Strength/urgency of agent animation

  Strong correlation                  Optional visual relationship
                                      between pits

  Market regime                       Ambient floor behaviour

  Firm performance                    Firm status / leaderboard / subtle
                                      visual prestige
  -----------------------------------------------------------------------

These mappings must be configurable and must never claim more precision
than the source data supports.

------------------------------------------------------------------------

# 5. Zoom and Level-of-Detail Model

The application should have at least four semantic zoom levels.

## Level 1 --- Exchange

Shows:

-   all pits;
-   pit activity;
-   broad direction;
-   asset-class groupings;
-   notable anomalies;
-   research notifications.

Avoid individual trader simulation rendering at full fidelity.

## Level 2 --- Pit

Shows:

-   instrument details;
-   price movement;
-   groups of traders;
-   firm colours/identities;
-   buyer/seller behaviour;
-   key market-state overlays.

## Level 3 --- Brokerage / strategy

Shows:

-   traders belonging to selected firm;
-   current firm exposure;
-   strategy allocation;
-   P&L;
-   active decisions;
-   recent trades.

Other firms may fade visually.

## Level 4 --- Individual trader

Shows:

-   current state;
-   strategy;
-   position;
-   risk;
-   recent decisions;
-   confidence;
-   bounded reasoning/features;
-   decision history.

## Simulation LOD

Rendering LOD and simulation LOD should be independent.

When zoomed out:

-   do not animate every off-screen agent at full resolution;
-   aggregate animation where appropriate;
-   maintain financial/strategy state accurately;
-   reduce visual/pathfinding update frequency;
-   use batched representations.

This should become one of the project's explicit Rust/Bevy performance
challenges.

------------------------------------------------------------------------

# 6. Market Data Architecture

## 6.1 Data flow

    External Market Feed
            |
            v
    Market Data Adapter
            |
            v
    Normalisation Layer
            |
            v
    Canonical Market Events
            |
       +----+----+
       |         |
       v         v
    Recorder   Live World
       |         |
       v         v
    Replay     Simulation

## 6.2 Canonical events

Define strongly typed Rust events such as:

``` rust
pub enum MarketEvent {
    Trade(TradeEvent),
    Quote(QuoteEvent),
    BookUpdate(BookUpdateEvent),
    Candle(CandleEvent),
    Session(SessionEvent),
}
```

All external vendors must be converted into canonical internal types
before reaching the simulation.

## 6.3 Time

Every event must contain:

-   exchange/source timestamp where available;
-   ingestion timestamp;
-   instrument identifier;
-   source identifier;
-   sequence information where available.

Time handling must be deterministic during replay.

## 6.4 Data-source abstraction

Create a trait similar to:

``` rust
#[async_trait]
pub trait MarketDataSource {
    async fn connect(&mut self) -> Result<()>;
    async fn next_event(&mut self) -> Result<MarketEvent>;
}
```

Implement adapters independently.

The MVP can begin with one easily accessible live feed.

------------------------------------------------------------------------

# 7. Deterministic Recording and Replay

Replay is a first-class feature, not an afterthought.

## Requirements

The system must be able to:

-   record canonical market events;
-   reload a recorded session;
-   reproduce event ordering;
-   replay at 0.25x, 0.5x, 1x, 2x, 5x, 10x and higher speeds;
-   pause;
-   resume;
-   step through time;
-   seek where practical;
-   reset simulation agents;
-   replay the same market session with the same random seed;
-   replay with a different seed;
-   compare strategy configurations against identical market conditions.

A deterministic simulation seed should be stored with each experiment.

Historical replay fidelity must be labelled according to available data.
Trade-only data must not be presented as a complete historical
order-book reconstruction.

------------------------------------------------------------------------

# 8. Brokerage Firms

Brokerages are fictional organisations operating inside the simulation.

Example firms:

-   Atlas Securities
-   Redwood Capital
-   Northstar Markets
-   Greyfriars Trading
-   Valkyrie Capital

Names should ultimately be configurable.

Each firm contains:

-   capital;
-   risk limits;
-   strategy allocations;
-   traders;
-   research state;
-   positions;
-   realised P&L;
-   unrealised P&L;
-   performance history.

Example:

    Atlas Securities
      120 Momentum traders
       80 Mean-reversion traders
       40 Order-flow traders
       30 Cross-market traders
       20 Market makers
       10 AI discretionary traders

The actual number of agents should scale with performance targets and
LOD.

------------------------------------------------------------------------

# 9. Trader / Agent Model

Each trader is an ECS entity.

Potential components:

``` rust
struct Trader;
struct FirmId(Entity);
struct InstrumentId(...);
struct StrategyId(...);
struct Position(...);
struct RiskState(...);
struct DecisionState(...);
struct Confidence(f32);
struct AgentPersonality(...);
struct AnimationState(...);
struct WorldPosition(...);
```

Agents should not be implemented as deeply nested object hierarchies.

Prefer data-oriented ECS components and systems.

## Agent categories

### Deterministic strategy agents

Rule-driven.

### Quantitative agents

Use mathematical/statistical models.

### AI agents

Use Jev or another bounded probabilistic decision model.

### Behavioural agents

May model hesitation, trend-following, crowd-following, panic, etc.

### Market-making agents

Operate under separate quoting/inventory rules.

The market price remains driven by the real external market feed in the
core visualisation mode. Simulated agents respond to the real market;
they do not rewrite history.

A separate synthetic-market sandbox may be considered later.

------------------------------------------------------------------------

# 10. Strategy Interface

Strategies should implement a common abstraction.

Conceptually:

``` rust
pub trait Strategy {
    fn evaluate(
        &mut self,
        context: &MarketContext,
        portfolio: &PortfolioState,
    ) -> Decision;
}
```

Example decision:

``` rust
pub enum Decision {
    Buy(OrderIntent),
    Sell(OrderIntent),
    Hold,
    Reduce(PositionAdjustment),
    Exit,
}
```

Strategies should be independently testable without Bevy rendering.

------------------------------------------------------------------------

# 11. Jev / Fast AI Integration

Jev is intended as an optional fast decision layer for selected agents.

The integration should be behind an interface so OpenCry is not tightly
coupled to one AI provider.

Example:

``` rust
pub trait AgentDecisionModel {
    fn decide(&self, state: &AgentObservation) -> AgentDecision;
}
```

## 11.1 Bounded output

AI agents must not return arbitrary instructions.

Use a predefined output type:

``` rust
pub struct ProbabilisticDecision {
    pub buy: f32,
    pub sell: f32,
    pub hold: f32,
    pub reduce: f32,
    pub exit: f32,
}
```

or an equivalent typed schema supported by the model.

## 11.2 Agent observation

Inputs may include:

-   recent returns;
-   volatility;
-   volume percentile;
-   spread;
-   order-flow features where available;
-   related-instrument movement;
-   current position;
-   firm risk state;
-   trader-specific strategy context;
-   P&L;
-   market regime features.

Do not send the entire world state unnecessarily.

## 11.3 Confidence visualisation

Confidence should influence visual behaviour.

Example:

-   BUY 0.51 -\> hesitant movement;
-   BUY 0.75 -\> decisive movement;
-   BUY 0.97 -\> highly aggressive movement.

This visualisation must remain an encoding of simulated model
confidence, not a claim of real-world certainty.

## 11.4 Failure behaviour

If AI inference:

-   times out;
-   returns invalid output;
-   becomes unavailable;
-   exceeds budget;

the agent must degrade safely to a deterministic fallback or HOLD.

The simulation must never stall waiting indefinitely for an AI response.

------------------------------------------------------------------------

# 12. System One / System Two Architecture

OpenCry should deliberately separate fast reactive decisions from slower
research reasoning.

## System One

Purpose:

-   immediate agent decisions;
-   frequent evaluation;
-   typed bounded output;
-   low latency.

Possible implementation:

-   Jev;
-   deterministic strategies;
-   lightweight local models.

## System Two

Purpose:

-   analyse sessions;
-   identify anomalies;
-   generate hypotheses;
-   explain patterns;
-   propose research experiments.

Possible implementation:

-   conventional reasoning LLM;
-   statistical analysis engine;
-   offline batch jobs.

System Two must not directly turn hypotheses into trusted live
strategies.

------------------------------------------------------------------------

# 13. AI Research Department

Each brokerage may eventually have a research department.

Research can analyse:

-   firm performance;
-   strategy performance;
-   instrument behaviour;
-   cross-market relationships;
-   market regimes;
-   failed trades;
-   successful trades;
-   unusual activity.

Example generated hypothesis:

> Silver volatility expansion may persist more frequently when Gold
> momentum is positive, DXY is falling, and Silver volume is above its
> 90th percentile.

The research engine must convert this into a machine-testable
experiment.

------------------------------------------------------------------------

# 14. Research Validation Pipeline

This is a critical requirement.

OpenCry must distinguish:

    Observation
        ->
    Hypothesis
        ->
    Formal Rule
        ->
    Historical Test
        ->
    In-Sample Results
        ->
    Out-of-Sample Test
        ->
    Paper Observation
        ->
    Candidate Strategy

AI-generated hypotheses must not be displayed as validated trading
edges.

## Minimum research output

For a tested hypothesis show:

-   hypothesis description;
-   exact rule;
-   instruments;
-   date range;
-   number of observations;
-   win rate where relevant;
-   expectancy;
-   average R;
-   maximum drawdown;
-   in-sample results;
-   out-of-sample results;
-   transaction-cost assumptions;
-   confidence intervals where appropriate;
-   known limitations.

Avoid optimising repeatedly against the out-of-sample period.

Store experiment provenance.

------------------------------------------------------------------------

# 15. Strategy Evolution

A later milestone may allow firms to adapt.

Example:

-   Momentum strategies perform well in high-volatility commodities.
-   Firm increases future capital allocation slightly.
-   Breakout strategy performs badly in low-volatility FX.
-   Firm reduces allocation.

Evolution must be constrained.

Prefer explicit allocation rules over uncontrolled model
self-modification.

Every allocation change should be recorded as an event so it can be
reproduced during replay.

------------------------------------------------------------------------

# 16. Cross-Market Intelligence

One of OpenCry's most important differentiators is simultaneous
multi-market representation.

The system should compute rolling relationships such as:

-   correlation;
-   relative volatility;
-   lead/lag candidates;
-   relative volume/activity;
-   divergence from recent relationships.

Potential example:

    DXY activity rises sharply
        ->
    EUR/USD responds
        ->
    GBP/USD responds
        ->
    Gold responds
        ->
    Silver does not

The application may flag the discrepancy as something worth
investigating.

It must not automatically interpret every divergence as a trade.

------------------------------------------------------------------------

# 17. User Interaction

## Camera

Support:

-   pan;
-   zoom;
-   smooth semantic zoom;
-   select pit;
-   select firm;
-   select trader;
-   return to whole-market view.

## Filters

Examples:

-   show all agents;
-   AI agents only;
-   selected firm;
-   selected strategy;
-   winners;
-   losers;
-   active positions;
-   high-confidence decisions.

## Time controls

For replay:

-   play;
-   pause;
-   speed;
-   step;
-   restart;
-   timestamp;
-   jump to notable event.

------------------------------------------------------------------------

# 18. Leaderboards

## Firm leaderboard

Show:

-   rank;
-   firm;
-   daily P&L;
-   cumulative P&L;
-   R;
-   drawdown;
-   risk-adjusted metrics where appropriate.

## Strategy leaderboard

Show performance by:

-   strategy;
-   instrument;
-   regime;
-   date range.

## Trader leaderboard

Optional and primarily for game-like engagement.

Leaderboards must not incentivise agents to take unlimited risk. Ranking
should include or constrain risk.

------------------------------------------------------------------------

# 19. Suggested Rust Workspace

``` text
opencry/
|
|-- Cargo.toml
|
|-- crates/
|   |
|   |-- market-types/
|   |   `-- canonical domain types
|   |
|   |-- market-data/
|   |   `-- external feeds and adapters
|   |
|   |-- market-core/
|   |   `-- state, features, instruments
|   |
|   |-- recorder/
|   |   `-- event persistence
|   |
|   |-- replay/
|   |   `-- deterministic playback
|   |
|   |-- simulation/
|   |   `-- agents, firms, positions
|   |
|   |-- strategies/
|   |   `-- deterministic strategies
|   |
|   |-- ai-agents/
|   |   `-- Jev / model abstractions
|   |
|   |-- research/
|   |   `-- hypothesis and backtesting
|   |
|   |-- analytics/
|   |   `-- cross-market statistics
|   |
|   `-- world/
|       `-- mapping simulation to spatial world
|
|-- apps/
|   |
|   |-- opencry/
|   |   `-- Bevy desktop application
|   |
|   |-- recorder-cli/
|   |   `-- headless market recorder
|   |
|   |-- replay-cli/
|   |   `-- headless deterministic replay
|   |
|   `-- research-cli/
|       `-- experiments/backtests
|
|-- assets/
|-- config/
|-- recordings/
|-- benches/
|-- tests/
`-- docs/
```

Keep financial/simulation logic outside the rendering application
wherever practical.

------------------------------------------------------------------------

# 20. Concurrency Architecture

Rust should be used deliberately, not merely as the implementation
language.

Potential runtime domains:

### Async ingestion

Tokio tasks receive market feeds.

### Canonical event bus

Normalised events move through bounded channels.

### Simulation

Bevy ECS systems update agent state.

### Analytics

Parallel computations calculate market features.

### AI inference

Dedicated bounded worker pool / async tasks.

### Recorder

Independent persistence writer.

Avoid a global `Arc<Mutex<World>>`.

Prefer:

-   message passing;
-   immutable snapshots;
-   ECS ownership;
-   narrowly scoped locks;
-   bounded queues;
-   backpressure.

Instrument queue sizes and dropped-event counts.

------------------------------------------------------------------------

# 21. Rust Learning Objectives

The project should intentionally exercise:

-   ownership;
-   borrowing;
-   lifetimes where appropriate;
-   enums and exhaustive matching;
-   traits;
-   generics;
-   error handling;
-   `Result`;
-   `Option`;
-   iterators;
-   async/await;
-   Tokio;
-   channels;
-   `Arc`;
-   locks only where justified;
-   Send/Sync;
-   ECS;
-   data-oriented design;
-   profiling;
-   memory allocation analysis;
-   benchmarking;
-   serialisation;
-   deterministic state;
-   property-based testing.

Avoid `unsafe` initially.

If `unsafe` is eventually introduced for measured performance reasons,
require:

-   benchmark evidence;
-   documented invariant;
-   isolated module;
-   tests;
-   explanation in README.

------------------------------------------------------------------------

# 22. Performance Targets

Initial targets should be treated as engineering objectives rather than
promises.

## MVP

-   1 instrument
-   100 agents
-   60 FPS on target development machine
-   live feed
-   recording
-   replay

## Intermediate

-   10 instruments
-   2,000 agents
-   stable interactive frame rate
-   multiple firms
-   multiple strategies

## Stretch

-   20+ instruments
-   10,000+ financial agents
-   aggressive rendering LOD
-   deterministic replay
-   concurrent analytics
-   selected AI agents

## Experimental

Investigate 100,000 logical agents while using visual aggregation and
simulation LOD.

Benchmark:

-   frame time;
-   simulation tick time;
-   event throughput;
-   allocations;
-   memory;
-   AI latency;
-   channel backlog.

------------------------------------------------------------------------

# 23. Rendering Direction

Use Bevy.

Initial graphics should be intentionally simple.

## Phase 1

-   isometric/grid floor;
-   circles/capsules for traders;
-   coloured firms;
-   labelled pits;
-   simple movement.

## Phase 2

-   stylised 2D/2.5D cartoon traders;
-   firm jackets;
-   gestures;
-   running/walking/idle states;
-   pit architecture.

## Phase 3

-   richer animation;
-   environmental effects;
-   crowd behaviour;
-   brokerage areas;
-   improved camera;
-   polished UI.

Do not block the simulation architecture on artwork.

------------------------------------------------------------------------

# 24. Market-to-World Mapping

Create a dedicated mapping layer.

Do NOT let rendering systems directly interpret vendor data.

Example:

    Vendor event
       ->
    Canonical market event
       ->
    Market state/features
       ->
    Visual semantic event
       ->
    ECS animation/state

Potential visual semantic events:

``` rust
enum VisualMarketEvent {
    ActivityExpanded { instrument: InstrumentId, strength: f32 },
    AggressiveBuying { instrument: InstrumentId, strength: f32 },
    AggressiveSelling { instrument: InstrumentId, strength: f32 },
    LiquidityAppeared { instrument: InstrumentId, level: Price, strength: f32 },
    LiquidityWithdrawn { instrument: InstrumentId, level: Price },
    VolatilityExpanded { instrument: InstrumentId, strength: f32 },
}
```

This decoupling allows the visual language to evolve independently.

------------------------------------------------------------------------

# 25. Persistence

Persist:

-   market recordings;
-   simulation configuration;
-   seeds;
-   firm definitions;
-   strategy versions;
-   experiment definitions;
-   research results;
-   leaderboard history;
-   AI model/version metadata;
-   decision logs where enabled.

Use versioned schemas.

Recorded sessions must remain readable after reasonable application
evolution.

------------------------------------------------------------------------

# 26. Configuration

Prefer external configuration for:

-   instruments;
-   pit locations;
-   firm definitions;
-   strategy allocation;
-   risk limits;
-   data sources;
-   simulation speed;
-   visual mappings;
-   AI enablement;
-   AI rate limits;
-   recording options.

Avoid recompilation for routine world changes.

------------------------------------------------------------------------

# 27. Observability

Add engineering telemetry from the beginning.

Track:

-   FPS;
-   ECS system timings;
-   market events/sec;
-   event queue depth;
-   dropped events;
-   recorder latency;
-   replay drift;
-   AI request latency;
-   AI timeout rate;
-   AI cost/request count if applicable;
-   active entities;
-   memory usage;
-   simulation tick duration.

Provide a developer diagnostics overlay.

------------------------------------------------------------------------

# 28. Testing Strategy

## Unit tests

Test:

-   market normalisation;
-   P&L calculations;
-   risk calculations;
-   strategies;
-   feature calculations;
-   research metrics;
-   decision validation.

## Determinism tests

Given:

-   identical market recording;
-   identical configuration;
-   identical seed;

the simulation should produce identical financial outcomes.

Visual frame-perfect determinism is not required unless explicitly
needed.

## Property tests

Useful for:

-   P&L invariants;
-   position accounting;
-   order state;
-   probability normalisation;
-   replay ordering.

## Integration tests

Test:

-   feed -\> canonical event;
-   event -\> recording;
-   recording -\> replay;
-   replay -\> simulation;
-   strategy -\> position -\> P&L;
-   AI timeout -\> fallback.

## Benchmarks

Use Criterion or equivalent for hot paths.

------------------------------------------------------------------------

# 29. Safety / Analytical Integrity

Because OpenCry may be used alongside discretionary trading, analytical
integrity is a product requirement.

The application should clearly distinguish:

-   observed market data;
-   derived metrics;
-   simulated agent behaviour;
-   AI opinion;
-   research hypothesis;
-   statistically tested result.

Never visually imply that simulated crowds represent actual institutions
unless supported by data.

Never silently use future information in historical strategy decisions.

Prevent look-ahead bias.

Record transaction-cost assumptions.

Research reports should disclose sample sizes.

Out-of-sample data should not be repeatedly tuned against.

AI-generated strategy ideas should be treated as hypotheses, not facts.

------------------------------------------------------------------------

# 30. MVP Definition

The first genuinely usable milestone should be deliberately constrained.

## MVP scope

One instrument: e.g. BTC/USD or another feed that is simple to obtain.

Features:

1.  Connect to live market data.
2.  Convert input into canonical events.
3.  Display one isometric pit.
4.  Render 100 simple agents.
5.  Create 3 fictional brokerage firms.
6.  Implement 3 deterministic strategies.
7.  Agents react to market conditions.
8.  Track simulated positions.
9.  Track P&L.
10. Display firm leaderboard.
11. Record market events.
12. Replay recorded session.
13. Camera zoom/pan.
14. Click trader to inspect state.
15. Maintain 60 FPS target.

No Jev is required for MVP.

The purpose is to prove the entire vertical slice.

------------------------------------------------------------------------

# 31. Milestone Plan

## Milestone 0 --- Repository and architecture

Deliver:

-   Cargo workspace;
-   crate boundaries;
-   CI;
-   formatting;
-   linting;
-   tests;
-   basic documentation.

Acceptance:

`cargo test --workspace` passes.

## Milestone 1 --- Market feed

Deliver:

-   one live data adapter;
-   canonical events;
-   terminal output;
-   reconnect handling.

Acceptance:

Market events stream reliably for one hour.

## Milestone 2 --- Recorder/replay

Deliver:

-   event recording;
-   session metadata;
-   replay CLI;
-   deterministic ordering.

Acceptance:

A recorded session can be replayed without the network.

## Milestone 3 --- Bevy pit

Deliver:

-   one pit;
-   isometric camera;
-   100 placeholder agents;
-   pan/zoom/select.

Acceptance:

Stable interactive rendering at target FPS.

## Milestone 4 --- Simulation

Deliver:

-   firms;
-   strategies;
-   positions;
-   P&L;
-   risk limits;
-   leaderboard.

Acceptance:

Same recording + seed produces same financial results.

## Milestone 5 --- Multi-pit exchange

Deliver:

-   configurable pit grid;
-   at least 10 instruments;
-   semantic zoom;
-   LOD;
-   whole-market activity view.

Acceptance:

User can identify high-activity versus quiet pits without opening
charts.

## Milestone 6 --- Cross-market analytics

Deliver:

-   rolling relationships;
-   activity anomalies;
-   divergence detection;
-   optional pit relationship visualisation.

Acceptance:

Analytics are reproducible from recorded data.

## Milestone 7 --- Jev / AI agents

Deliver:

-   provider-neutral AI interface;
-   Jev adapter;
-   typed decisions;
-   probabilities/confidence;
-   timeouts;
-   deterministic fallback;
-   AI-agent filter.

Acceptance:

AI failure cannot stall or crash simulation.

## Milestone 8 --- Research engine

Deliver:

-   hypothesis schema;
-   backtest runner;
-   IS/OOS split;
-   metrics;
-   provenance;
-   research UI.

Acceptance:

A proposed hypothesis can be transformed into a reproducible experiment.

## Milestone 9 --- AI research

Deliver:

-   slower reasoning model integration;
-   hypothesis generation;
-   conversion to constrained experiment specification;
-   user approval before expensive experiments if appropriate.

Acceptance:

Generated hypotheses are clearly labelled unvalidated until tests
complete.

## Milestone 10 --- Visual polish

Deliver:

-   cartoon traders;
-   jackets;
-   gestures;
-   better pits;
-   animation linked to market semantics;
-   polished portfolio demo.

------------------------------------------------------------------------

# 32. Coding-Agent Working Rules

The coding agent implementing this project should follow these rules.

1.  Build vertical slices rather than creating every abstraction
    upfront.
2.  Keep domain logic independent of Bevy wherever practical.
3.  Do not introduce AI until deterministic agents work.
4.  Do not introduce complex graphics until the market/replay/simulation
    loop works.
5.  Prefer typed domain models over stringly typed data.
6.  Avoid global mutable state.
7.  Avoid unnecessary cloning to bypass borrow-checker problems.
8.  Treat compiler friction as a signal to improve ownership boundaries.
9.  Do not use `unsafe` without explicit justification.
10. Add tests with every domain module.
11. Benchmark before optimising.
12. Preserve deterministic replay.
13. Make external integrations replaceable behind traits.
14. Keep AI decisions bounded and validated.
15. Never let an AI response directly mutate arbitrary world state.
16. Store experiment provenance.
17. Prevent look-ahead bias.
18. Document architectural decisions.

------------------------------------------------------------------------

# 33. Suggested Initial Technical Stack

-   **Language:** Rust
-   **Game engine / ECS:** Bevy
-   **Async runtime:** Tokio
-   **Serialisation:** Serde
-   **Logging/tracing:** tracing
-   **Errors:** thiserror / anyhow as appropriate
-   **HTTP/WebSocket:** provider-dependent; prefer established async
    Rust crates
-   **Benchmarking:** Criterion
-   **Testing:** standard Rust test framework + property testing where
    valuable
-   **AI:** provider-neutral interface, Jev adapter as an optional
    implementation
-   **CI:** GitHub Actions

Choose storage format after benchmarking replay requirements. Start
simple before adopting a database.

------------------------------------------------------------------------

# 34. Example User Experience

The user opens OpenCry.

The camera is above a large exchange floor.

Gold, Silver, GBP/USD, EUR/USD, DXY, S&P 500, Nasdaq, Oil and Bitcoin
each occupy their own pit.

Most pits are moderately active.

Silver suddenly becomes visibly more crowded.

Its agents begin moving faster.

The Silver pit becomes one of the most visually active regions on the
floor.

GBP/USD remains quiet.

The user notices the contrast without opening a chart.

A subtle research indicator appears over Atlas Securities.

The user clicks it.

OpenCry reports:

> Potential cross-market pattern detected. Silver volatility expansion
> has recently coincided with positive Gold momentum, weakening DXY and
> unusually high Silver activity.

Status:

**UNVALIDATED HYPOTHESIS**

The user starts an experiment.

The research engine evaluates the rule against historical data.

Results are separated into discovery and out-of-sample periods.

If the pattern survives, its status becomes:

**RESEARCH CANDIDATE**

The user zooms into Silver.

Most traders are visible.

The user enables:

**AI AGENTS ONLY**

Other agents fade.

Several AI agents are buying with moderate confidence. A smaller number
are selling with high confidence.

The user clicks one.

Its exact input features and typed decision probabilities are displayed.

The application does not tell the user what to trade.

It gives the user a new way to inspect what is happening and investigate
why.

------------------------------------------------------------------------

# 35. Portfolio / CV Positioning

Suggested project description:

> **OpenCry --- Real-Time Market Simulation Engine**
>
> Built a real-time graphical market simulation in Rust and Bevy that
> transforms live and historical financial-market data into an
> interactive open-outcry exchange. Designed an event-driven
> architecture supporting deterministic replay, multi-instrument
> simulation, autonomous brokerage agents, competing strategies,
> probabilistic AI decision models, cross-market analytics and
> statistically validated research experiments. Applied Rust's ownership
> and concurrency model to high-throughput asynchronous data ingestion
> and large ECS-based agent populations.

The repository README should prominently include:

-   short demo GIF/video;
-   architecture diagram;
-   performance benchmarks;
-   explanation of why Rust;
-   explanation of deterministic replay;
-   screenshots of whole-market and pit views;
-   explanation of AI architecture;
-   research-validation methodology.

------------------------------------------------------------------------

# 36. Long-Term Possibilities

Potential later features:

-   synthetic market mode where agents actually form the market;
-   multiplayer spectator mode;
-   browser/WebAssembly viewer if practical;
-   strategy plugin SDK;
-   remote/headless simulation server;
-   distributed research workers;
-   reinforcement-learning agents;
-   brokerage management gameplay;
-   historical event packs;
-   VR/immersive exchange-floor mode;
-   voice/audio representation of market intensity;
-   user-defined pits;
-   custom market-data adapters;
-   exportable research notebooks/reports.

These are explicitly post-MVP.

------------------------------------------------------------------------

# 37. Definition of Success

OpenCry is successful as a project if it achieves three things.

## Engineering

It demonstrates serious Rust engineering:

-   memory-safe concurrent systems;
-   asynchronous streaming;
-   ECS;
-   deterministic simulation;
-   performance profiling;
-   clean architecture.

## Visualisation

A user can zoom out and gain a rapid intuitive sense of which parts of
the market are active, quiet or behaving unusually.

## Research

The system can generate or surface interesting hypotheses while
maintaining a hard boundary between:

**interesting observation** and **validated evidence**.

The application should augment human investigation rather than disguise
simulation or AI confidence as market truth.

------------------------------------------------------------------------

# 38. First Instruction to the Coding Agent

Start with **Milestone 0 and Milestone 1 only**.

Do not begin by building the full game.

Create the Rust workspace and implement:

1.  canonical instrument and market-event types;
2.  a `MarketDataSource` abstraction;
3.  one real market-data adapter;
4.  reconnect/error handling;
5.  structured tracing;
6.  unit tests;
7.  a small CLI that prints the canonical event stream.

Keep Bevy out of the first milestone.

The first architectural objective is to prove that live external market
data can enter OpenCry through a clean, typed, vendor-independent Rust
boundary. Once that is stable, implement recording/replay before
beginning the graphical world.
