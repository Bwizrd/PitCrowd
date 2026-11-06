//! Pit participant engine: a per-pit, two-sided, shouted auction among 18 slots.
//!
//! This module is the single source of truth for what every trader in a pit is
//! doing and why. Both the 2D floor and the 3D amphitheater render the state in
//! here; neither invents behaviour of its own.
//!
//! Model (see PIT_MICROSTRUCTURE_AND_HISTORICAL_REPLAY.md §3):
//! * Customer orders arrive at brokers as a Poisson stream whose rate and side
//!   mix are the only things a scenario controls.
//! * Locals quote both sides around a drifting fair value, skewed by their own
//!   inventory. They pull quotes when the market goes fast.
//! * Brokers broadcast an order (hands up, shouting) *before* anyone fills it, so
//!   the sound of the pit rises before the price moves. If nobody is on the other
//!   side they walk the price, which moves fair value without a print: a vacuum.
//! * Fills are allocated with human noise: proximity, attention, a small chance
//!   of the wrong counterparty, and a small chance that two people both think they
//!   got it (an out-trade).
//! * The reporter prints only price changes.
//!
//! Everything is deterministic given the seed, and `step` is pure Rust so the
//! whole thing is unit-testable without a Bevy `App`.

use std::collections::VecDeque;

use bevy::prelude::*;

use crate::market::{Instrument, MarketActivitySim};
use crate::scenario::{MarketScenario, MarketScenarioController};

/// Number of slots in a pit. Slot indices are shared with the 2D and 3D views.
pub const SLOTS: usize = 18;
/// Ring-buffer capacity for the per-pit event log.
pub const EVENT_RING_CAP: usize = 256;
/// Maximum sub-step length so behaviour does not depend on frame rate.
const MAX_SUBSTEP: f32 = 0.05;

// ---------------------------------------------------------------------------
// Basic types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Buy,
    Sell,
}

impl Side {
    pub fn opposite(self) -> Side {
        match self {
            Side::Buy => Side::Sell,
            Side::Sell => Side::Buy,
        }
    }

    pub fn sign(self) -> i32 {
        match self {
            Side::Buy => 1,
            Side::Sell => -1,
        }
    }

    pub fn is_buy(self) -> bool {
        matches!(self, Side::Buy)
    }
}

/// What a slot does for a living. Fixed per slot so the 2D and 3D rosters agree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParticipantRole {
    /// Institutional broker on the top rail: receives the biggest customer orders.
    LeadBroker,
    /// Exchange official on the pulpit: prints price changes.
    Reporter,
    /// Filling broker on the steps: receives smaller customer orders.
    Broker,
    /// Independent local: quotes both sides, runs inventory.
    Local,
    /// Runner: carries filled tickets out; never quotes.
    Runner,
}

/// Where a slot stands. Drives reaction latency and fill proximity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    Rail,
    Pulpit,
    Middle,
    Outer,
}

pub fn role_for_slot(slot: usize) -> ParticipantRole {
    match slot {
        0 => ParticipantRole::LeadBroker,
        1 => ParticipantRole::Reporter,
        4 | 8 | 13 => ParticipantRole::Broker,
        10 | 14 => ParticipantRole::Runner,
        _ => ParticipantRole::Local,
    }
}

pub fn tier_for_slot(slot: usize) -> Tier {
    match slot {
        0 => Tier::Rail,
        1 => Tier::Pulpit,
        2..=9 => Tier::Middle,
        _ => Tier::Outer,
    }
}

/// Angular position of a slot around the pit, radians, used for proximity.
/// Rail broker stands north (-pi/2); the pulpit is the centre (angle 0 but distance 0).
pub fn slot_angle(slot: usize) -> f32 {
    match slot {
        0 => -std::f32::consts::FRAC_PI_2,
        1 => 0.0,
        2..=9 => (slot - 2) as f32 / 8.0 * std::f32::consts::TAU,
        _ => (slot - 10) as f32 / 8.0 * std::f32::consts::TAU + std::f32::consts::PI / 8.0,
    }
}

/// Smallest angular distance between two slots, 0..pi.
pub fn slot_angular_distance(a: usize, b: usize) -> f32 {
    if a == 1 || b == 1 {
        return std::f32::consts::FRAC_PI_2;
    }
    let d = (slot_angle(a) - slot_angle(b)).rem_euclid(std::f32::consts::TAU);
    d.min(std::f32::consts::TAU - d)
}

/// The behavioural state machine for one participant.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Phase {
    /// Scanning the pit, hands down.
    Idle,
    /// A local showing a bid or offer. Hands up: palms in for a bid, palms out for an offer.
    Quoting {
        side: Side,
        price_ticks: i32,
        qty: u32,
        remaining: f32,
        age: f32,
    },
    /// A broker broadcasting a customer order. Same hands as a quote, but he is the aggressor.
    Working {
        side: Side,
        price_ticks: i32,
        limit_ticks: i32,
        qty_left: u32,
        age: f32,
        since_improve: f32,
        /// Market orders keep walking to get done; limit orders stop at the limit.
        is_market: bool,
    },
    /// Aggressor pointing at the counterparty he just traded with ("SOLD!" / "BUY 'EM!").
    Hitting {
        counterparty: usize,
        side: Side,
        remaining: f32,
    },
    /// Passive side acknowledging the aggressor (point back, nod).
    Confirming {
        counterparty: usize,
        side: Side,
        remaining: f32,
    },
    /// Writing the trade on the card.
    Endorsing { remaining: f32 },
    /// Was at the best price and did not get the trade. Hands drop.
    Missed { remaining: f32 },
    /// Two people both think they did the trade.
    Disputing { counterparty: usize, remaining: f32 },
    /// Local has pulled quotes and stepped back: the vacuum.
    BackingAway { remaining: f32 },
    /// Runner sprinting a ticket out.
    Running { remaining: f32 },
    /// Reporter keying a print.
    Printing { remaining: f32, up: bool },
}

impl Phase {
    pub fn counterparty(&self) -> Option<usize> {
        match self {
            Phase::Hitting { counterparty, .. }
            | Phase::Confirming { counterparty, .. }
            | Phase::Disputing { counterparty, .. } => Some(*counterparty),
            _ => None,
        }
    }

    /// The side the hands are showing, if any.
    pub fn signalled_side(&self) -> Option<Side> {
        match self {
            Phase::Quoting { side, .. }
            | Phase::Working { side, .. }
            | Phase::Hitting { side, .. }
            | Phase::Confirming { side, .. } => Some(*side),
            _ => None,
        }
    }

    /// Is this participant adding to the noise of the pit right now?
    pub fn is_shouting(&self) -> bool {
        matches!(
            self,
            Phase::Quoting { .. }
                | Phase::Working { .. }
                | Phase::Hitting { .. }
                | Phase::Disputing { .. }
        )
    }

    pub fn is_quoting(&self) -> bool {
        matches!(self, Phase::Quoting { .. })
    }

    pub fn is_idle(&self) -> bool {
        matches!(self, Phase::Idle)
    }
}

#[derive(Debug, Clone)]
pub struct Participant {
    pub slot: usize,
    pub role: ParticipantRole,
    pub tier: Tier,
    pub phase: Phase,
    /// Net position in contracts. Locals only.
    pub inventory: i32,
    pub max_inventory: i32,
    /// 0..1, how readily this person hits rather than waits.
    pub aggressiveness: f32,
    /// Seconds between decisions at baseline. Middle step is faster than the outer step.
    pub latency: f32,
    /// 0..1, drifts; low attention means slower reactions and fewer fills.
    pub attention: f32,
    /// Grows with shouting, recovers while idle. Fatigue widens spreads and lengthens latency.
    pub fatigue: f32,
    /// Countdown to the next decision.
    pub next_decision: f32,
    /// Brokers' queue of customer orders not yet started.
    pub deck: VecDeque<(Side, u32, bool)>,
    pub trades: u32,
    pub pnl_ticks: f32,
    /// A broker's partially filled order to resume after pointing: (side, price, limit, qty_left, is_market).
    pub working_resume: Option<(Side, i32, i32, u32, bool)>,
}

impl Participant {
    fn new(slot: usize, rng: &mut XorShift) -> Self {
        let role = role_for_slot(slot);
        let tier = tier_for_slot(slot);
        let latency = match tier {
            Tier::Rail => 0.45,
            Tier::Pulpit => 0.30,
            Tier::Middle => 0.30 + rng.unit() * 0.25,
            Tier::Outer => 0.60 + rng.unit() * 0.40,
        };
        Self {
            slot,
            role,
            tier,
            phase: Phase::Idle,
            inventory: 0,
            max_inventory: 8 + (rng.unit() * 8.0) as i32,
            aggressiveness: 0.25 + rng.unit() * 0.6,
            latency,
            attention: 0.65 + rng.unit() * 0.35,
            fatigue: 0.0,
            next_decision: rng.unit() * latency,
            deck: VecDeque::new(),
            trades: 0,
            pnl_ticks: 0.0,
            working_resume: None,
        }
    }

    pub fn inventory_stress(&self) -> f32 {
        (self.inventory.abs() as f32 / self.max_inventory as f32).clamp(0.0, 1.0)
    }

    fn can_broker(&self) -> bool {
        matches!(
            self.role,
            ParticipantRole::LeadBroker | ParticipantRole::Broker
        )
    }
}

// ---------------------------------------------------------------------------
// Events
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PitEventKind {
    /// A customer order reached a broker.
    OrderArrival { broker: usize, side: Side, qty: u32 },
    /// Someone put a bid or offer into the pit (or a broker improved his price).
    Quote {
        slot: usize,
        side: Side,
        price_ticks: i32,
        qty: u32,
    },
    /// A quote was pulled or expired.
    Withdraw { slot: usize },
    /// A trade. `side` is the aggressor's side.
    Fill {
        aggressor: usize,
        passive: usize,
        side: Side,
        price_ticks: i32,
        qty: u32,
    },
    /// Was at the best price but did not get the trade.
    Missed { slot: usize },
    /// The reporter recorded a new price.
    Print { price_ticks: i32, up: bool },
    /// Two participants both think they traded.
    OutTrade { a: usize, b: usize },
    /// Trading halted for news.
    NewsPause { seconds: f32 },
    /// Fast-market condition changed.
    FastMarket { on: bool },
    /// Local stepped back and pulled quotes.
    BackAway { slot: usize },
}

#[derive(Debug, Clone, Copy)]
pub struct PitEvent {
    pub t: f32,
    pub kind: PitEventKind,
}

// ---------------------------------------------------------------------------
// Flow generator parameters (what a scenario controls)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FlowParams {
    /// Customer orders per second at activity 1.0.
    pub arrival_rate: f32,
    /// Probability that an arriving order is a buy.
    pub buy_prob: f32,
    /// Mean order size in contracts.
    pub mean_qty: f32,
    /// Probability that an order is a block (about 6x mean).
    pub block_prob: f32,
    /// How eagerly locals requote when idle, 0..1.
    pub local_quote_rate: f32,
    /// Seconds of silence remaining (news pause).
    pub news_pause: f32,
}

impl FlowParams {
    pub fn baseline(inst: Instrument) -> Self {
        // Mean quantities differ by pit so block sizes look right on the cards.
        let mean_qty = match inst {
            Instrument::Sp500 | Instrument::Nasdaq | Instrument::Us30 | Instrument::Uk100 => 6.0,
            Instrument::Gold | Instrument::Silver => 4.0,
            Instrument::Bitcoin => 2.0,
            _ => 5.0,
        };
        Self {
            arrival_rate: 0.9,
            buy_prob: 0.5,
            mean_qty,
            block_prob: 0.05,
            local_quote_rate: 0.55,
            news_pause: 0.0,
        }
    }

    /// Relax toward another parameter set with the given rate (1/s).
    pub fn relax_toward(&mut self, target: &FlowParams, dt: f32, rate: f32) {
        let k = (dt * rate).min(1.0);
        self.arrival_rate += (target.arrival_rate - self.arrival_rate) * k;
        self.buy_prob += (target.buy_prob - self.buy_prob) * k;
        self.mean_qty += (target.mean_qty - self.mean_qty) * k;
        self.block_prob += (target.block_prob - self.block_prob) * k;
        self.local_quote_rate += (target.local_quote_rate - self.local_quote_rate) * k;
    }
}

/// Tier A flow: what the real tape says customers are doing. While a live feed drives a pit
/// this replaces the generator's arrival rate, side bias and block odds, so the paper the
/// brokers shout is the paper the market actually saw.
/// What the crowd takes from the desk's read (Jev or the floor rule): a lean to respect and
/// how much to trust it. It widens quotes against the lean; it never moves price.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct FloorRead {
    /// -1 sellers .. 1 buyers.
    pub lean: f32,
    /// 0..1.
    pub confidence: f32,
    /// 0..1: the side in control is running out.
    pub exhaustion: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LiveFlow {
    /// Customer orders per second, from the tick rate.
    pub arrivals_per_sec: f32,
    /// Probability an arriving order is a buy, from the tick rule.
    pub buy_prob: f32,
    /// Probability an order is a block, from how far single ticks are jumping.
    pub block_prob: f32,
}

// ---------------------------------------------------------------------------
// Derived signals (what the views and HUD read)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct PitSignals {
    pub last_ticks: i32,
    pub prev_print_ticks: i32,
    pub best_bid_ticks: Option<i32>,
    pub best_ask_ticks: Option<i32>,
    /// Smoothed order arrivals per second.
    pub arrival_intensity: f32,
    /// Smoothed fills per second.
    pub fill_rate: f32,
    /// Smoothed signed fill flow, -1..1. Positive = buyers lifting offers.
    pub imbalance: f32,
    /// Quotes outstanding per active local, 0..1. Depth proxy.
    pub unfilled_interest: f32,
    /// Active slots / 18.
    pub participation: f32,
    /// 0..1. Shouting participants plus arrivals plus fill bursts. Leads price.
    pub sound: f32,
    /// Mean local inventory stress, 0..1.
    pub inventory_stress: f32,
    /// Smoothed absolute fair-value change in ticks per second.
    pub volatility: f32,
    pub fast_market: bool,
    /// 0..1 fraction of active locals currently backing away.
    pub vacuum: f32,
    /// Signed unfilled broker intent, -1..1: what the rail is trying to do, filled or not.
    pub pressure: f32,
}

// ---------------------------------------------------------------------------
// RNG
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
pub struct XorShift(u64);

impl XorShift {
    pub fn new(seed: u64) -> Self {
        Self(seed.max(1) ^ 0x9E37_79B9_7F4A_7C15)
    }

    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// Uniform in [0, 1).
    pub fn unit(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32
    }

    pub fn chance(&mut self, p: f32) -> bool {
        self.unit() < p
    }

    pub fn range_usize(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            (self.next_u64() % n as u64) as usize
        }
    }

    /// Approximate Poisson draw for small means (rate*dt well under 5).
    pub fn poisson(&mut self, mean: f32) -> u32 {
        if mean <= 0.0 {
            return 0;
        }
        let l = (-mean).exp();
        let mut k = 0u32;
        let mut p = 1.0f32;
        loop {
            p *= self.unit();
            if p <= l || k > 12 {
                return k;
            }
            k += 1;
        }
    }
}

// ---------------------------------------------------------------------------
// The pit market
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct PitMarket {
    pub instrument: Instrument,
    pub participants: Vec<Participant>,
    pub params: FlowParams,
    pub baseline: FlowParams,
    pub signals: PitSignals,
    /// Full event ring (newest at back).
    pub events: VecDeque<PitEvent>,
    /// Events emitted since the last `begin_frame`.
    recent: Vec<PitEvent>,
    pub clock: f32,
    pub active_count: usize,
    /// Fair value in ticks relative to the instrument's base price.
    pub fair_ticks: f32,
    prev_fair_ticks: f32,
    /// Real price in ticks when a live feed drives this pit; fair value is pulled hard to it.
    pub live_anchor: Option<f32>,
    /// Real flow read off the tape; `None` means the generator decides.
    pub live_flow: Option<LiveFlow>,
    /// The desk's read of this pit, while one is fresh.
    pub floor_read: Option<FloorRead>,
    rng: XorShift,
    /// Scenario shock relaxation rate (1/s). Zero means the preset is permanent.
    relax_rate: f32,
    arrivals_this_window: f32,
    fills_this_window: f32,
    signed_fill_volume: f32,
    abs_fill_volume: f32,
    fill_burst: f32,
}

impl PitMarket {
    pub fn new(instrument: Instrument, seed: u64) -> Self {
        let mut rng = XorShift::new(seed);
        let participants = (0..SLOTS).map(|s| Participant::new(s, &mut rng)).collect();
        let baseline = FlowParams::baseline(instrument);
        Self {
            instrument,
            participants,
            params: baseline,
            baseline,
            signals: PitSignals::default(),
            events: VecDeque::with_capacity(EVENT_RING_CAP),
            recent: Vec::new(),
            clock: 0.0,
            active_count: SLOTS,
            fair_ticks: 0.0,
            prev_fair_ticks: 0.0,
            live_anchor: None,
            live_flow: None,
            floor_read: None,
            rng,
            relax_rate: 0.0,
            arrivals_this_window: 0.0,
            fills_this_window: 0.0,
            signed_fill_volume: 0.0,
            abs_fill_volume: 0.0,
            fill_burst: 0.0,
        }
    }

    /// Price in instrument units for a tick count.
    pub fn price_of(&self, ticks: i32) -> f32 {
        self.instrument.base_price() + ticks as f32 * tick_size(self.instrument)
    }

    /// Inverse of `price_of`: the tick count for a real price (ticks are offset from the
    /// instrument's base price, not from zero).
    pub fn ticks_for_price(&self, price: f32) -> i32 {
        ((price - self.instrument.base_price()) / tick_size(self.instrument)).round() as i32
    }

    pub fn last_price(&self) -> f32 {
        self.price_of(self.signals.last_ticks)
    }

    /// The digit a trader would flash for a price: last significant digit.
    pub fn price_digit(&self, ticks: i32) -> u32 {
        ticks.rem_euclid(10) as u32
    }

    pub fn recent_events(&self) -> &[PitEvent] {
        &self.recent
    }

    pub fn participant(&self, slot: usize) -> &Participant {
        &self.participants[slot.min(SLOTS - 1)]
    }

    pub fn is_active(&self, slot: usize) -> bool {
        slot < self.active_count
    }

    /// Set how many slots are present. Slots 0 and 1 are always present.
    pub fn set_active_count(&mut self, n: usize) {
        let n = n.clamp(3, SLOTS);
        if n == self.active_count {
            return;
        }
        self.active_count = n;
        for slot in n..SLOTS {
            if !self.participants[slot].phase.is_idle() {
                self.participants[slot].phase = Phase::Idle;
                self.participants[slot].deck.clear();
                self.participants[slot].working_resume = None;
            }
        }
        // Anyone whose counterparty just left stops pointing at them.
        for slot in 0..n {
            if let Some(cp) = self.participants[slot].phase.counterparty()
                && cp >= n
            {
                self.participants[slot].phase = Phase::Endorsing { remaining: 1.0 };
            }
        }
    }

    /// Apply a scenario preset. `relax_rate` > 0 makes the preset decay back to baseline.
    pub fn apply_params(&mut self, params: FlowParams, relax_rate: f32) {
        if params.news_pause > 0.0 && self.params.news_pause <= 0.0 {
            self.push_event(PitEventKind::NewsPause {
                seconds: params.news_pause,
            });
            // Everyone pulls quotes and freezes on the news.
            for p in self.participants.iter_mut() {
                if p.phase.is_quoting() {
                    p.phase = Phase::BackingAway {
                        remaining: params.news_pause + 0.5,
                    };
                }
            }
        }
        self.params = params;
        self.relax_rate = relax_rate;
    }

    /// Inject a one-off burst of one-sided flow onto the lead broker (the "shock" key).
    pub fn inject_shock(&mut self, side: Side, blocks: u32) {
        let mean = self.params.mean_qty.max(1.0);
        for _ in 0..blocks {
            let qty = (mean * (4.0 + self.rng.unit() * 4.0)).round() as u32;
            self.participants[0]
                .deck
                .push_back((side, qty.max(1), true));
        }
        self.params.arrival_rate = (self.params.arrival_rate * 2.5).max(2.0);
        self.params.buy_prob = if side.is_buy() { 0.88 } else { 0.12 };
        self.params.block_prob = 0.35;
        if self.relax_rate <= 0.0 {
            self.relax_rate = 0.02;
        }
    }

    fn push_event(&mut self, kind: PitEventKind) {
        let ev = PitEvent {
            t: self.clock,
            kind,
        };
        if self.events.len() >= EVENT_RING_CAP {
            self.events.pop_front();
        }
        self.events.push_back(ev);
        self.recent.push(ev);
    }

    /// Anchor fair value to a live price. A first anchor, or a gap the pit could not have
    /// traded through, jumps the whole pit; a tick-by-tick move is followed smoothly.
    pub fn set_live_anchor(&mut self, ticks: i32) {
        let t = ticks as f32;
        let jump = match self.live_anchor {
            None => true,
            Some(_) => (t - self.fair_ticks).abs() > LIVE_JUMP_TICKS,
        };
        if jump {
            self.jump_to(ticks);
        }
        self.live_anchor = Some(t);
    }

    pub fn clear_live_anchor(&mut self) {
        self.live_anchor = None;
        self.live_flow = None;
    }

    /// Hand the pit the tape's read of the flow (or take it away).
    pub fn set_live_flow(&mut self, flow: Option<LiveFlow>) {
        self.live_flow = flow;
    }

    /// Hand the pit the desk's read (or take it away when it goes stale).
    pub fn set_floor_read(&mut self, read: Option<FloorRead>) {
        self.floor_read = read;
    }

    /// Move the pit to a new price without trading through the gap: everyone pulls their
    /// quotes and the tape restarts there, so the stale book cannot be swept.
    pub fn jump_to(&mut self, ticks: i32) {
        self.fair_ticks = ticks as f32;
        self.prev_fair_ticks = ticks as f32;
        self.signals.last_ticks = ticks;
        self.signals.prev_print_ticks = ticks;
        self.signals.best_bid_ticks = None;
        self.signals.best_ask_ticks = None;
        for p in self.participants.iter_mut() {
            p.phase = Phase::Idle;
            p.deck.clear();
            p.working_resume = None;
        }
    }

    /// Call once per rendered frame before `step`. Clears the recent-event list.
    pub fn begin_frame(&mut self) {
        self.recent.clear();
    }

    /// Advance the pit by `dt` seconds. `activity` (0..1) scales the arrival rate.
    pub fn step(&mut self, dt: f32, activity: f32) {
        let mut left = dt.max(0.0);
        while left > 0.0 {
            let h = left.min(MAX_SUBSTEP);
            self.substep(h, activity.clamp(0.0, 1.0));
            left -= h;
        }
    }

    fn substep(&mut self, dt: f32, activity: f32) {
        self.clock += dt;

        // Scenario relaxation and news pause.
        if self.relax_rate > 0.0 {
            let base = self.baseline;
            self.params.relax_toward(&base, dt, self.relax_rate);
        }
        if self.params.news_pause > 0.0 {
            self.params.news_pause = (self.params.news_pause - dt).max(0.0);
        }
        let paused = self.params.news_pause > 0.0;

        // 1. Customer order arrivals.
        if !paused {
            let scale = 0.15 + activity * 1.25;
            let lambda = match self.live_flow {
                Some(flow) => flow.arrivals_per_sec,
                None => self.params.arrival_rate * scale,
            };
            let n = self.rng.poisson(lambda * dt);
            for _ in 0..n {
                self.arrive_order();
            }
        }

        // 2. Participant decisions.
        for slot in 0..self.active_count {
            self.tick_participant(slot, dt, paused);
        }

        // 3. Fatigue and attention drift for everyone active. Against a live screen locals
        // also lay inventory off electronically, a lot at a time, so a one-way tape does
        // not leave them all short and on strike: the real market kept filling the buyers,
        // so someone in this pit has to keep offering.
        let live = self.live_anchor.is_some();
        for slot in 0..self.active_count {
            let inv = self.participants[slot].inventory;
            if live && self.participants[slot].role == ParticipantRole::Local && inv != 0 {
                let lay_off_rate = (inv.abs() as f32 * LIVE_LAY_OFF_PER_SEC).max(0.3);
                if self.rng.chance((dt * lay_off_rate).min(1.0)) {
                    self.participants[slot].inventory -= inv.signum();
                }
            }
            let p = &mut self.participants[slot];
            if p.phase.is_shouting() {
                p.fatigue = (p.fatigue + dt * 0.03).min(1.0);
            } else {
                p.fatigue = (p.fatigue - dt * 0.02).max(0.0);
            }
            let wobble = ((self.clock * 0.17 + slot as f32 * 1.3).sin()) * 0.15;
            p.attention = (0.75 + wobble - p.fatigue * 0.3).clamp(0.2, 1.0);
        }

        // 4. Signals.
        self.update_signals(dt);
    }

    fn arrive_order(&mut self) {
        let (buy_prob, block_prob) = match self.live_flow {
            Some(flow) => (flow.buy_prob, flow.block_prob),
            None => (self.params.buy_prob, self.params.block_prob),
        };
        let side = if self.rng.chance(buy_prob) {
            Side::Buy
        } else {
            Side::Sell
        };
        let mean = self.params.mean_qty.max(1.0);
        let is_block = self.rng.chance(block_prob);
        let qty = if is_block {
            (mean * (5.0 + self.rng.unit() * 3.0)).round() as u32
        } else {
            (mean * (0.4 + self.rng.unit() * 1.4)).round().max(1.0) as u32
        };

        let brokers: Vec<usize> = (0..self.active_count)
            .filter(|&s| self.participants[s].can_broker())
            .collect();
        if brokers.is_empty() {
            return;
        }
        // Blocks go to the rail. Otherwise the desk hands the order to a free broker,
        // falling back to the shortest deck.
        let is_market = self.rng.chance(0.7);
        let broker = if is_block && self.participants[0].deck.len() < 2 {
            0
        } else {
            let idle: Vec<usize> = brokers
                .iter()
                .copied()
                .filter(|&b| {
                    self.participants[b].phase.is_idle() && self.participants[b].deck.is_empty()
                })
                .collect();
            if !idle.is_empty() {
                idle[self.rng.range_usize(idle.len())]
            } else {
                *brokers
                    .iter()
                    .min_by_key(|&&b| self.participants[b].deck.len())
                    .unwrap_or(&0)
            }
        };

        let p = &mut self.participants[broker];
        let cap = if broker == 0 { 8 } else { 5 };
        if p.deck.len() < cap {
            p.deck.push_back((side, qty, is_market));
        }
        self.arrivals_this_window += 1.0;
        self.push_event(PitEventKind::OrderArrival { broker, side, qty });
    }

    fn tick_participant(&mut self, slot: usize, dt: f32, paused: bool) {
        // Time-based phase transitions first.
        let phase = self.participants[slot].phase;
        match phase {
            Phase::Idle => {}
            Phase::Quoting {
                side,
                price_ticks,
                qty,
                remaining,
                age,
            } => {
                let remaining = remaining - dt;
                if remaining <= 0.0 {
                    self.participants[slot].phase = Phase::Idle;
                    self.push_event(PitEventKind::Withdraw { slot });
                } else {
                    self.participants[slot].phase = Phase::Quoting {
                        side,
                        price_ticks,
                        qty,
                        remaining,
                        age: age + dt,
                    };
                }
            }
            Phase::Working {
                side,
                price_ticks,
                limit_ticks,
                qty_left,
                age,
                since_improve,
                is_market,
            } => {
                self.participants[slot].phase = Phase::Working {
                    side,
                    price_ticks,
                    limit_ticks,
                    qty_left,
                    age: age + dt,
                    since_improve: since_improve + dt,
                    is_market,
                };
            }
            Phase::Hitting {
                counterparty,
                side,
                remaining,
            } => {
                let remaining = remaining - dt;
                if remaining <= 0.0 {
                    // A broker with lots left on the card goes straight back to shouting;
                    // everyone else writes the trade up.
                    let endorse = 0.7 + self.rng.unit() * 0.4;
                    let p = &mut self.participants[slot];
                    p.phase = match p.working_resume.take() {
                        Some((ws, px, lim, left, is_market)) if left > 0 => Phase::Working {
                            side: ws,
                            price_ticks: px,
                            limit_ticks: lim,
                            qty_left: left,
                            age: 0.5,
                            since_improve: 0.0,
                            is_market,
                        },
                        _ => Phase::Endorsing { remaining: endorse },
                    };
                } else {
                    self.participants[slot].phase = Phase::Hitting {
                        counterparty,
                        side,
                        remaining,
                    };
                }
            }
            Phase::Confirming {
                counterparty,
                side,
                remaining,
            } => {
                let remaining = remaining - dt;
                self.participants[slot].phase = if remaining <= 0.0 {
                    Phase::Endorsing {
                        remaining: 0.7 + self.rng.unit() * 0.5,
                    }
                } else {
                    Phase::Confirming {
                        counterparty,
                        side,
                        remaining,
                    }
                };
            }
            Phase::Endorsing { remaining } => {
                let remaining = remaining - dt;
                self.participants[slot].phase = if remaining <= 0.0 {
                    Phase::Idle
                } else {
                    Phase::Endorsing { remaining }
                };
            }
            Phase::Missed { remaining }
            | Phase::BackingAway { remaining }
            | Phase::Running { remaining } => {
                let remaining = remaining - dt;
                if remaining <= 0.0 {
                    self.participants[slot].phase = Phase::Idle;
                } else {
                    self.participants[slot].phase = match phase {
                        Phase::Missed { .. } => Phase::Missed { remaining },
                        Phase::BackingAway { .. } => Phase::BackingAway { remaining },
                        _ => Phase::Running { remaining },
                    };
                }
            }
            Phase::Disputing {
                counterparty,
                remaining,
            } => {
                let remaining = remaining - dt;
                self.participants[slot].phase = if remaining <= 0.0 {
                    Phase::Endorsing { remaining: 1.0 }
                } else {
                    Phase::Disputing {
                        counterparty,
                        remaining,
                    }
                };
            }
            Phase::Printing { remaining, up } => {
                let remaining = remaining - dt;
                self.participants[slot].phase = if remaining <= 0.0 {
                    Phase::Idle
                } else {
                    Phase::Printing { remaining, up }
                };
            }
        }

        // Decision clock.
        let p = &mut self.participants[slot];
        p.next_decision -= dt;
        if p.next_decision > 0.0 {
            return;
        }
        let fatigue = p.fatigue;
        let attention = p.attention;
        let latency = p.latency * (1.0 + fatigue * 0.8) / attention.max(0.3);
        p.next_decision = latency * (0.6 + self.rng.unit() * 0.8);

        if paused {
            return;
        }

        match self.participants[slot].role {
            ParticipantRole::Local => self.decide_local(slot),
            ParticipantRole::LeadBroker | ParticipantRole::Broker => self.decide_broker(slot),
            ParticipantRole::Runner => self.decide_runner(slot),
            ParticipantRole::Reporter => {}
        }
    }

    // ----- Locals -----------------------------------------------------------

    fn half_spread_ticks(&self, p: &Participant) -> f32 {
        let vol = self.signals.volatility;
        let fast = if self.signals.fast_market { 2.0 } else { 0.0 };
        1.0 + vol * 0.6 + fast + p.fatigue * 1.0 + p.inventory_stress() * 1.5
    }

    fn decide_local(&mut self, slot: usize) {
        let phase = self.participants[slot].phase;
        match phase {
            Phase::Idle => {
                // Fast market: a local on the wrong side of the move steps back; one on the
                // right side stays in to get paid.
                let stress = self.participants[slot].inventory_stress();
                let inv = self.participants[slot].inventory;
                let wrong_footed = (inv > 0 && self.signals.pressure < -0.2)
                    || (inv < 0 && self.signals.pressure > 0.2);
                let back_chance = if wrong_footed {
                    0.12 + stress * 0.5
                } else {
                    0.05
                };
                if self.signals.fast_market && self.rng.chance(back_chance) {
                    self.participants[slot].phase = Phase::BackingAway {
                        remaining: 2.0 + self.rng.unit() * 2.5,
                    };
                    self.push_event(PitEventKind::BackAway { slot });
                    return;
                }

                // Flatten a heavy position by hitting the other side's quote.
                if stress > 0.6 {
                    let reduce = if self.participants[slot].inventory > 0 {
                        Side::Sell
                    } else {
                        Side::Buy
                    };
                    if self.try_aggress(slot, reduce, None) {
                        return;
                    }
                }

                // Scalp a stale quote (local-vs-local) if someone is through fair value.
                if self
                    .rng
                    .chance(self.participants[slot].aggressiveness * 0.35)
                {
                    let fair = self.fair_ticks;
                    let stale_ask = self
                        .best_quote(Side::Sell)
                        .filter(|&(_, px)| (px as f32) < fair - 0.6);
                    let stale_bid = self
                        .best_quote(Side::Buy)
                        .filter(|&(_, px)| (px as f32) > fair + 0.6);
                    if stale_ask.is_some() && self.try_aggress(slot, Side::Buy, None) {
                        return;
                    }
                    if stale_bid.is_some() && self.try_aggress(slot, Side::Sell, None) {
                        return;
                    }
                }

                // Otherwise show a quote. Heavy flow makes locals keener: that is the business.
                let flow_boost = 1.0 + (self.signals.arrival_intensity / 3.0).clamp(0.0, 1.0);
                let want = self.params.local_quote_rate
                    * (0.5 + self.participants[slot].attention * 0.5)
                    * flow_boost;
                if !self.rng.chance(want.min(0.95)) {
                    return;
                }
                self.post_local_quote(slot);
            }
            Phase::Quoting {
                side,
                price_ticks,
                qty,
                ..
            } => {
                // Re-price if fair moved away from the quote.
                let fair = self.fair_ticks;
                let off = match side {
                    Side::Buy => fair - price_ticks as f32,
                    Side::Sell => price_ticks as f32 - fair,
                };
                // Judge the quote by the spread it was posted at: a quote widened against
                // one-way flow is meant to sit there until the paper pays up.
                let half = self.quoted_half_spread(slot, side);
                if off < half * 0.4 || off > half * 2.2 {
                    // Quote is through fair value or hopelessly far: pull it and requote later.
                    self.participants[slot].phase = Phase::Idle;
                    self.push_event(PitEventKind::Withdraw { slot });
                } else if qty == 0 {
                    self.participants[slot].phase = Phase::Idle;
                }
            }
            _ => {}
        }
    }

    /// Half spread a local shows on `side` right now: the base spread, plus a premium for
    /// standing against one-way paper (that is what the flow pays for).
    fn quoted_half_spread(&self, slot: usize, side: Side) -> f32 {
        let pressure = self.signals.pressure;
        let against_flow =
            (side == Side::Buy && pressure < 0.0) || (side == Side::Sell && pressure > 0.0);
        // Against the screen the premium is capped: the real market says where the offer is.
        let premium = if self.live_anchor.is_some() { 0.8 } else { 2.0 };
        // A local standing against a confident read wants paying for it too: up to a tick.
        let against_read = self
            .floor_read
            .map(|r| {
                let against =
                    (side == Side::Buy && r.lean < 0.0) || (side == Side::Sell && r.lean > 0.0);
                if against {
                    (r.lean.abs() * r.confidence).min(1.0)
                } else {
                    0.0
                }
            })
            .unwrap_or(0.0);
        self.half_spread_ticks(&self.participants[slot])
            + if against_flow {
                pressure.abs() * premium
            } else {
                0.0
            }
            + against_read
    }

    fn post_local_quote(&mut self, slot: usize) {
        let p = &self.participants[slot];
        let inv = p.inventory;
        let stress = p.inventory_stress();
        // Inventory skew: long locals lean to selling, short locals to buying.
        // Side: reduce a heavy position; otherwise lean toward the side the flow will hit,
        // because that is where the business is, and charge more for it.
        let pressure = self.signals.pressure;
        let side = if stress > 0.35 {
            if inv > 0 { Side::Sell } else { Side::Buy }
        } else if self.rng.chance((0.5 - pressure * 0.3).clamp(0.15, 0.85)) {
            Side::Buy
        } else {
            Side::Sell
        };
        let half = self.quoted_half_spread(slot, side);
        let p = &self.participants[slot];
        let skew = (inv as f32 / p.max_inventory as f32) * 1.5;
        let mut px = match side {
            Side::Buy => (self.fair_ticks - half - skew).floor() as i32,
            Side::Sell => (self.fair_ticks + half - skew).ceil() as i32,
        };
        // Against a live screen the real market says where the touch is: locals quote
        // within a couple of ticks of it and express their lean in side and size instead.
        if self.live_anchor.is_some() {
            let fair = self.fair_ticks.round() as i32;
            px = match side {
                Side::Buy => px.clamp(fair - LIVE_QUOTE_BAND_TICKS, fair - 1),
                Side::Sell => px.clamp(fair + 1, fair + LIVE_QUOTE_BAND_TICKS),
            };
        }
        let heavy = self.signals.fast_market || self.signals.arrival_intensity > 1.5;
        let mut qty = 2 + self.rng.range_usize(5) as u32 + if stress > 0.5 { 2 } else { 0 };
        if heavy {
            qty *= 2;
        }
        let remaining = 2.5 + self.rng.unit() * 3.5;
        self.participants[slot].phase = Phase::Quoting {
            side,
            price_ticks: px,
            qty,
            remaining,
            age: 0.0,
        };
        self.push_event(PitEventKind::Quote {
            slot,
            side,
            price_ticks: px,
            qty,
        });
    }

    // ----- Brokers ----------------------------------------------------------

    fn decide_broker(&mut self, slot: usize) {
        let phase = self.participants[slot].phase;
        match phase {
            Phase::Idle => {
                // Nothing of my own: take an order off the most loaded colleague's deck.
                if self.participants[slot].deck.is_empty() {
                    let donor = (0..self.active_count)
                        .filter(|&b| {
                            b != slot
                                && self.participants[b].can_broker()
                                && self.participants[b].deck.len() >= 2
                        })
                        .max_by_key(|&b| self.participants[b].deck.len());
                    if let Some(d) = donor
                        && let Some(order) = self.participants[d].deck.pop_back()
                    {
                        self.participants[slot].deck.push_back(order);
                    }
                }
                let Some((side, qty, is_market)) = self.participants[slot].deck.pop_front() else {
                    return;
                };
                // Start at the touch if there is one, else at fair, then walk toward the limit.
                let fair = self.fair_ticks;
                let touch = self.best_quote(side.opposite()).map(|(_, px)| px);
                let start = match (side, touch) {
                    (Side::Buy, Some(ask)) => ask.min(fair.round() as i32 + 1),
                    (Side::Sell, Some(bid)) => bid.max(fair.round() as i32 - 1),
                    (Side::Buy, None) => fair.floor() as i32,
                    (Side::Sell, None) => fair.ceil() as i32,
                };
                let mean = self.params.mean_qty.max(1.0);
                let reach = if is_market {
                    400
                } else if qty as f32 > mean * 3.0 {
                    3
                } else {
                    2
                };
                let limit = start + side.sign() * reach;
                self.participants[slot].phase = Phase::Working {
                    side,
                    price_ticks: start,
                    limit_ticks: limit,
                    qty_left: qty,
                    age: 0.0,
                    since_improve: 0.0,
                    is_market,
                };
                self.push_event(PitEventKind::Quote {
                    slot,
                    side,
                    price_ticks: start,
                    qty,
                });
            }
            Phase::Working {
                side,
                price_ticks,
                limit_ticks,
                qty_left,
                age,
                since_improve,
                is_market,
            } => {
                // Shout for at least half a second before anyone can fill: hands before prints.
                if age < 0.5 {
                    return;
                }
                if self.try_aggress(slot, side, Some(price_ticks)) {
                    return;
                }
                // Nobody on the other side at this price: walk it.
                let at_limit = match side {
                    Side::Buy => price_ticks >= limit_ticks,
                    Side::Sell => price_ticks <= limit_ticks,
                };
                let walk_every = if is_market { 0.8 } else { 1.4 };
                if is_market && age > 14.0 {
                    // Even a market order gets pulled when the pit will not trade.
                    self.participants[slot].phase = Phase::Endorsing { remaining: 0.6 };
                    self.push_event(PitEventKind::Withdraw { slot });
                    return;
                }
                if !at_limit && since_improve > walk_every {
                    // A market order goes straight to the touch; a limit order walks a tick.
                    let touch = self.best_quote(side.opposite()).map(|(_, px)| px);
                    let px = match (is_market, touch) {
                        (true, Some(t)) => {
                            let within = match side {
                                Side::Buy => t <= limit_ticks,
                                Side::Sell => t >= limit_ticks,
                            };
                            if within { t } else { price_ticks + side.sign() }
                        }
                        _ => price_ticks + side.sign(),
                    };
                    // A one-tick walk into empty space moves the pit's idea of value; a jump to a
                    // standing quote does not, the fill that follows does.
                    let walked_into_space = touch.is_none() || px == price_ticks + side.sign();
                    self.participants[slot].phase = Phase::Working {
                        side,
                        price_ticks: px,
                        limit_ticks,
                        qty_left,
                        age,
                        since_improve: 0.0,
                        is_market,
                    };
                    self.push_event(PitEventKind::Quote {
                        slot,
                        side,
                        price_ticks: px,
                        qty: qty_left,
                    });
                    // The pit hears a bid being walked: fair value follows, no print yet.
                    if walked_into_space {
                        self.fair_ticks += side.sign() as f32 * 0.4;
                    }
                } else if at_limit {
                    // Unfilled at the limit: pressure leaks into fair value (vacuum), then give up.
                    self.fair_ticks += side.sign() as f32 * 0.12;
                    let patience = if is_market { 12.0 } else { 4.0 };
                    if age > patience {
                        self.participants[slot].phase = Phase::Endorsing { remaining: 0.6 };
                        self.push_event(PitEventKind::Withdraw { slot });
                    }
                }
            }
            _ => {}
        }
    }

    fn decide_runner(&mut self, slot: usize) {
        if !self.participants[slot].phase.is_idle() {
            return;
        }
        // Runners go when a broker has just finished a card.
        let someone_endorsing = (0..self.active_count).any(|s| {
            self.participants[s].can_broker()
                && matches!(self.participants[s].phase, Phase::Endorsing { .. })
        });
        if someone_endorsing && self.rng.chance(0.5) {
            self.participants[slot].phase = Phase::Running {
                remaining: 2.5 + self.rng.unit() * 2.0,
            };
        }
    }

    // ----- Matching ---------------------------------------------------------

    /// Best resting quote on `side`: (slot, price). Best bid is highest, best ask lowest.
    fn best_quote(&self, side: Side) -> Option<(usize, i32)> {
        let mut best: Option<(usize, i32)> = None;
        for s in 0..self.active_count {
            if let Phase::Quoting {
                side: qs,
                price_ticks,
                qty,
                ..
            } = self.participants[s].phase
            {
                if qs != side || qty == 0 {
                    continue;
                }
                best = match best {
                    None => Some((s, price_ticks)),
                    Some((_, bp)) => {
                        let better = match side {
                            Side::Buy => price_ticks > bp,
                            Side::Sell => price_ticks < bp,
                        };
                        if better { Some((s, price_ticks)) } else { best }
                    }
                };
            }
        }
        best
    }

    /// `aggressor` tries to trade on `side` against resting quotes. `limit` caps the price
    /// (None = take whatever is best). Returns true if a fill happened.
    fn try_aggress(&mut self, aggressor: usize, side: Side, limit: Option<i32>) -> bool {
        let Some((_, best_px)) = self.best_quote(side.opposite()) else {
            return false;
        };
        let within_limit = |px: i32| match limit {
            None => true,
            Some(lim) => match side {
                Side::Buy => px <= lim,
                Side::Sell => px >= lim,
            },
        };
        if !within_limit(best_px) {
            return false;
        }

        // How much the aggressor wants. Brokers sweep the whole card; locals take a few.
        let (mut qty_left, sweeping) = match self.participants[aggressor].phase {
            Phase::Working { qty_left, .. } => (qty_left, true),
            _ => (1 + self.rng.range_usize(3) as u32, false),
        };
        if qty_left == 0 {
            return false;
        }

        // Candidates: everyone resting within the limit, up to two ticks from the best.
        // Each gets an exponential race key weighted by attention, tier and proximity,
        // so allocation is price priority first, then human noise.
        let noisy = self.rng.chance(0.12);
        let mut candidates: Vec<(usize, i32, u32, f32)> = Vec::new();
        for s in 0..self.active_count {
            if s == aggressor {
                continue;
            }
            if let Phase::Quoting {
                side: qs,
                price_ticks,
                qty,
                ..
            } = self.participants[s].phase
            {
                if qs != side.opposite() || qty == 0 || !within_limit(price_ticks) {
                    continue;
                }
                let worse_by = (price_ticks - best_px) * side.sign();
                if worse_by > 2 || (worse_by == 2 && !sweeping) {
                    continue;
                }
                let p = &self.participants[s];
                let tier_w = match p.tier {
                    Tier::Middle => 1.0,
                    Tier::Outer => 0.6,
                    _ => 0.8,
                };
                let prox =
                    1.0 / (1.0 + slot_angular_distance(aggressor, s) / std::f32::consts::PI * 3.0);
                let w = (p.attention * tier_w * prox).max(0.02);
                let race = -(self.rng.unit().max(1e-4)).ln() / w;
                // Noise: occasionally a worse price wins the race against the best.
                let level = if noisy { 0 } else { worse_by };
                candidates.push((s, price_ticks, qty, level as f32 * 10.0 + race));
            }
        }
        if candidates.is_empty() {
            return false;
        }
        candidates.sort_by(|a, b| a.3.partial_cmp(&b.3).unwrap_or(std::cmp::Ordering::Equal));

        let mut filled: Vec<usize> = Vec::new();
        for &(passive, px, pqty, _) in &candidates {
            if qty_left == 0 {
                break;
            }
            let q = qty_left.min(pqty).max(1);
            self.execute_fill(aggressor, passive, side, px, q);
            filled.push(passive);
            qty_left -= q;
            if !sweeping {
                break;
            }
        }
        if filled.is_empty() {
            return false;
        }

        // Carry the remainder of a broker's card (execute_fill already trimmed it).
        if sweeping
            && let Some((_, _, _, left, _)) = self.participants[aggressor].working_resume
            && left == 0
        {
            self.participants[aggressor].working_resume = None;
        }

        // Everyone at the best price who did not trade missed it; one of them may dispute.
        let losers: Vec<usize> = candidates
            .iter()
            .filter(|c| c.1 == best_px && !filled.contains(&c.0))
            .map(|c| c.0)
            .collect();
        let mut out_trade_done = false;
        for l in losers {
            if !out_trade_done && self.rng.chance(0.04) {
                self.participants[l].phase = Phase::Disputing {
                    counterparty: aggressor,
                    remaining: 2.6,
                };
                self.push_event(PitEventKind::OutTrade { a: aggressor, b: l });
                out_trade_done = true;
            } else if self.rng.chance(0.7) {
                self.participants[l].phase = Phase::Missed {
                    remaining: 0.9 + self.rng.unit() * 0.5,
                };
                self.push_event(PitEventKind::Missed { slot: l });
            }
        }
        true
    }

    fn execute_fill(&mut self, aggressor: usize, passive: usize, side: Side, px: i32, qty: u32) {
        // Aggressor state. A broker keeps his card in `working_resume` so he can go back to
        // shouting after pointing; the caller trims the quantity.
        let hit_time = 0.55 + self.rng.unit() * 0.25;
        if let Phase::Working {
            side: ws,
            price_ticks,
            limit_ticks,
            qty_left,
            is_market,
            ..
        } = self.participants[aggressor].phase
        {
            self.participants[aggressor].working_resume = Some((
                ws,
                price_ticks,
                limit_ticks,
                qty_left.saturating_sub(qty),
                is_market,
            ));
        } else if let Some((ws, px, lim, left, is_market)) =
            self.participants[aggressor].working_resume
        {
            self.participants[aggressor].working_resume =
                Some((ws, px, lim, left.saturating_sub(qty), is_market));
        }
        self.participants[aggressor].phase = Phase::Hitting {
            counterparty: passive,
            side,
            remaining: hit_time,
        };

        // Passive state and inventory.
        let pas = &mut self.participants[passive];
        pas.phase = Phase::Confirming {
            counterparty: aggressor,
            side: side.opposite(),
            remaining: 0.45 + self.rng.unit() * 0.25,
        };
        if pas.role == ParticipantRole::Local {
            pas.inventory -= side.sign() * qty as i32;
            // Passive earns the half-spread relative to fair.
            let edge = (px as f32 - self.fair_ticks) * side.sign() as f32;
            pas.pnl_ticks += edge * qty as f32;
        }
        pas.trades += 1;
        let agg = &mut self.participants[aggressor];
        if agg.role == ParticipantRole::Local {
            agg.inventory += side.sign() * qty as i32;
            let edge = (self.fair_ticks - px as f32) * side.sign() as f32;
            agg.pnl_ticks += edge * qty as f32;
        }
        agg.trades += 1;

        // Price formation: the print is the fill; fair follows with impact.
        let mean = self.params.mean_qty.max(1.0);
        let impact = 0.45 * (qty as f32 / mean).sqrt();
        self.fair_ticks += (px as f32 - self.fair_ticks) * 0.5 + side.sign() as f32 * impact;

        self.fills_this_window += 1.0;
        self.signed_fill_volume += side.sign() as f32 * qty as f32;
        self.abs_fill_volume += qty as f32;
        self.fill_burst = (self.fill_burst + 0.35).min(1.5);

        self.push_event(PitEventKind::Fill {
            aggressor,
            passive,
            side,
            price_ticks: px,
            qty,
        });

        // Reporter prints a change.
        let prev = self.signals.last_ticks;
        self.signals.last_ticks = px;
        if px != self.signals.prev_print_ticks {
            let up = px > prev;
            self.signals.prev_print_ticks = px;
            if self.participants[1].role == ParticipantRole::Reporter {
                self.participants[1].phase = Phase::Printing { remaining: 0.8, up };
            }
            self.push_event(PitEventKind::Print {
                price_ticks: px,
                up,
            });
        }
    }

    // ----- Signals ----------------------------------------------------------

    fn update_signals(&mut self, dt: f32) {
        // Live feed: the real market owns fair value; the pit's own fills only jitter it.
        if let Some(anchor) = self.live_anchor {
            self.fair_ticks += (anchor - self.fair_ticks) * (dt * 10.0).min(1.0);
            // Quotes the screen has moved away from get pulled, as a local watching the
            // board would; otherwise the pit prints stale prices off the real market.
            let fair = self.fair_ticks;
            let mut pulled = Vec::new();
            for slot in 0..self.participants.len() {
                if let Phase::Quoting {
                    side, price_ticks, ..
                } = self.participants[slot].phase
                {
                    let off = match side {
                        Side::Buy => price_ticks as f32 - fair,
                        Side::Sell => fair - price_ticks as f32,
                    };
                    let stale = (fair - price_ticks as f32).abs() > LIVE_STALE_QUOTE_TICKS;
                    if off > 1.5 || stale {
                        self.participants[slot].phase = Phase::Endorsing { remaining: 0.3 };
                        pulled.push(slot);
                    }
                } else if let Phase::Working {
                    side,
                    ref mut price_ticks,
                    ref mut limit_ticks,
                    ..
                } = self.participants[slot].phase
                {
                    // A broker working paper against the screen re-prices to it: one tick
                    // behind the real market, keeping the reach he had (a market order stays
                    // a market order).
                    if (fair - *price_ticks as f32).abs() > LIVE_STALE_QUOTE_TICKS {
                        let reach = (*limit_ticks - *price_ticks).abs();
                        let px = (fair - side.sign() as f32).round() as i32;
                        *price_ticks = px;
                        *limit_ticks = px + side.sign() * reach;
                    }
                }
            }
            for slot in pulled {
                self.push_event(PitEventKind::Withdraw { slot });
            }
        }
        let k_fast = (dt * 1.6).min(1.0);
        let k_slow = (dt * 0.45).min(1.0);
        let best_bid = self.best_quote(Side::Buy).map(|(_, p)| p);
        let best_ask = self.best_quote(Side::Sell).map(|(_, p)| p);

        let s = &mut self.signals;
        s.arrival_intensity +=
            (self.arrivals_this_window / dt.max(1e-3) - s.arrival_intensity) * k_fast;
        s.fill_rate += (self.fills_this_window / dt.max(1e-3) - s.fill_rate) * k_fast;
        let mean = self.params.mean_qty.max(1.0);
        // Signed aggressor volume over what actually traded in the window, shrunk by two
        // routine orders so a single fill reads as a lean, not a verdict. Sustained one-way
        // paper at any pace pushes this toward the rails; two-way business cancels out.
        let signed_norm =
            (self.signed_fill_volume / (self.abs_fill_volume + mean * 2.0)).clamp(-1.0, 1.0);
        s.imbalance += (signed_norm - s.imbalance) * (dt * 0.6).min(1.0);
        self.arrivals_this_window = 0.0;
        self.fills_this_window = 0.0;
        let keep = (1.0 - dt * 0.4).max(0.0);
        self.signed_fill_volume *= keep;
        self.abs_fill_volume *= keep;

        // Pressure: what the brokers are trying to do, whether or not anyone fills them.
        let mut intent = 0.0f32;
        let mut interest = 0.0f32;
        for i in 0..self.active_count {
            let p = &self.participants[i];
            if let Phase::Working { side, qty_left, .. } = p.phase {
                intent += side.sign() as f32 * qty_left as f32;
                interest += qty_left as f32;
            }
            if let Some((side, _, _, left, _)) = p.working_resume {
                intent += side.sign() as f32 * left as f32;
                interest += left as f32;
            }
            for &(side, q, _) in p.deck.iter() {
                intent += side.sign() as f32 * q as f32 * 0.5;
                interest += q as f32 * 0.5;
            }
        }
        // Signed over total paper on the rail, shrunk by a few routine orders: one broker
        // alone is a lean, a rail full of buyers is one-way paper.
        let pressure_norm = (intent / (interest + mean * 1.5)).clamp(-1.0, 1.0);
        s.pressure += (pressure_norm - s.pressure) * (dt * 1.5).min(1.0);
        self.fill_burst = (self.fill_burst - dt * 1.4).max(0.0);

        // Book state.
        s.best_bid_ticks = best_bid;
        s.best_ask_ticks = best_ask;

        // Participation and depth.
        let active_locals = (0..self.active_count)
            .filter(|&i| self.participants[i].role == ParticipantRole::Local)
            .count()
            .max(1);
        let quoting = (0..self.active_count)
            .filter(|&i| self.participants[i].phase.is_quoting())
            .count();
        let backing = (0..self.active_count)
            .filter(|&i| matches!(self.participants[i].phase, Phase::BackingAway { .. }))
            .count();
        let shouting = (0..self.active_count)
            .filter(|&i| self.participants[i].phase.is_shouting())
            .count();
        let working = (0..self.active_count)
            .filter(|&i| matches!(self.participants[i].phase, Phase::Working { .. }))
            .count();
        s.participation = self.active_count as f32 / SLOTS as f32;
        let depth = quoting as f32 / active_locals as f32;
        s.unfilled_interest += (depth - s.unfilled_interest) * k_fast;
        s.vacuum += (backing as f32 / active_locals as f32 - s.vacuum) * k_fast;

        let stress: f32 = (0..self.active_count)
            .filter(|&i| self.participants[i].role == ParticipantRole::Local)
            .map(|i| self.participants[i].inventory_stress())
            .sum::<f32>()
            / active_locals as f32;
        s.inventory_stress += (stress - s.inventory_stress) * k_slow;

        // Sound: who is shouting, how much is arriving, how hard the brokers are pushing.
        let raw_sound = (shouting as f32 / self.active_count.max(1) as f32) * 0.55
            + (s.arrival_intensity / 2.5).clamp(0.0, 1.0) * 0.25
            + (working as f32 / 3.0).clamp(0.0, 1.0) * 0.2
            + self.fill_burst * 0.15;
        let paused = self.params.news_pause > 0.0;
        let target_sound = if paused {
            0.02
        } else {
            raw_sound.clamp(0.0, 1.0)
        };
        s.sound += (target_sound - s.sound) * (dt * 3.0).min(1.0);

        // Volatility from fair-value motion, fast-market hysteresis.
        let dfair = (self.fair_ticks - self.prev_fair_ticks).abs() / dt.max(1e-3);
        self.prev_fair_ticks = self.fair_ticks;
        s.volatility += (dfair - s.volatility) * (dt * 0.5).min(1.0);
        let was_fast = s.fast_market;
        s.fast_market = if was_fast {
            s.volatility > 1.2
        } else {
            s.volatility > 2.4
        };
        if s.fast_market != was_fast {
            let on = s.fast_market;
            self.push_event(PitEventKind::FastMarket { on });
        }
    }
}

/// Contract multiplier: price units per contract, for notional money on the floor.
/// Futures-style sizes: COMEX gold 100 oz, silver 5,000 oz, CME e-mini S&P 50x, Nasdaq 20x,
/// CBOT e-mini Dow 5x, ICE FTSE 100 10x, CME FX 62,500 GBP / 125,000 EUR, CME bitcoin 5 BTC.
pub fn contract_multiplier(inst: Instrument) -> f32 {
    match inst {
        Instrument::Gold => 100.0,
        Instrument::Silver => 5_000.0,
        Instrument::GbpUsd => 62_500.0,
        Instrument::EurUsd => 125_000.0,
        Instrument::Uk100 => 10.0,
        Instrument::Sp500 => 50.0,
        Instrument::Nasdaq => 20.0,
        Instrument::Us30 => 5.0,
        Instrument::Bitcoin => 5.0,
    }
}

/// Tick size per instrument in price units.
/// Gap (ticks) beyond which a live price change is a jump rather than a move to trade through.
pub const LIVE_JUMP_TICKS: f32 = 12.0;
/// A resting quote this far from the live price is pulled rather than left to be picked off.
pub const LIVE_STALE_QUOTE_TICKS: f32 = 3.0;
/// Under a live anchor locals quote no further than this from the screen.
pub const LIVE_QUOTE_BAND_TICKS: i32 = 2;
/// Fraction of a local's position laid off on the screen per second while live.
pub const LIVE_LAY_OFF_PER_SEC: f32 = 0.35;

pub fn tick_size(inst: Instrument) -> f32 {
    match inst {
        Instrument::Gold => 0.10,
        Instrument::Silver => 0.01,
        Instrument::GbpUsd | Instrument::EurUsd => 0.0001,
        Instrument::Uk100 => 0.5,
        Instrument::Sp500 | Instrument::Nasdaq => 0.25,
        Instrument::Us30 => 1.0,
        Instrument::Bitcoin => 5.0,
    }
}

// ---------------------------------------------------------------------------
// Scenario presets as flow parameters
// ---------------------------------------------------------------------------

/// Parameters for one instrument under a scenario, plus the relaxation rate back to baseline.
pub fn preset_for(scenario: MarketScenario, inst: Instrument) -> (FlowParams, f32) {
    let mut p = FlowParams::baseline(inst);
    match scenario {
        MarketScenario::Normal => {
            // Mild alternating lean so pits are not all identical.
            p.buy_prob = if (inst as usize).is_multiple_of(2) {
                0.54
            } else {
                0.46
            };
            (p, 0.0)
        }
        MarketScenario::FedShock => {
            // Three seconds of silence, then one-sided institutional flow everywhere.
            p.news_pause = 3.0;
            p.block_prob = 0.30;
            match inst {
                Instrument::Uk100 => {
                    p.arrival_rate = 1.6;
                    p.buy_prob = 0.25;
                }
                Instrument::Gold => {
                    p.arrival_rate = 2.6;
                    p.buy_prob = 0.12;
                }
                Instrument::Silver => {
                    p.arrival_rate = 2.2;
                    p.buy_prob = 0.15;
                }
                Instrument::Sp500 => {
                    p.arrival_rate = 2.0;
                    p.buy_prob = 0.20;
                }
                Instrument::Nasdaq => {
                    p.arrival_rate = 2.3;
                    p.buy_prob = 0.16;
                }
                Instrument::EurUsd | Instrument::GbpUsd => {
                    p.arrival_rate = 1.8;
                    p.buy_prob = 0.12;
                }
                Instrument::Us30 => {
                    p.arrival_rate = 2.0;
                    p.buy_prob = 0.22;
                }
                Instrument::Bitcoin => {
                    p.arrival_rate = 1.2;
                    p.buy_prob = 0.30;
                }
            }
            (p, 0.012)
        }
        MarketScenario::SilverSqueeze => {
            match inst {
                Instrument::Silver => {
                    p.arrival_rate = 3.2;
                    p.buy_prob = 0.90;
                    p.block_prob = 0.40;
                }
                Instrument::Gold => {
                    p.arrival_rate = 1.6;
                    p.buy_prob = 0.72;
                    p.block_prob = 0.15;
                }
                _ => {
                    p.arrival_rate = 0.35;
                    p.buy_prob = 0.5;
                }
            }
            (p, 0.008)
        }
        MarketScenario::LunchChop => {
            p.arrival_rate = 0.12;
            p.buy_prob = 0.5;
            p.block_prob = 0.01;
            p.local_quote_rate = 0.25;
            (p, 0.0)
        }
    }
}

// ---------------------------------------------------------------------------
// Bevy resource and systems
// ---------------------------------------------------------------------------

/// All nine pit markets.
#[derive(Resource, Debug, Clone)]
pub struct PitEngines {
    pub pits: Vec<PitMarket>,
    pub last_scenario: MarketScenario,
}

impl Default for PitEngines {
    fn default() -> Self {
        let pits = Instrument::ALL
            .iter()
            .enumerate()
            .map(|(i, inst)| PitMarket::new(*inst, 0x51_7E_A5 + i as u64 * 7919))
            .collect();
        let mut engines = Self {
            pits,
            last_scenario: MarketScenario::Normal,
        };
        engines.apply_scenario(MarketScenario::Normal);
        engines
    }
}

impl PitEngines {
    pub fn pit(&self, inst: Instrument) -> &PitMarket {
        &self.pits[inst as usize]
    }

    pub fn pit_mut(&mut self, inst: Instrument) -> &mut PitMarket {
        &mut self.pits[inst as usize]
    }

    pub fn apply_scenario(&mut self, scenario: MarketScenario) {
        self.last_scenario = scenario;
        for pit in self.pits.iter_mut() {
            let (params, relax) = preset_for(scenario, pit.instrument);
            pit.apply_params(params, relax);
        }
    }

    /// Manual shock key: one-sided block flow into one pit.
    pub fn shock(&mut self, inst: Instrument, side: Side) {
        self.pit_mut(inst).inject_shock(side, 3);
    }
}

/// Pure helper: how many slots are present for a given activity level.
pub fn participation_for_activity(activity: f32) -> usize {
    crate::trader::activity_to_agent_count(activity.clamp(0.0, 1.0), 3, SLOTS)
}

/// System: advances every pit by the frame time. Runs before anything that renders pit state.
pub fn step_pit_engines(
    time: Res<Time>,
    sim: Res<MarketActivitySim>,
    feed: Res<crate::live_feed::LiveFeed>,
    mut controller: ResMut<MarketScenarioController>,
    mut engines: ResMut<PitEngines>,
) {
    let dt = time.delta_secs().min(0.25);
    let now = time.elapsed_secs_f64();

    // Scenario changes come from the controller; the engine owns the consequences.
    if controller.active_scenario != engines.last_scenario {
        let scenario = controller.active_scenario;
        engines.apply_scenario(scenario);
    }

    for (i, inst) in Instrument::ALL.iter().enumerate() {
        let pit = &mut engines.pits[i];
        pit.begin_frame();
        // Live mode: a pit moves only while its real price is ticking. No feed, no action.
        let activity = match feed.mode {
            crate::live_feed::FeedMode::Simulated => sim.get(*inst),
            crate::live_feed::FeedMode::Live => match feed.pit_activity(*inst, now) {
                Some(a) => a,
                None => continue,
            },
        };
        pit.set_live_flow(match feed.mode {
            crate::live_feed::FeedMode::Live => feed.pit_flow(*inst, now),
            crate::live_feed::FeedMode::Simulated => None,
        });
        pit.set_active_count(participation_for_activity(activity));
        pit.step(dt, activity);

        // Bridge for legacy readers: realised imbalance and intensity, not scripted inputs.
        controller.deltas[i] = pit.signals.imbalance;
        controller.volume_multipliers[i] =
            (0.4 + pit.signals.arrival_intensity * 1.4).clamp(0.25, 5.0);
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn run(pit: &mut PitMarket, seconds: f32, activity: f32) {
        let steps = (seconds / 0.05).ceil() as usize;
        for _ in 0..steps {
            pit.begin_frame();
            pit.step(0.05, activity);
        }
    }

    fn count_events(pit: &PitMarket, f: impl Fn(&PitEventKind) -> bool) -> usize {
        pit.events.iter().filter(|e| f(&e.kind)).count()
    }

    #[test]
    fn roster_is_fixed_and_covers_all_roles() {
        assert_eq!(role_for_slot(0), ParticipantRole::LeadBroker);
        assert_eq!(role_for_slot(1), ParticipantRole::Reporter);
        assert_eq!(role_for_slot(10), ParticipantRole::Runner);
        let locals = (0..SLOTS)
            .filter(|&s| role_for_slot(s) == ParticipantRole::Local)
            .count();
        assert!(locals >= 10);
        assert_eq!(tier_for_slot(5), Tier::Middle);
        assert_eq!(tier_for_slot(12), Tier::Outer);
    }

    #[test]
    fn deterministic_given_seed() {
        let mut a = PitMarket::new(Instrument::Silver, 42);
        let mut b = PitMarket::new(Instrument::Silver, 42);
        run(&mut a, 30.0, 0.7);
        run(&mut b, 30.0, 0.7);
        assert_eq!(a.signals.last_ticks, b.signals.last_ticks);
        assert_eq!(a.events.len(), b.events.len());
        assert_eq!(a.participants[3].inventory, b.participants[3].inventory);
    }

    #[test]
    fn normal_flow_is_two_sided() {
        let mut pit = PitMarket::new(Instrument::Gold, 7);
        pit.apply_params(preset_for(MarketScenario::Normal, Instrument::Gold).0, 0.0);
        run(&mut pit, 60.0, 0.6);
        let bids = count_events(&pit, |k| {
            matches!(
                k,
                PitEventKind::Quote {
                    side: Side::Buy,
                    ..
                }
            )
        });
        let asks = count_events(&pit, |k| {
            matches!(
                k,
                PitEventKind::Quote {
                    side: Side::Sell,
                    ..
                }
            )
        });
        let fills = count_events(&pit, |k| matches!(k, PitEventKind::Fill { .. }));
        assert!(bids > 0 && asks > 0, "bids {bids} asks {asks}");
        assert!(fills > 5, "fills {fills}");
        let buy_fills = count_events(&pit, |k| {
            matches!(
                k,
                PitEventKind::Fill {
                    side: Side::Buy,
                    ..
                }
            )
        });
        assert!(
            buy_fills > 0 && buy_fills < fills,
            "both sides should initiate trades"
        );
    }

    #[test]
    fn locals_are_not_all_on_one_side_at_once() {
        // The core failure of the old model: everyone signalled the same side.
        let mut pit = PitMarket::new(Instrument::Sp500, 11);
        pit.apply_params(preset_for(MarketScenario::Normal, Instrument::Sp500).0, 0.0);
        let mut both_seen = false;
        for _ in 0..1200 {
            pit.begin_frame();
            pit.step(0.05, 0.8);
            let buys = pit
                .participants
                .iter()
                .filter(|p| p.phase.signalled_side() == Some(Side::Buy))
                .count();
            let sells = pit
                .participants
                .iter()
                .filter(|p| p.phase.signalled_side() == Some(Side::Sell))
                .count();
            if buys > 0 && sells > 0 {
                both_seen = true;
                break;
            }
        }
        assert!(
            both_seen,
            "at some point bids and offers must be shown simultaneously"
        );
    }

    #[test]
    fn one_sided_flow_moves_price_in_its_direction() {
        let mut pit = PitMarket::new(Instrument::Silver, 3);
        let (p, relax) = preset_for(MarketScenario::SilverSqueeze, Instrument::Silver);
        pit.apply_params(p, relax);
        run(&mut pit, 45.0, 0.95);
        assert!(
            pit.signals.last_ticks > 3,
            "squeeze should lift price, got {}",
            pit.signals.last_ticks
        );
        assert!(pit.signals.imbalance > 0.0);
    }

    #[test]
    fn news_pause_is_silent_then_loud() {
        let mut pit = PitMarket::new(Instrument::Gold, 5);
        run(&mut pit, 10.0, 0.6);
        let (p, relax) = preset_for(MarketScenario::FedShock, Instrument::Gold);
        let px_before = pit.signals.last_ticks;
        pit.apply_params(p, relax);
        let fills_before = count_events(&pit, |k| matches!(k, PitEventKind::Fill { .. }));
        run(&mut pit, 2.8, 0.9);
        let fills_during = count_events(&pit, |k| matches!(k, PitEventKind::Fill { .. }));
        assert_eq!(
            fills_before, fills_during,
            "no trades during the news pause"
        );
        assert!(
            pit.signals.sound < 0.15,
            "pit should be near silent, sound {}",
            pit.signals.sound
        );
        run(&mut pit, 12.0, 0.9);
        assert!(
            pit.signals.sound > 0.35,
            "pit should be loud after the pause, sound {}",
            pit.signals.sound
        );
        assert!(
            pit.signals.last_ticks < px_before - 2,
            "gold gets sold on the Fed: before {} after {}",
            px_before,
            pit.signals.last_ticks
        );
    }

    #[test]
    fn sound_rises_before_price_moves() {
        // Coval & Shumway: the pit gets loud before the price moves.
        let mut pit = PitMarket::new(Instrument::Nasdaq, 9);
        pit.apply_params(
            preset_for(MarketScenario::LunchChop, Instrument::Nasdaq).0,
            0.0,
        );
        run(&mut pit, 20.0, 0.2);
        let quiet_sound = pit.signals.sound;
        pit.inject_shock(Side::Sell, 4);
        let mut t_sound: Option<f32> = None;
        let mut t_price: Option<f32> = None;
        let start_px = pit.signals.last_ticks;
        for i in 0..400 {
            pit.begin_frame();
            pit.step(0.05, 0.9);
            let t = i as f32 * 0.05;
            if t_sound.is_none() && pit.signals.sound > quiet_sound + 0.12 {
                t_sound = Some(t);
            }
            if t_price.is_none() && (pit.signals.last_ticks - start_px).abs() >= 2 {
                t_price = Some(t);
            }
            if t_sound.is_some() && t_price.is_some() {
                break;
            }
        }
        let (ts, tp) = (
            t_sound.expect("sound should rise"),
            t_price.expect("price should move"),
        );
        assert!(ts <= tp, "sound at {ts}s must lead price at {tp}s");
    }

    #[test]
    fn reporter_prints_only_changes() {
        let mut pit = PitMarket::new(Instrument::Us30, 13);
        run(&mut pit, 60.0, 0.7);
        let mut last: Option<i32> = None;
        for e in pit.events.iter() {
            if let PitEventKind::Print { price_ticks, .. } = e.kind {
                assert_ne!(Some(price_ticks), last, "consecutive prints must differ");
                last = Some(price_ticks);
            }
        }
    }

    #[test]
    fn inventories_stay_bounded() {
        let mut pit = PitMarket::new(Instrument::Silver, 21);
        let (p, relax) = preset_for(MarketScenario::SilverSqueeze, Instrument::Silver);
        pit.apply_params(p, relax);
        run(&mut pit, 120.0, 0.95);
        for p in pit
            .participants
            .iter()
            .filter(|p| p.role == ParticipantRole::Local)
        {
            assert!(
                p.inventory.abs() <= p.max_inventory * 3,
                "slot {} inventory {}",
                p.slot,
                p.inventory
            );
        }
    }

    #[test]
    fn fills_only_between_active_slots_with_distinct_counterparties() {
        let mut pit = PitMarket::new(Instrument::Gold, 17);
        pit.set_active_count(7);
        run(&mut pit, 40.0, 0.5);
        for e in pit.events.iter() {
            if let PitEventKind::Fill {
                aggressor, passive, ..
            } = e.kind
            {
                assert!(aggressor < 7 && passive < 7);
                assert_ne!(aggressor, passive);
            }
        }
    }

    #[test]
    fn shrinking_the_pit_clears_dangling_counterparties() {
        let mut pit = PitMarket::new(Instrument::Gold, 19);
        run(&mut pit, 20.0, 0.9);
        pit.set_active_count(4);
        for s in 0..4 {
            if let Some(cp) = pit.participants[s].phase.counterparty() {
                assert!(cp < 4);
            }
        }
        for s in 4..SLOTS {
            assert!(pit.participants[s].phase.is_idle());
        }
    }

    #[test]
    fn lunch_chop_is_quiet() {
        let mut pit = PitMarket::new(Instrument::EurUsd, 23);
        pit.apply_params(
            preset_for(MarketScenario::LunchChop, Instrument::EurUsd).0,
            0.0,
        );
        run(&mut pit, 60.0, 0.1);
        assert!(pit.signals.sound < 0.45, "sound {}", pit.signals.sound);
        assert!(pit.signals.fill_rate < 1.0);
    }

    #[test]
    fn tick_sizes_and_prices_format() {
        let pit = PitMarket::new(Instrument::EurUsd, 1);
        let p = pit.price_of(3);
        assert!((p - (1.0860 + 0.0003)).abs() < 1e-6);
        assert_eq!(pit.price_digit(-1), 9);
        assert_eq!(pit.price_digit(27), 7);
    }

    #[test]
    fn engines_default_covers_all_instruments() {
        let engines = PitEngines::default();
        assert_eq!(engines.pits.len(), Instrument::ALL.len());
        assert_eq!(
            engines.pit(Instrument::Bitcoin).instrument,
            Instrument::Bitcoin
        );
    }
}

#[cfg(test)]
mod debug_probe {
    use super::*;
    #[test]
    #[ignore]
    fn probe_fed_gold() {
        let mut pit = PitMarket::new(Instrument::Gold, 5);
        for _ in 0..200 {
            pit.begin_frame();
            pit.step(0.05, 0.6);
        }
        let (p, relax) = preset_for(MarketScenario::FedShock, Instrument::Gold);
        println!(
            "before: last {} fair {:.2}",
            pit.signals.last_ticks, pit.fair_ticks
        );
        pit.apply_params(p, relax);
        let n0 = pit.events.len();
        let mut last_phase = format!("{:?}", pit.participants[0].phase);
        for i in 0..300 {
            pit.begin_frame();
            pit.step(0.05, 0.9);
            let ph = format!("{:?}", pit.participants[0].phase);
            let ph = ph
                .split(", age")
                .next()
                .unwrap_or(&ph)
                .split(", remaining")
                .next()
                .unwrap_or(&ph)
                .to_string();
            if ph != last_phase {
                println!(
                    "  [{:6.2}] slot0 -> {} deck={:?} resume={:?}",
                    10.0 + i as f32 * 0.05,
                    ph,
                    pit.participants[0].deck,
                    pit.participants[0].working_resume
                );
                last_phase = ph;
            }
        }
        for e in pit
            .events
            .iter()
            .skip(n0.saturating_sub(pit.events.len().saturating_sub(EVENT_RING_CAP)))
        {
            match e.kind {
                PitEventKind::Fill {
                    aggressor,
                    passive,
                    side,
                    price_ticks,
                    qty,
                } => println!(
                    "{:6.2} FILL agg{} pas{} {:?} px{} q{}",
                    e.t, aggressor, passive, side, price_ticks, qty
                ),
                PitEventKind::Quote {
                    slot,
                    side,
                    price_ticks,
                    qty,
                } => println!(
                    "{:6.2} quote s{} {:?} px{} q{}",
                    e.t, slot, side, price_ticks, qty
                ),
                PitEventKind::OrderArrival { broker, side, qty } => {
                    println!("{:6.2} arrive b{} {:?} q{}", e.t, broker, side, qty)
                }
                other => println!("{:6.2} {:?}", e.t, other),
            }
        }
        println!(
            "after: last {} fair {:.2} imb {:.2} pressure {:.2} sound {:.2} depth {:.2} vacuum {:.2} arr {:.2}",
            pit.signals.last_ticks,
            pit.fair_ticks,
            pit.signals.imbalance,
            pit.signals.pressure,
            pit.signals.sound,
            pit.signals.unfilled_interest,
            pit.signals.vacuum,
            pit.signals.arrival_intensity
        );
        let since = pit.clock - 15.0;
        let c = |f: &dyn Fn(&PitEventKind) -> bool| {
            pit.events
                .iter()
                .filter(|e| e.t >= since && f(&e.kind))
                .count()
        };
        println!(
            "counts(last 15s): arrivals {} bids {} asks {} withdraws {} fills {} missed {} backaway {} outtrade {}",
            c(&|k| matches!(k, PitEventKind::OrderArrival { .. })),
            c(&|k| matches!(
                k,
                PitEventKind::Quote {
                    side: Side::Buy,
                    ..
                }
            )),
            c(&|k| matches!(
                k,
                PitEventKind::Quote {
                    side: Side::Sell,
                    ..
                }
            )),
            c(&|k| matches!(k, PitEventKind::Withdraw { .. })),
            c(&|k| matches!(k, PitEventKind::Fill { .. })),
            c(&|k| matches!(k, PitEventKind::Missed { .. })),
            c(&|k| matches!(k, PitEventKind::BackAway { .. })),
            c(&|k| matches!(k, PitEventKind::OutTrade { .. }))
        );
        for b in [0usize, 4, 8, 13] {
            println!(
                "broker {} phase {:?} deck {:?}",
                b,
                format!("{:?}", pit.participants[b].phase)
                    .split(", age")
                    .next()
                    .unwrap(),
                pit.participants[b].deck
            );
        }
        let mut hist = std::collections::BTreeMap::new();
        for p in pit.participants.iter().take(pit.active_count) {
            *hist
                .entry(
                    format!("{:?}", p.phase)
                        .split(' ')
                        .next()
                        .unwrap()
                        .to_string(),
                )
                .or_insert(0) += 1;
        }
        println!("phase histogram now: {:?}", hist);
        for p in pit
            .participants
            .iter()
            .filter(|p| p.role == ParticipantRole::Local)
        {
            print!("s{}:{} ", p.slot, p.inventory);
        }
        println!();
    }
    #[test]
    fn live_anchor_keeps_the_tape_on_the_real_price() {
        let mut pit = PitMarket::new(Instrument::Gold, 7);
        pit.apply_params(FlowParams::baseline(Instrument::Gold), 0.0);
        pit.set_active_count(14);
        pit.set_live_anchor(41_200);
        let fills_after_jump = {
            pit.begin_frame();
            pit.step(0.5, 0.8);
            pit.recent_events()
                .iter()
                .filter(|e| matches!(e.kind, PitEventKind::Fill { .. }))
                .count()
        };
        assert!(
            fills_after_jump <= 4,
            "a jump must not sweep a stale book: {fills_after_jump} fills"
        );
        // Walk the anchor up a tick a second; every deal must print within a few ticks of
        // the live price at the moment it trades (prints are sparse, so the last print may lag).
        let mut worst = 0;
        let mut fills = 0;
        for i in 1..=240 {
            let anchor = 41_200 + i / 4;
            pit.set_live_anchor(anchor);
            pit.begin_frame();
            pit.step(0.25, 0.8);
            for e in pit.recent_events() {
                if let PitEventKind::Fill {
                    price_ticks,
                    aggressor,
                    passive,
                    side,
                    ..
                } = e.kind
                {
                    fills += 1;
                    if (price_ticks - anchor).abs() > 4 {
                        println!(
                            "off-market fill: px {price_ticks} anchor {anchor} fair {:.1} side {side:?} agg {:?} pas {:?} bid {:?} ask {:?}",
                            pit.fair_ticks,
                            pit.participants[aggressor].role,
                            pit.participants[passive].role,
                            pit.signals.best_bid_ticks,
                            pit.signals.best_ask_ticks
                        );
                    }
                    worst = worst.max((price_ticks - anchor).abs());
                }
            }
        }
        assert!(
            fills >= 5,
            "pit should keep trading while anchored: {fills} fills"
        );
        assert!(
            worst <= 4,
            "a deal printed {worst} ticks off the live price"
        );
        assert!(
            (pit.fair_ticks - 41_260.0).abs() < 2.0,
            "fair {}",
            pit.fair_ticks
        );
    }

    #[test]
    fn live_flow_decides_which_side_the_paper_is_on() {
        // A tape reading 90% upticks, anchor drifting up: the paper that arrives must be
        // buying, and the realised imbalance (what the nameplate shows) must agree.
        let mut pit = PitMarket::new(Instrument::Nasdaq, 11);
        pit.apply_params(FlowParams::baseline(Instrument::Nasdaq), 0.0);
        pit.set_active_count(14);
        pit.set_live_anchor(36_000);
        pit.set_live_flow(Some(LiveFlow {
            arrivals_per_sec: 1.8,
            buy_prob: 0.88,
            block_prob: 0.08,
        }));
        let (mut buys, mut sells) = (0, 0);
        let (mut arrivals, mut missed, mut quotes, mut withdraws, mut broker_withdraws) =
            (0, 0, 0, 0, 0);
        let mut imbalance_samples = Vec::new();
        for i in 1..=160 {
            pit.set_live_anchor(36_000 + i / 8);
            pit.begin_frame();
            pit.step(0.25, 0.8);
            if i > 40 {
                imbalance_samples.push(pit.signals.imbalance);
            }
            for e in pit.recent_events() {
                match e.kind {
                    PitEventKind::Fill { side, .. } => {
                        if side.is_buy() {
                            buys += 1
                        } else {
                            sells += 1
                        }
                    }
                    PitEventKind::OrderArrival { .. } => arrivals += 1,
                    PitEventKind::Missed { .. } => missed += 1,
                    PitEventKind::Quote { .. } => quotes += 1,
                    PitEventKind::Withdraw { slot } => {
                        withdraws += 1;
                        if pit.participants[slot].can_broker() {
                            broker_withdraws += 1
                        }
                    }
                    _ => {}
                }
            }
            if i % 40 == 0 {
                let decks: usize = pit
                    .participants
                    .iter()
                    .filter(|p| p.can_broker())
                    .map(|p| p.deck.len())
                    .sum();
                let working = pit
                    .participants
                    .iter()
                    .filter(|p| matches!(p.phase, Phase::Working { .. }))
                    .count();
                println!(
                    "   decks queued {decks} working {working} broker withdraws {broker_withdraws}"
                );
                println!(
                    "t={}s fills {}/{} arrivals {arrivals} missed {missed} quotes {quotes} withdraws {withdraws} | vacuum {:.2} depth {:.2} pressure {:.2} stress {:.2} bid {:?} ask {:?} fair {:.1}",
                    i / 4,
                    buys,
                    sells,
                    pit.signals.vacuum,
                    pit.signals.unfilled_interest,
                    pit.signals.pressure,
                    pit.signals.inventory_stress,
                    pit.signals.best_bid_ticks,
                    pit.signals.best_ask_ticks,
                    pit.fair_ticks
                );
            }
        }
        assert!(
            buys + sells >= 20,
            "pit traded too little: {buys} buys {sells} sells"
        );
        let share = buys as f32 / (buys + sells) as f32;
        assert!(
            share > 0.7,
            "buy share of fills {share:.2} ({buys}/{sells})"
        );
        // The nameplate's imbalance is a short-memory signal, so judge it over the run, not
        // at one instant: it should lean the way the tape leans most of the time.
        let mean = imbalance_samples.iter().sum::<f32>() / imbalance_samples.len() as f32;
        let positive = imbalance_samples.iter().filter(|v| **v > 0.0).count() as f32
            / imbalance_samples.len() as f32;
        assert!(mean > 0.08, "mean imbalance {mean:.3}");
        assert!(
            positive > 0.6,
            "imbalance positive {positive:.2} of the time"
        );

        // Taking the flow away hands control back to the generator (balanced baseline),
        // once the brokers have worked off the buy paper still on their decks.
        pit.set_live_flow(None);
        let (mut buys, mut sells) = (0, 0);
        for i in 1..=480 {
            pit.set_live_anchor(36_020 + (i % 16) / 8);
            pit.begin_frame();
            pit.step(0.25, 0.8);
            if i <= 160 {
                continue;
            }
            for e in pit.recent_events() {
                if let PitEventKind::Fill { side, .. } = e.kind {
                    if side.is_buy() { buys += 1 } else { sells += 1 }
                }
            }
        }
        let share = buys as f32 / (buys + sells).max(1) as f32;
        assert!(
            (0.3..=0.7).contains(&share),
            "generator share {share:.2} ({buys}/{sells})"
        );
    }

    #[test]
    fn ticks_round_trip_through_real_prices() {
        for (inst, price) in [
            (Instrument::Gold, 4121.83f32),
            (Instrument::Silver, 57.812),
            (Instrument::EurUsd, 1.12359),
            (Instrument::Nasdaq, 29576.0),
            (Instrument::Bitcoin, 82586.58),
        ] {
            let pit = PitMarket::new(inst, 1);
            let back = pit.price_of(pit.ticks_for_price(price));
            assert!(
                (back - price).abs() <= tick_size(inst) * 0.51,
                "{inst:?}: {price} -> {back}"
            );
        }
    }

    #[test]
    fn live_anchor_gap_is_a_jump_not_a_sweep() {
        let mut pit = PitMarket::new(Instrument::Gold, 3);
        pit.apply_params(FlowParams::baseline(Instrument::Gold), 0.0);
        pit.set_active_count(14);
        pit.set_live_anchor(41_200);
        for _ in 0..40 {
            pit.begin_frame();
            pit.step(0.25, 0.8);
        }
        pit.set_live_anchor(41_500);
        assert_eq!(pit.signals.last_ticks, 41_500);
        pit.begin_frame();
        pit.step(0.5, 0.8);
        let fills = pit
            .recent_events()
            .iter()
            .filter(|e| matches!(e.kind, PitEventKind::Fill { .. }))
            .count();
        assert!(fills <= 4, "{fills} fills right after a 300-tick jump");
        pit.clear_live_anchor();
        assert!(pit.live_anchor.is_none());
    }
}
