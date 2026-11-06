use bevy::prelude::*;

/// Financial instrument represented on the OpenCry exchange floor
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Component)]
pub enum Instrument {
    Gold,
    Silver,
    GbpUsd,
    EurUsd,
    Uk100,
    Sp500,
    Nasdaq,
    Us30,
    Bitcoin,
}

impl Instrument {
    pub const ALL: [Instrument; 9] = [
        Instrument::Gold,
        Instrument::Silver,
        Instrument::GbpUsd,
        Instrument::EurUsd,
        Instrument::Uk100,
        Instrument::Sp500,
        Instrument::Nasdaq,
        Instrument::Us30,
        Instrument::Bitcoin,
    ];

    pub fn name(&self) -> &'static str {
        match self {
            Self::Gold => "GOLD",
            Self::Silver => "SILVER",
            Self::GbpUsd => "GBP/USD",
            Self::EurUsd => "EUR/USD",
            Self::Uk100 => "FTSE 100",
            Self::Sp500 => "S&P 500",
            Self::Nasdaq => "NASDAQ 100",
            Self::Us30 => "DOW JONES",
            Self::Bitcoin => "BITCOIN",
        }
    }

    pub fn symbol(&self) -> &'static str {
        match self {
            Self::Gold => "XAU/USD",
            Self::Silver => "XAG/USD",
            Self::GbpUsd => "GBP/USD",
            Self::EurUsd => "EUR/USD",
            Self::Uk100 => "UK100",
            Self::Sp500 => "US500",
            Self::Nasdaq => "NAS100",
            Self::Us30 => "US30",
            Self::Bitcoin => "BTC/USD",
        }
    }

    pub fn district(&self) -> District {
        match self {
            Self::Gold | Self::Silver => District::Metals,
            Self::GbpUsd | Self::EurUsd => District::Forex,
            Self::Sp500 | Self::Nasdaq | Self::Us30 | Self::Uk100 => District::Equities,
            Self::Bitcoin => District::Crypto,
        }
    }

    /// Base accent color for the pit border and label
    pub fn accent_color(&self) -> Color {
        match self {
            Self::Gold => Color::srgb(0.95, 0.77, 0.20), // Gold amber
            Self::Silver => Color::srgb(0.85, 0.90, 0.95), // Crisp silver white
            Self::GbpUsd => Color::srgb(0.25, 0.60, 0.90), // British blue
            Self::EurUsd => Color::srgb(0.30, 0.75, 0.75), // Euro cyan
            Self::Uk100 => Color::srgb(0.80, 0.25, 0.35), // FTSE claret
            Self::Sp500 => Color::srgb(0.90, 0.45, 0.25), // Index orange
            Self::Nasdaq => Color::srgb(0.85, 0.30, 0.50), // Tech magenta
            Self::Us30 => Color::srgb(0.35, 0.55, 0.90), // Dow blue
            Self::Bitcoin => Color::srgb(0.98, 0.58, 0.15), // Bitcoin orange
        }
    }

    /// Initial activity setpoint from syllabus
    pub fn default_activity(&self) -> f32 {
        match self {
            Self::Silver => 0.95,  // Frantic crowd
            Self::Nasdaq => 0.80,  // Fast tech
            Self::Us30 => 0.70,    // Busy blue-chip index
            Self::Sp500 => 0.62,   // Steady benchmark
            Self::Uk100 => 0.55,   // London session index
            Self::Bitcoin => 0.50, // Moderate crypto
            Self::Gold => 0.45,    // Calm metal
            Self::EurUsd => 0.25,  // Slow forex
            Self::GbpUsd => 0.12,  // Quiet market
        }
    }

    /// Realistic benchmark baseline price for price prints
    pub fn base_price(&self) -> f32 {
        match self {
            Self::Gold => 2738.4,
            Self::Silver => 31.85,
            Self::GbpUsd => 1.3050,
            Self::EurUsd => 1.0860,
            Self::Uk100 => 10740.0,
            Self::Sp500 => 5860.2,
            Self::Nasdaq => 20450.0,
            Self::Us30 => 52600.0,
            Self::Bitcoin => 68400.0,
        }
    }
}

/// Asset class district grouping on the exchange floor
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum District {
    Metals,
    Forex,
    Equities,
    Crypto,
}

impl District {
    pub const ALL: [District; 4] = [
        District::Metals,
        District::Equities,
        District::Forex,
        District::Crypto,
    ];

    pub fn label(&self) -> &'static str {
        match self {
            Self::Metals => "METALS DISTRICT",
            Self::Forex => "FX DISTRICT",
            Self::Equities => "INDICES DISTRICT",
            Self::Crypto => "CRYPTO DISTRICT",
        }
    }

    /// Short name for the floor sign.
    pub fn sign(&self) -> &'static str {
        match self {
            Self::Metals => "METALS",
            Self::Forex => "FOREX",
            Self::Equities => "INDICES",
            Self::Crypto => "CRYPTO",
        }
    }

    pub fn colour(&self) -> Color {
        match self {
            Self::Metals => Color::srgb(0.95, 0.80, 0.35),
            Self::Forex => Color::srgb(0.40, 0.70, 1.0),
            Self::Equities => Color::srgb(0.55, 0.95, 0.65),
            Self::Crypto => Color::srgb(1.0, 0.60, 0.25),
        }
    }

    /// The pits in this district, in floor order.
    pub fn pits(&self) -> impl Iterator<Item = Instrument> + '_ {
        Instrument::ALL
            .into_iter()
            .filter(move |i| i.district() == *self)
    }
}

/// Simulated brokerage firm identities (Section 21)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Component)]
pub enum Firm {
    Atlas,
    Redwood,
    Northstar,
    Apex,
    Horizon,
}

impl Firm {
    pub const ALL: [Firm; 5] = [
        Firm::Atlas,
        Firm::Redwood,
        Firm::Northstar,
        Firm::Apex,
        Firm::Horizon,
    ];

    pub fn name(&self) -> &'static str {
        match self {
            Self::Atlas => "Atlas Capital",
            Self::Redwood => "Redwood Trading",
            Self::Northstar => "Northstar Securities",
            Self::Apex => "Apex Quant",
            Self::Horizon => "Horizon Market Makers",
        }
    }

    pub fn color(&self) -> Color {
        match self {
            Self::Atlas => Color::srgb(0.20, 0.60, 0.95), // Blue coats
            Self::Redwood => Color::srgb(0.90, 0.25, 0.25), // Red coats
            Self::Northstar => Color::srgb(0.95, 0.85, 0.25), // Yellow coats
            Self::Apex => Color::srgb(0.25, 0.85, 0.50),  // Green coats
            Self::Horizon => Color::srgb(0.75, 0.35, 0.90), // Purple coats
        }
    }
}

/// Normalizes activity into strictly bounded [0.0, 1.0]
pub fn clamp_activity(activity: f32) -> f32 {
    activity.clamp(0.0, 1.0)
}

/// Dynamic market activity simulator tracking smooth transitions and shock events
#[derive(Resource, Debug, Clone)]
pub struct MarketActivitySim {
    pub activities: [f32; 9],
    pub target_activities: [f32; 9],
    pub elapsed_secs: f32,
    pub scenario_active: bool,
}

impl Default for MarketActivitySim {
    fn default() -> Self {
        let mut activities = [0.0; 9];
        let mut target_activities = [0.0; 9];
        for (i, inst) in Instrument::ALL.iter().enumerate() {
            let act = inst.default_activity();
            activities[i] = act;
            target_activities[i] = act;
        }
        Self {
            activities,
            target_activities,
            elapsed_secs: 0.0,
            scenario_active: true,
        }
    }
}

impl MarketActivitySim {
    pub fn get(&self, instrument: Instrument) -> f32 {
        let idx = instrument as usize;
        self.activities[idx]
    }

    pub fn set_target(&mut self, instrument: Instrument, target: f32) {
        let idx = instrument as usize;
        self.target_activities[idx] = clamp_activity(target);
    }

    pub fn trigger_shock(&mut self, instrument: Instrument) {
        let idx = instrument as usize;
        // Immediate jump towards peak excitement
        self.target_activities[idx] = 0.98;
        self.activities[idx] = (self.activities[idx] + 0.35).min(0.98);
    }

    /// Advance simulation clock and smoothly interpolate activities toward targets
    pub fn update(&mut self, dt: f32) {
        self.elapsed_secs += dt;

        // If scenario is active, progress the scripted narrative from Section 24
        if self.scenario_active {
            let t = self.elapsed_secs % 60.0; // Repeat 60-second narrative cycle
            if t < 10.0 {
                // 0-10s: Baseline normal
                self.target_activities[Instrument::Gold as usize] = 0.35;
                self.target_activities[Instrument::Silver as usize] = 0.40;
                self.target_activities[Instrument::GbpUsd as usize] = 0.20;
            } else if t < 20.0 {
                // 10-20s: Gold begins rising (0.35 -> 0.75)
                let progress = (t - 10.0) / 10.0;
                self.target_activities[Instrument::Gold as usize] = 0.35 + 0.40 * progress;
            } else if t < 35.0 {
                // 20-35s: Silver reacts violently! (0.40 -> 0.98)
                let progress = (t - 20.0) / 15.0;
                self.target_activities[Instrument::Silver as usize] = 0.40 + 0.58 * progress;
                self.target_activities[Instrument::Us30 as usize] = 0.55 + 0.25 * progress;
            } else if t < 45.0 {
                // 35-45s: Nasdaq quietens down
                self.target_activities[Instrument::Nasdaq as usize] = 0.45;
                self.target_activities[Instrument::Silver as usize] = 0.95;
            } else {
                // 45-60s: Silver stays dominant, others cool
                self.target_activities[Instrument::Silver as usize] = 0.92;
                self.target_activities[Instrument::Gold as usize] = 0.50;
            }
        }

        // Smooth exponential interpolation (lerp) towards target
        let lerp_rate = 2.0 * dt;
        for i in 0..9 {
            let diff = self.target_activities[i] - self.activities[i];
            self.activities[i] += diff * lerp_rate.min(1.0);
            self.activities[i] = clamp_activity(self.activities[i]);
        }
    }
}

/// System updating the simulation clock each frame
pub fn update_market_simulation(time: Res<Time>, mut sim: ResMut<MarketActivitySim>) {
    sim.update(time.delta_secs());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_instrument_metadata() {
        assert_eq!(Instrument::Silver.name(), "SILVER");
        assert_eq!(Instrument::Silver.symbol(), "XAG/USD");
        assert_eq!(Instrument::Silver.district(), District::Metals);
        assert_eq!(Instrument::ALL.len(), 9);
    }

    #[test]
    fn test_clamp_activity() {
        assert_eq!(clamp_activity(-0.5), 0.0);
        assert_eq!(clamp_activity(0.42), 0.42);
        assert_eq!(clamp_activity(1.5), 1.0);
    }

    #[test]
    fn test_sim_shock_trigger() {
        let mut sim = MarketActivitySim::default();
        sim.trigger_shock(Instrument::Silver);
        assert!(sim.get(Instrument::Silver) >= 0.95);
        assert_eq!(sim.target_activities[Instrument::Silver as usize], 0.98);
    }

    #[test]
    fn test_sim_smooth_interpolation() {
        let mut sim = MarketActivitySim {
            scenario_active: false, // Disable scripted updates for pure math test
            ..Default::default()
        };
        sim.activities[Instrument::Gold as usize] = 0.20;
        sim.set_target(Instrument::Gold, 0.80);

        // Step 0.5s
        sim.update(0.5);
        let updated = sim.get(Instrument::Gold);
        assert!(updated > 0.20 && updated <= 0.80);
    }

    #[test]
    fn test_firm_colors() {
        assert_eq!(Firm::ALL.len(), 5);
        assert_eq!(Firm::Atlas.name(), "Atlas Capital");
    }
}
