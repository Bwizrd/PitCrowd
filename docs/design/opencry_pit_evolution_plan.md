# OpenCry / JevFloor: Pit Evolution Plan

## Purpose & Trading Utility Filter

Every visual and auditory component in JevFloor must pass one test:
> **Does this reduce cognitive load and provide an intuitive read on who is in control of the market?**

Modern electronic markets conceal institutional intent behind iceberg algorithms and dark pools. By mapping Level 2/3 order flow, volume velocity, and book depth into spatial crowd geometry, physical gestures, and sound, JevFloor restores the high-bandwidth sensory advantages of the historical open-outcry trading floor.

---

## 1. What We Incorporate (High Trading Value)

| Pit Element | Modern Electronic Reality | Trading Decision Edge |
| :--- | :--- | :--- |
| **Lead Broker & Hand Signals** | Aggressive Market Orders (Taker Sweeps) | **Palms In (Green)** = Institutional Buy Sweep.<br>**Palms Out (Red)** = Institutional Sell Sweep.<br>Instantly reveals aggressive taker intent. |
| **Locals' Swarm vs. Retreat** | Resting Limit Depth (Maker Liquidity) | **Swarm & Pack**: High liquidity absorption at support/resistance.<br>**Scatter & Retreat**: Market makers pulling bids; momentum breakout likely. |
| **Elevated Pulpit (Price Reporter)** | Clean Last Print & Tick Delta | Central visual anchor displaying clean current price, tick direction (+/-), and print flash. |
| **Floor Roar / Decibels** | Trade Velocity (Trades / Second) | 360° audio awareness. Ambient hum indicates low-volatility chop; a roar indicates sudden volume expansion. |
| **Cross-Pit Contagion Waves** | Cross-Asset Lead/Lag (Rates ➔ Equities) | Visual/acoustic ripples crossing floor aisles flag macro trend days and lead-lag front-running opportunities. |
| **Perimeter Desks & Flash Beams** | Institutional Order Influx Detection | Beam/runner from perimeter booth to pit alerts to incoming institutional block volume before execution completes. |

---

## 2. What We Ruthlessly Leave Out (Visual Clutter)

- ❌ **Detailed fractional finger codes (1/8ths, 1/4s):** Modern assets trade in decimals. Reading micro-finger positions on isometric sprites creates unneeded visual strain. Simple **Palms In (Buy)** vs **Palms Out (Sell)** with arm elevation indicating urgency is far clearer.
- ❌ **Floor debris / paper order slips:** Realistic for 1985, but creates distracting visual noise on screens.
- ❌ **The Sims-style life needs:** Characters never eat, chat, or take breaks. Every entity directly represents an order-flow micro-structure state.
- ❌ **Out-trade disputes / Referees:** Electronic matching engines are deterministic; out-trade arbitration has no modern equivalent.

---

## 3. Phased Implementation Roadmap

```
┌────────────────────────────────────────────────────────┐
│  Phase 1: Roles, Gestures & Key (Visual Hierarchy)     │
│  - Central Price Reporter Pulpit in pit center         │
│  - Distinct Floor Broker sprite with Palms In/Out      │
│  - Reactive Local Scalper sprites forming the ring     │
│  - On-screen Role Key / Legend explaining characters   │
└───────────────────────────┬────────────────────────────┘
                            │
┌───────────────────────────▼────────────────────────────┐
│  Phase 2: Absorption & Liquidity Physics               │
│  - Locals pack tightly when book depth is thick        │
│  - Locals back away when liquidity pulls               │
│  - Swarm animation upon aggressive sweep fill          │
└───────────────────────────┬────────────────────────────┘
                            │
┌───────────────────────────▼────────────────────────────┐
│  Phase 3: Perimeter Desks & Influx Pipelines           │
│  - Outer floor booths (Institutional, Retail, Arbs)    │
│  - Flasher / runner beam to pit on large size          │
└───────────────────────────┬────────────────────────────┘
                            │
┌───────────────────────────▼────────────────────────────┐
│  Phase 4: Ambient Audio Engine & Cross-Pit Contagion   │
│  - Dynamic room hum ➔ crescendo crowd roar             │
│  - Contagion ripples between correlated pits           │
└────────────────────────────────────────────────────────┘
```

---

## 4. Phase 1 Specification (Immediate Target)

### A. Role Distinction & Visual Hierarchy
1. **The Floor Broker (The Aggressor)**:
   - **Appearance:** Royal Blue coat with gold collar / lapels, holding an order clipboard/deck.
   - **Stance:** Positioned prominently at the upper tier / focal point of each pit.
   - **Gestures:**
     - Buy Imbalance (Delta > 0): Raises hands with **Palms In** (Green glow/indicators).
     - Sell Imbalance (Delta < 0): Raises hands with **Palms Out** (Red glow/indicators).
     - Neutral / Chop: Hands down, reading order deck.
2. **The Local Scalpers (The Liquidity Providers)**:
   - **Appearance:** Array of distinctive firm-colored jackets (Emerald, Crimson, Amber, Turquoise).
   - **Stance:** Arranged along the steps of the pit, facing toward the central pulpit and broker.
   - **Behavior:** Shuffle and align toward the active broker during volume surges.
3. **The Price Reporter (The Official Anchor)**:
   - **Appearance:** Burgundy exchange jacket and headset, standing atop a raised wooden central rostrum / pulpit in the middle of the pit.
   - **Display:** Rostrum terminal displaying the current price, formatted neatly, flashing Green on upticks and Red on downticks.

### B. On-Screen Role Key / Legend
- Render a compact, clean semi-transparent Key in the HUD:
  - 👔 **Floor Broker**: Institutional Order Executer (Palms In = Buy, Palms Out = Sell)
  - 🦺 **Locals (Scalpers)**: Independent Market Makers providing two-way liquidity
  - 🎙️ **Price Reporter**: Exchange Official on central pulpit logging prints
  - ✋ **Hand Signals**: Green Palms In (Bids), Red Palms Out (Offers)
