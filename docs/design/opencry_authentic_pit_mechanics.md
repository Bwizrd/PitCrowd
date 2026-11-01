# OpenCry: Authentic Open Outcry Pit Mechanics & Simulation Architecture

> **Note (2026-10-20):** the mechanics, hand-signal and scenario sections of this document are superseded by [`PIT_MICROSTRUCTURE_AND_HISTORICAL_REPLAY.md`](PIT_MICROSTRUCTURE_AND_HISTORICAL_REPLAY.md). See its Appendix A for the specific corrections (price vs quantity signals, the silent 1987 open, roster numbering).

## 1. Executive Summary & Purpose

The goal of **JevFloor (OpenCry)** is to resurrect the lost high-bandwidth intelligence of historical open outcry trading pits (such as CME, CBOT, LIFFE, and NYMEX) and map modern electronic order flow directly into spatial, biological, and acoustic intuition.

In modern electronic trading, institutional size is masked by algorithms, icebergs, and dark pools. Tracing market state through numeric DOMs (Depth of Market) or standard 2D candlestick charts imposes heavy cognitive load. In contrast, the trading pits of the 1980s and 1990s were human supercomputers: traders used **spatial density, hand signals, decibel volume, body posture, and cross-pit contagion** to intuitively gauge institutional liquidity imbalances long before prices finalized on electronic screens.

This document details the exact historical operation of trading pits, catalogs the participants and physical mechanics, and specifies how JevFloor faithfully models these dynamics.

---

## 2. Physical Geometry & Floor Architecture

```
                    ┌────────────────────────────────────────────────────────┐
                    │               PERIMETER BOOTHS & PHONE DESKS           │
                    │   (Institutional Banks, Brokerages, Wire Terminals)   │
                    └────────────────────────────────────────────────────────┘
                                     │                     │
                Hand Signals (Palm In/Out)                 │ Runners Sprinting Tickets
                                     ▼                     ▼
┌──────────────────────────────────────────────────────────────────────────────────────────────┐
│                                 THE CONTINUOUS ISOMETRIC FLOOR                               │
│                                                                                              │
│       Aisle / Walkway                       Aisle / Walkway                  Aisle / Walkway │
│                                                                                              │
│    ┌───────────────────────────┐                       ┌───────────────────────────┐         │
│    │     OCTAGONAL PIT #1      │                       │     OCTAGONAL PIT #2      │         │
│    │     (e.g., S&P 500)       │                       │  (e.g., 10Y/30Y Treasury) │         │
│    │                           │                       │                           │         │
│    │     [Top Tier: Brokers]   │   Cross-Pit Shock     │     [Top Tier: Brokers]   │         │
│    │     [Mid Tier: Locals]    │ ◄───────────────────► │     [Mid Tier: Locals]    │         │
│    │     [Pit Center: Lead Mo] │   Acoustic Contagion  │     [Pit Center: Lead Mo] │         │
│    │                           │                       │                           │         │
│    │     [Raised Pulpit /      │                       │     [Raised Pulpit /      │         │
│    │      Price Reporters]     │                       │      Price Reporters]     │         │
│    └───────────────────────────┘                       └───────────────────────────┘         │
│                                                                                              │
└──────────────────────────────────────────────────────────────────────────────────────────────┘
```

### The Stepped Octagonal Pit
- **Concentric Amphitheater Design:** Pits were constructed as descending hexagonal or octagonal steps descending into a sunken center ("the well").
- **Line-of-Sight Ergonomics:** A flat floor restricts visibility to the front row. Stepped tiers ensured that every single trader across a 300+ person pit could look over others' shoulders to read eye contact, mouth movements, badges, and hand signals.
- **Contract Delivery Zones:**
  - **The Center Well:** Reserved for the "Lead Month" (front-month contract), where volume, volatility, and physical crowding were highest.
  - **The Upper Tiers & Corners:** Assigned to back-month contracts, calendar spreads, and option strikes.
  - **Top Step:** Prime vantage point occupied by senior commission brokers needing direct line of sight to their firm's perimeter desks.

---

## 3. The Cast of Characters & Roles

A pit was an ecosystem of exchange officials, executing agents, and risk-taking market makers. There was **no single monopoly market maker**; liquidity was provided by competing independent locals.

### A. Exchange Officials (Floor Governance)

1. **Price Reporters ("Pit Observers" on the Catwalk / Central Pulpit):**
   - **Position:** Stationed on an elevated podium (the pulpit) or an overhead catwalk directly inside or above the pit.
   - **Role:** Exchange staff equipped with headsets and specialized keypads. They monitored trades occurring at *new* price ticks.
   - **Action:** The moment a trade executed at a new tick, the reporter keyed it in. This instantaneously updated the giant electronic overhead wallboards and disseminated prints globally via Reuters, Bloomberg, and the ticker tape.
2. **Floor Officials ("Referees"):**
   - **Attire:** Distinctive jackets (burgundy, blue, or gold).
   - **Role:** Enforced exchange rules, resolved disputes regarding who screamed "Done!" first, and arbitrated "out-trades" (unmatched or contested transactions).

### B. Market Participants (The Traders)

1. **Floor Brokers (Commission Brokers):**
   - **Affiliation:** Major investment banks and clearing houses (Goldman Sachs, Refco, Merrill Lynch, Salomon Brothers).
   - **Tools:** Held physical **order decks**—stacks of order tickets (market, limit, stop).
   - **Mandate:** Pure execution for off-floor institutional clients. They carried zero personal market risk and profited from commission per contract.
   - **Behavior:** Stood on upper steps facing outward toward perimeter phone desks. When large orders arrived, they stepped forward, raised hands, and bellowed order sizes into the pit.
2. **"Locals" (Independent Scalpers & Market Makers):**
   - **Affiliation:** Self-employed individuals trading personal capital or backed by proprietary clearing accounts.
   - **Mandate:** Provide two-way liquidity (bidding and offering) and scalp micro-ticks.
   - **Behavior:** They stood facing inward toward the brokers. When a massive order hit the pit, dozens of locals stepped in simultaneously, absorbing pieces (e.g., 10, 50, or 200 contracts each) onto their books.
3. **Spreaders & Arbitrageurs:**
   - **Mandate:** Traded mispricings between contracts (e.g., Cash vs. Futures, S&P 500 vs. NASDAQ, or Yield Curve Spreads between Eurodollars and 10-Year Notes).
   - **Behavior:** Constant rapid head movement between adjacent pits or between phone desks and the pit well.

### C. The Perimeter Pipeline (Outside World Connection)

1. **Phone Desks & Order Clerks:**
   - Positioned in continuous rows along the outer walls of the trading floor.
   - Received real-time telephone and teletype orders from upstairs trading desks and institutional clients worldwide.
2. **Flashers (Signal Clerks):**
   - Clambered onto elevated pedestals at their perimeter desks to flash hand signals across 50–100 yards of frantic trading floor directly to their pit broker.
3. **Runners:**
   - Couriers sprinting paper tickets between perimeter desks and brokers in the pit for complex or confidential orders too large to signal openly.

---

## 4. Trading Rules, Protocols, and Price Discovery

Open outcry was governed by strict exchange rules to maintain a transparent, continuous double auction.

### Open Outcry Rule
- **No Private Whispering:** Any trade made secretly or without shouting to the crowd was illegal and grounds for expulsion. Every bid and offer had to be declared audibly and visually.

### Hand Signal Language
Hand signals functioned as high-speed optical data links:
- **Direction (Buy vs. Sell):**
  - **Palm Facing IN towards own body:** "I want to BUY" (Bid).
  - **Palm Facing OUT away from body:** "I want to SELL" (Offer).
- **Price Ticks:**
  - Fingers extended vertically or horizontally relative to the face/chin indicated fractional ticks (e.g., quarters, eighths, or single basis points).
- **Contract Size / Quantity:**
  - Fingers held vertically away from the body indicated 1 through 9 contracts.
  - Touching the chin or chest indicated tens or hundreds of contracts.
  - Crossed arms or hands on the top of the head signified large blocks (e.g., "Full Boat").

### The Lifecycle of an Institutional Price Cascade

```
[Institutional Client Order]
         │ "Sell 5,000 Dec S&P at Market"
         ▼
[Perimeter Desk Clerk]
         │ Flashes hand signal across hall (Palm OUT, Size 5,000)
         ▼
[Lead Floor Broker in Pit]
         │ Bellows into the pit: "5,000 to sell! Who wants 'em at 40?"
         ▼
[Locals / Scalpers Sense Massive Supply Imbalance]
         │ Locals immediately pull their bids down to avoid being run over:
         │ "No bid at 40! 35 bid! 30 bid!"
         ▼
[Price Cascade & Swarm]
         │ Broker drops price tick-by-tick: "38! 35! 32! 30!"
         │ Traders physically surge forward, waving badges and yelling "Sold!"
         ▼
[Absorption & Print]
         │ 40 locals take slices of 50-200 contracts each; order filled.
         │ Price Reporter on Pulpit types 30.00 print -> ticker tape updates.
         ▼
[Post-Shock Rebound / Inventory Rebalancing]
         │ Broker closes deck. Selling pressure vanishes instantly.
         │ Locals now sit heavily LONG. They immediately shout bids higher
         │ to scratch inventory, causing an immediate 2-tick rebound.
```

---

## 5. Acoustic and Spatial Dynamics

Veteran traders rarely stared at electronic quotes. They navigated by **ambient decibels and physical motion vectors**:

1. **The Floor Roar (Decibel Dynamics):**
   - **Quiet Chop:** Rhythmic, low-frequency hum. Locals trade back and forth within a tight 1-tick band.
   - **Breakout / Institutional Influx:** Sound transitions from a low hum to an explosive, wall-vibrating roar.
   - **Acoustic Contagion:** When the Treasury Bond pit erupted over economic releases, the sound wave physically traveled across the floor. Equity and currency traders heard the roar 1–3 seconds before news hit Bloomberg screens, immediately adjusting their positioning.
2. **Physical Phalanxes and Scrums:**
   - When a broker held an order, locals physically leaned in, jostled for position, and shoved tickets toward the broker’s eyes.
   - A cluster of bodies converging on a single spot was an unmistakable spatial signal that a major institutional trade was taking place.

---

## 6. Relevance to Modern Electronic Trading

Although floor pits have largely transitioned to electronic matching engines, **the underlying microstructure physics remain identical**:

| Historical Pit Phenomenon | Modern Electronic Equivalent (Level 2/3 Data) | What It Indicates |
| :--- | :--- | :--- |
| **Locals backing away / pulling bids** | Liquidity provider order book cancellation / queue depletion | Market makers refusing to provide cheap liquidity against aggressive flow |
| **Broker shouting large block & dropping price** | Aggressive institutional market sweep / Iceberg execution | Urgent directional liquidity demand exhausting resting depth |
| **Physical scrum around broker** | Heavy clustered trade prints on Time & Sales | High volume-at-price node establishing value or capitulation |
| **Post-order quiet & immediate rebound** | Delta exhaustion & mean-reversion bounce | Inventory absorption completed; market makers scratching hedges |
| **Cross-pit acoustic roar** | Cross-asset lead-lag correlation (Yields vs. Equities) | Macro shock propagating across fragmented liquidity venues |

By translating electronic order flow metrics (delta imbalance, book pressure, sweep velocity, trade clustering) into an authentic isometric pit simulation, traders can tap into innate human biological perception rather than deciphering raw numbers.

---

## 7. JevFloor Implementation Roadmap

To advance JevFloor from decorative isometric sprites to an authentic, functionally intuitive trading tool, the following components are specified:

### A. Architectural & Environmental Assets
- **The Central Pulpit / Rostrum:**
  - Place a raised octagonal or rectangular podium in the pit center or adjacent catwalk.
  - Station **Price Reporters** with green/gold badges and visual terminal screens that flash green/red when a new high/low tick occurs.
- **Perimeter Desks & Phone Banks:**
  - Line the outer borders of the isometric floor with wooden desks, bank turrets, and phone clerks.
  - Render optical **beam lines or hand-signal animations** connecting the desks to pit brokers during institutional order routing.

### B. Distinct Agent Roles & Behavior States
- **Commission Broker Sprite:**
  - Holds an order book/deck.
  - Spawns at the perimeter or upper pit step.
  - Animations: Arms raised with **palms facing outward** during aggressive sell flow, or **palms inward** during aggressive buy flow.
- **Local Scalper Sprites:**
  - Clustered in the pit well.
  - Behavior: Circle and converge upon active brokers when volume accelerates; retreat to pit edges when spread widens.
- **Floor Official Sprites:**
  - Patrol aisles and pit peripheries in distinct colored coats.

### C. Sensory & Audio-Visual Systems
- **Dynamic Decibel Sound Engine:**
  - Layered ambient audio: Background murmurs during low activity; rising crowd roar tied directly to trade rate and volume velocity.
- **Floor Contagion Vectors:**
  - Visual shockwaves (rings or runners) propagating across floor aisles from one pit to another when volatility spikes.
- **Market Flow Integration:**
  - Map incoming WebSocket/feed book delta and trade prints directly to broker hand gestures and local agent swarm density.
