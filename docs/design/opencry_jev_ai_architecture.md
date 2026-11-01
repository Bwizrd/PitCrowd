# OpenCry --- Jev / System One AI Architecture Addendum

**Purpose:** Define how Jev-style System One AI fits into OpenCry.
**Principle:** AI interprets uncertain market state; Rust retains
control of workflow, risk, validation and state mutation.

## 1. Core Architecture

Jev should not simply be an autonomous BUY / SELL / HOLD oracle. Its
best role is fast probabilistic interpretation between deterministic
market features and deterministic Rust workflows.

``` text
REAL MARKET DATA
       |
NORMALISED MARKET STATE
       |
DETERMINISTIC FEATURE ENGINE
       |
JEV / SYSTEM ONE
       |
TYPED PROBABILISTIC INTERPRETATION
       |
DETERMINISTIC RUST WORKFLOW
       |
 +-----+-------------+----------------+
 |                   |                |
TRADER LOGIC     BEVY BEHAVIOUR   RESEARCH EVENT
 |                                    |
RISK ENGINE                       SYSTEM TWO
```

Jev handles fuzzy judgement. Rust handles guarantees.

## 2. Decomposed System One Questions

Instead of asking "Should this trader buy Silver?", OpenCry should ask
smaller questions such as:

-   `bullish_pressure`
-   `bearish_pressure`
-   `momentum_persistence`
-   `breakout_likelihood`
-   `reversal_likelihood`
-   `market_regime`
-   `cross_market_confirmation`
-   `unusual_activity`
-   `liquidity_stress`
-   `trend_quality`

Example output:

``` text
bullish_pressure           0.82
bearish_pressure           0.21
momentum_persistence       0.76
breakout_likelihood        0.67
reversal_likelihood        0.24
cross_market_confirmation  0.88
unusual_activity           0.94

regime:
  trending                 0.72
  ranging                  0.16
  transitional             0.12
```

These are model interpretations, not objective truths or probabilities
of trade profitability.

## 3. Input State

Send compact, relevant structured state rather than the entire world.

``` text
Instrument: XAGUSD

Silver:
  return_1m: +0.42%
  return_5m: +0.87%
  volume_percentile: 96
  volatility_percentile: 91
  aggressive_buy_ratio: 0.68
  spread_percentile: 63

Gold:
  return_5m: +0.31%
  activity_percentile: 71

DXY:
  return_5m: -0.24%
  activity_percentile: 66

Trader:
  role: momentum
  position: flat
  pnl_today_r: +1.7
  firm_risk_utilisation: 31%
```

Rust should calculate exact features such as returns, volatility,
percentiles, P&L, drawdown, exposure and correlations. The model
interprets those features rather than recalculating deterministic facts.

## 4. Typed Domain Output

Keep Jev API types at the integration boundary. Convert them into
OpenCry domain types.

``` rust
struct MarketInterpretation {
    bullish_pressure: Probability,
    bearish_pressure: Probability,
    momentum_persistence: Probability,
    breakout_likelihood: Probability,
    reversal_likelihood: Probability,
    cross_market_confirmation: Probability,
    unusual_activity: Probability,
    regime: RegimeProbabilities,
}
```

A constrained `Probability` type should validate `0.0..=1.0`.

## 5. Rust Retains Decision Control

A strategy can consume the interpretation:

``` rust
if analysis.momentum_persistence > 0.80
    && analysis.cross_market_confirmation > 0.70
    && analysis.bullish_pressure > analysis.bearish_pressure
    && risk.can_open_position()
{
    strategy.consider_long_entry();
}
```

The AI provides probabilistic judgement. Rust owns constraints, risk,
state transitions, position accounting and validation.

## 6. Provider-Neutral Boundary

Do not tightly couple OpenCry to Jev.

``` rust
trait AgentDecisionModel {
    async fn analyse(
        &self,
        observation: &MarketObservation,
    ) -> Result<MarketInterpretation, ModelError>;
}
```

Potential implementations:

``` text
JevDecisionModel
DeterministicDecisionModel
LlmDecisionModel
LocalDecisionModel
MockDecisionModel
```

This protects OpenCry from API changes, enables offline tests, permits
model comparisons and keeps deterministic replay possible.

## 7. Shared Intelligence vs Independent Agents

Do not give every one of 10,000 visual agents an independent external
inference call.

A possible architecture:

``` text
10,000 logical agents

9,500 deterministic / quantitative agents
  400 consuming shared System One market interpretations
  100 genuinely independent AI discretionary agents
```

Exact numbers are configuration and benchmarking decisions.

Shared analysis can work like:

``` text
SILVER STATE
     |
    JEV
     |
SHARED SILVER INTERPRETATION
     |
 +---+---------+----------+
 |             |          |
Momentum     MeanRev   CrossMarket
Trader       Trader     Trader
```

Different strategies consume different parts of the same probabilistic
interpretation.

## 8. Confidence Becomes Behaviour

AI confidence can become part of OpenCry's visual language.

``` text
BUY inclination 0.51 -> hesitant
BUY inclination 0.65 -> purposeful walking
BUY inclination 0.80 -> running
BUY inclination 0.95 -> highly aggressive animation
```

Other encodings can include path directness, gesture strength,
hesitation and clustering.

This makes the character a visual debugger for the model's state.

Crucially, model confidence is not the probability that a trade will
make money.

## 9. Compute Graph

System One should be a node inside explicit software workflows.

``` text
             MARKET FEATURES
                   |
             SYSTEM ONE
                   |
      +------------+------------+
      |            |            |
   Momentum      Regime       Anomaly
     0.84          0.72         0.91
      |            |            |
      +------------+------------+
                   |
             RUST WORKFLOW
                   |
          +--------+--------+
          |                 |
      TRADER LOGIC      RESEARCH EVENT
```

The compute graph itself remains ordinary, inspectable and testable
code.

## 10. System One and System Two

### System One

Fast, reactive, frequent, typed and probabilistic.

Uses:

-   market interpretation
-   trader perception
-   classification
-   anomaly detection
-   regime interpretation
-   routing and scoring

Jev is a candidate implementation.

### System Two

Slower and deliberative.

Uses:

-   explaining unusual behaviour
-   generating strategy hypotheses
-   analysing failed/successful trades
-   proposing cross-market relationships
-   designing experiments
-   summarising sessions

A conventional reasoning model is more appropriate here.

## 11. AI Research Department

System Two may produce:

``` text
UNVALIDATED HYPOTHESIS

Silver volatility expansion may persist more frequently when:
- Gold momentum is positive
- DXY is falling
- Silver activity is above its 90th percentile
```

The AI is allowed to propose the idea. It is not allowed to declare the
idea profitable.

Every hypothesis follows:

``` text
Observation
 -> AI Hypothesis
 -> Formal Machine-Testable Rule
 -> Historical Test
 -> In-Sample Result
 -> Out-of-Sample Result
 -> Paper Observation
 -> Research Candidate
```

Store provenance, feature definitions, rule, dataset, sample size,
costs, metrics, model/version and experiment version.

## 12. Brokerage Intelligence

Different firms may develop different simulated cultures:

``` text
Atlas Securities    momentum + cross-market
Redwood Capital     mean reversion
Northstar Markets   liquidity / order flow
Greyfriars Trading  contrarian / relative value
Valkyrie Capital    breakout / volatility expansion
```

System One agents can interpret the same market differently because
strategies weight the probabilistic features differently.

System Two researchers can generate hypotheses consistent with each
firm's philosophy.

## 13. Controlled Evolution

Later a firm may modestly change capital allocation after validated
research.

Never allow an LLM to arbitrarily rewrite strategy code and deploy it
into the running simulation.

Prefer:

``` text
AI proposes
 -> Rust validates
 -> research engine tests
 -> policy permits
 -> configuration changes
 -> event recorded
```

## 14. Failure Handling

External AI must never be a critical dependency for the simulation loop.

Handle:

-   timeout
-   unavailable service
-   malformed transport response
-   rate limiting
-   budget exhaustion
-   authentication failure
-   unavailable model/version

Possible fallback:

``` text
HOLD
last valid interpretation
deterministic strategy
no AI interpretation
```

Never freeze the Bevy frame waiting for inference.

## 15. Async Boundary

``` text
BEVY / SIMULATION
      |
bounded request channel
      |
AI WORKERS
      |
bounded result channel
      |
SIMULATION CONSUMER
```

Use timeouts, bounded queues, stale-result rejection and cancellation
where appropriate.

If market state has materially changed before a response arrives,
discard the stale result.

## 16. Inference Scheduling

Do not infer every frame.

Potential triggers:

-   fixed interval
-   meaningful market-state change
-   activity threshold crossed
-   anomaly detected
-   selected pit receives higher update frequency

Example only:

``` text
background pits: 1 analysis/sec
selected pit:    5 analyses/sec
```

Final rates should come from measurement.

## 17. Cost Controls

Track and configure:

``` text
requests/sec
requests/session
input volume
estimated cost/session
estimated cost/hour
max_requests_per_second
max_independent_ai_agents
max_ai_cost_per_hour
```

When a budget is reached, reduce inference gracefully.

Shared interpretation should be the default optimisation.

## 18. Observability

Measure:

``` text
request count
latency p50/p95/p99
timeouts
errors
fallback count
stale responses
input size
estimated cost
model version
```

For research, also examine performance by confidence bucket, regime and
model version.

## 19. Deterministic Replay

External inference complicates deterministic replay.

### Recorded inference mode

Persist:

``` text
observation hash
model/version
typed output
probabilities
timestamp
```

Replay consumes the recorded inference.

### Re-inference mode

Replay historical market data but call the current model again.

Use this for model comparisons and research. It is not expected to
reproduce the original run.

## 20. Testing

Create a `MockDecisionModel` with predetermined responses. Most
automated tests should not call Jev.

Test:

-   probability validation
-   domain conversion
-   fallback selection
-   stale-result rejection
-   confidence-to-animation mapping
-   deterministic workflow branching
-   timeout
-   rate limit
-   service unavailable
-   late response

Given known probabilities and risk state, the Rust workflow should
produce a known outcome.

## 21. Security and Integrity

Never place API keys in source code, Git, recordings, screenshots or
logs.

Do not allow model output to:

-   execute shell commands
-   mutate arbitrary files
-   dynamically execute generated code
-   bypass risk controls
-   directly place real-money trades

The AI boundary remains deliberately narrow.

## 22. Open-Source Boundary

Reasonable public components:

``` text
AI interfaces
Jev adapter
market interpretation schema
mock models
confidence visualisation
simulation infrastructure
research framework
```

Potentially private:

``` text
personal strategy rules
proprietary hypotheses
validated research
private datasets
API credentials
live trading integrations
```

The OpenCry engine can be public without publishing the valuable
research it eventually produces.

## 23. Implementation Phases

### Phase 0 --- No AI

Graphics, fake market activity, market-state abstraction and
deterministic agents.

### Phase 1 --- Mock System One

Implement `AgentDecisionModel`, `MockDecisionModel` and
`MarketInterpretation`.

### Phase 2 --- Jev Spike

Standalone CLI: market observation in, typed probabilities out. Measure
latency, cost, stability and API ergonomics.

### Phase 3 --- Shared Pit Intelligence

Connect Jev to one pit, probably Silver. Visualise the shared
interpretation.

### Phase 4 --- Independent AI Agents

Add a small number of independently querying agents.

### Phase 5 --- Multi-Market Intelligence

Use Gold, Silver, DXY, FX and indices together.

### Phase 6 --- System Two Research

Add slower hypothesis generation.

### Phase 7 --- Validation Engine

Require deterministic testing before research ideas receive candidate
status.

## 24. Acceptance Criteria

The AI architecture is successful when:

-   OpenCry consumes typed domain outputs rather than arbitrary prose.
-   Rust remains responsible for application state transitions.
-   Jev can disappear and the application continues running.
-   inference never blocks rendering.
-   inference rate/cost are observable and configurable.
-   confidence has a meaningful visual representation.
-   selecting an AI trader exposes features, interpretation,
    probabilities and strategy response.
-   AI outputs can be recorded for deterministic replay.
-   generated hypotheses remain explicitly unvalidated until tested.

## 25. Key Design Principle

Wrong:

``` text
MARKET -> AI -> DO WHATEVER AI SAYS
```

OpenCry:

``` text
MARKET
  -> DETERMINISTIC FEATURES
  -> PROBABILISTIC AI INTERPRETATION
  -> DETERMINISTIC RUST WORKFLOW
       -> risk
       -> simulation
       -> visualisation
       -> research
```

## 26. Long-Term Vision

At maturity OpenCry can contain three simultaneous layers:

``` text
LAYER 1 — REAL MARKET

prices
volume
liquidity
volatility
cross-market behaviour


LAYER 2 — SYSTEM ONE

rapid probabilistic interpretations

"momentum looks persistent"
"activity is unusual"
"cross-market confirmation is strong"
"regime appears transitional"


LAYER 3 — SYSTEM TWO

slower research

"Why did Atlas perform well?"
"Is this Silver/Gold/DXY relationship repeatable?"
"Generate a falsifiable hypothesis."
"Test it."
```

The Bevy world turns those abstract layers into something spatial,
behavioural and inspectable.

The end goal is not:

> AI tells the user what to trade.

It is:

> OpenCry gives the user a living representation of market state,
> simulated intelligence, uncertainty, competing strategies and emerging
> research, while deterministic software keeps those concepts distinct.
