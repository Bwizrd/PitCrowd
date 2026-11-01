# OpenCry: Market Data Strategy & The Procedural Advantage
### *Why Free Pepperstone cTrader Feeds + Jev Are More Than Enough to Power the Living Floor*

---

## 1. The Myth of the Expensive Institutional Feed

A common fear when building market visualization software is that the system requires multi-thousand-dollar institutional feeds—such as raw CME ITCH/TotalView tapes, Nasdaq direct feeds, or expensive Bloomberg/Refinitiv terminals.

**In reality, this is a misconception.**

OpenCry does not require a dedicated data feed for every simulated trader on the screen. By combining:
1. **The Procedural Collective Principle ("Boids / Flock of Birds")**,
2. **Pepperstone cTrader's Free Open API Data Layers (Ticks, Trendbars & DOM Depth)**, and
3. **Jev's TypeSafe System-One Probabilistic Envelopes**,

we already possess 100% of the data fuel required to drive an immersive, living trading floor in real time.

---

## 2. The Procedural Secret: The "Flock of Birds" Principle

In game engines and film CGI (such as animating massive crowd battle scenes or flocks of birds), the computer does **not** track individual instructions for every single character.

Instead, simulation uses **procedural collective steering**:
* **Macro Market Forces (cTrader + Jev):** Establish the aggregate "weather" in the room—locomotion velocity, crowd density, emotional tension, and directional vectors.
* **Micro Agents (Bevy ECS):** Procedurally simulate individual footsteps, coat colors, random head turns, and hand gestures within that macro field.

```
┌─────────────────────────────────────────────────────────────┐
│             AGGREGATE MARKET DATA & JEV ENVELOPE            │
│         (cTrader Ticks/Depth + Jev System-One State)        │
└──────────────────────────────┬──────────────────────────────┘
                               │
                               ▼
┌─────────────────────────────────────────────────────────────┐
│                PROCEDURAL BEVY ECS STEERING                 │
│         Drives 50 Living Agents Reacting to the Room        │
└─────────────────────────────────────────────────────────────┘
```

You do not need a feed for every clerk in the pit. The data feed sets the room's atmospheric pressure; the visual engine animates the people reacting to that pressure.

---

## 3. Data Mapping: Extracting Physical Motion from Existing Feeds

Even looking strictly at standard **OHLCV trendbars, real-time ticks, and Jev probabilities**, notice the extraordinary depth of physical motion extracted:

| Market Data Source | Physical Floor Expression in OpenCry |
| :--- | :--- |
| **Volume (Bar / Cumulative)** | Total number of agents occupying the pit (Crowd density). |
| **Tick Arrival Velocity (Ticks/sec)** | Locomotion speed (strolling $\to$ brisk walk $\to$ sprint). |
| **Bar Range $(High - Low)$** | Spatial dispersion (how widely agents spread across the floor). |
| **Body vs. Wick Ratio (Intrabar Delta)** | Directional lean (pressing the buy rail vs. sell rail). |
| **Spread $(Ask - Bid)$** | Friction and hesitation (wide spreads = nervous crowd spacing). |
| **Level 2 DOM Imbalance** | Asymmetrical crowding (buyers swarming one half of the ring). |
| **Jev `p_persistent` / `p_counter`** | Cohesive phalanx charge vs. fractured hesitation. |

---

## 4. What Pepperstone cTrader Actually Delivers (Free of Charge)

Pepperstone's cTrader Open API provides access to data far beyond simple historical candles:

1. **Live Trendbars (`ProtoOAGetTrendbarsReq` / Live Streaming):**  
   Streaming 1-minute, 5-minute, and tick-based OHLCV bars.
2. **Sub-Millisecond Tick Spots (`ProtoOASpotEvent`):**  
   Every bid and ask price change streamed in real time.
3. **Level 2 Depth of Market (`ProtoOAOrderBookEvent`):**  
   Real-time order book queues showing up to 10–20 price levels of bidding and offering volume.

All of these are available through standard cTrader Open API credentials with zero monthly subscription fees.

---

## 5. Free and Low-Cost Additions for the Future

If specialized asset classes require deeper direct feeds in the future:

1. **Crypto Pits (Bitcoin, Ethereum, Solana):**  
   Binance, Coinbase, and Kraken provide **100% free, public WebSockets** streaming full Level 2 order books and trade-by-trade executions with zero authentication and zero cost.
2. **US Futures Pits (CME ES, NQ, Crude Oil, Gold Futures):**  
   Modern infrastructure providers like **Databento** stream raw CME institutional tick-by-tick and depth feeds on a **pay-as-you-go** model (charging by the megabyte). An entire active trading session costs **$0.15 to $0.50**, completely eliminating expensive monthly commitments.

---

## 6. Conclusion

The data problem is solved. 

Between Pepperstone cTrader for physical market ticks and Jev for cognitive intent, we have all the inputs necessary to bring the OpenCry trading floor to life.
