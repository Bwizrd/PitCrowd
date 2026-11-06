use crate::market::{Instrument, MarketActivitySim};
use bevy::prelude::*;

/// Available market scenarios for weekend simulation and trading demonstrations
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MarketScenario {
    #[default]
    Normal,
    FedShock,
    SilverSqueeze,
    LunchChop,
}

impl MarketScenario {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Normal => "Normal Flow",
            Self::FedShock => "Fed Rate Decision (Macro Rotation)",
            Self::SilverSqueeze => "Silver Short Squeeze (Liquidity Sweep)",
            Self::LunchChop => "Lunchtime Chop (Dead Floor)",
        }
    }

    pub fn description(&self) -> &'static str {
        match self {
            Self::Normal => "Steady multi-asset order flow with balanced bid/ask depth.",
            Self::FedShock => {
                "US Dollar surges; massive institutional selling in Gold; Equities shake."
            }
            Self::SilverSqueeze => {
                "Violent buyer sweep; Silver crowd explodes into a directional phalanx."
            }
            Self::LunchChop => {
                "Low volume, wide spreads, clerks milling lazily with zero institutional flow."
            }
        }
    }
}

/// Dynamic market delta (net buy/sell aggression) for an instrument:
///   +1.0 = Max aggressive buying (lifting offers)
///   -1.0 = Max aggressive selling (hitting bids)
///    0.0 = Neutral / balanced auction
#[derive(Resource, Debug, Clone)]
pub struct MarketScenarioController {
    pub active_scenario: MarketScenario,
    pub scenario_elapsed: f32,
    /// Net directional delta [-1.0, 1.0] for each of the 9 instruments
    pub deltas: [f32; 9],
    /// Volume intensity multiplier [0.1, 5.0] for each instrument
    pub volume_multipliers: [f32; 9],
    /// Last triggered announcement headline
    pub announcement_banner: String,
    pub banner_timer: f32,
}

impl Default for MarketScenarioController {
    fn default() -> Self {
        Self {
            active_scenario: MarketScenario::Normal,
            scenario_elapsed: 0.0,
            deltas: [0.0; 9],
            volume_multipliers: [1.0; 9],
            announcement_banner: "Floor Open - Normal Multi-Asset Auction".to_string(),
            banner_timer: 5.0,
        }
    }
}

impl MarketScenarioController {
    pub fn switch_scenario(&mut self, scenario: MarketScenario, sim: &mut MarketActivitySim) {
        self.active_scenario = scenario;
        self.scenario_elapsed = 0.0;
        self.announcement_banner = format!("[SCENARIO] {}", scenario.name());
        self.banner_timer = 6.0;

        // Every pit starts from its baseline so nothing leaks from the previous scenario.
        for (i, inst) in Instrument::ALL.iter().enumerate() {
            self.deltas[i] = 0.0;
            self.volume_multipliers[i] = 1.0;
            sim.set_target(*inst, inst.default_activity());
        }

        match scenario {
            MarketScenario::Normal => {
                sim.scenario_active = true;
                for (i, inst) in Instrument::ALL.iter().enumerate() {
                    self.deltas[i] = 0.10 * (i as f32 % 2.0 * 2.0 - 1.0);
                    self.volume_multipliers[i] = 1.0;
                    sim.set_target(*inst, inst.default_activity());
                }
            }
            MarketScenario::FedShock => {
                sim.scenario_active = false;
                // Dow and FTSE sold hard with the other indices
                self.deltas[Instrument::Us30 as usize] = -0.82;
                self.volume_multipliers[Instrument::Us30 as usize] = 3.5;
                sim.set_target(Instrument::Us30, 0.92);
                self.deltas[Instrument::Uk100 as usize] = -0.70;
                self.volume_multipliers[Instrument::Uk100 as usize] = 2.5;
                sim.set_target(Instrument::Uk100, 0.80);

                // Gold and Silver get aggressively sold off (-0.90 delta)
                self.deltas[Instrument::Gold as usize] = -0.88;
                self.volume_multipliers[Instrument::Gold as usize] = 4.0;
                sim.set_target(Instrument::Gold, 0.90);

                self.deltas[Instrument::Silver as usize] = -0.85;
                self.volume_multipliers[Instrument::Silver as usize] = 3.2;
                sim.set_target(Instrument::Silver, 0.88);

                // Equities drop sharply
                self.deltas[Instrument::Sp500 as usize] = -0.75;
                self.deltas[Instrument::Nasdaq as usize] = -0.82;
                sim.set_target(Instrument::Sp500, 0.80);
                sim.set_target(Instrument::Nasdaq, 0.85);

                // FX EUR/USD and GBP/USD collapse vs strong Dollar
                self.deltas[Instrument::EurUsd as usize] = -0.90;
                self.deltas[Instrument::GbpUsd as usize] = -0.85;
                sim.set_target(Instrument::EurUsd, 0.75);
                sim.set_target(Instrument::GbpUsd, 0.70);
            }
            MarketScenario::SilverSqueeze => {
                sim.scenario_active = false;
                // Silver volume and buying aggression explode!
                self.deltas[Instrument::Silver as usize] = 0.98;
                self.volume_multipliers[Instrument::Silver as usize] = 5.0;
                sim.set_target(Instrument::Silver, 0.99);
                sim.trigger_shock(Instrument::Silver);

                // Gold follows sympathetically
                self.deltas[Instrument::Gold as usize] = 0.75;
                self.volume_multipliers[Instrument::Gold as usize] = 2.5;
                sim.set_target(Instrument::Gold, 0.82);

                // Other markets calm down in comparison
                self.deltas[Instrument::EurUsd as usize] = 0.05;
                self.deltas[Instrument::GbpUsd as usize] = -0.05;
                sim.set_target(Instrument::EurUsd, 0.20);
                sim.set_target(Instrument::GbpUsd, 0.15);
            }
            MarketScenario::LunchChop => {
                sim.scenario_active = false;
                // Complete quiet across the entire floor
                for (i, inst) in Instrument::ALL.iter().enumerate() {
                    self.deltas[i] = 0.0;
                    self.volume_multipliers[i] = 0.25;
                    sim.set_target(*inst, 0.08);
                    sim.activities[i] = 0.08;
                }
            }
        }
    }

    pub fn get_delta(&self, inst: Instrument) -> f32 {
        self.deltas[inst as usize]
    }

    pub fn get_volume_multiplier(&self, inst: Instrument) -> f32 {
        self.volume_multipliers[inst as usize]
    }

    pub fn update(&mut self, dt: f32) {
        self.scenario_elapsed += dt;
        if self.banner_timer > 0.0 {
            self.banner_timer -= dt;
        }
    }
}

/// System: Handles F2/F3/F4/F5 hotkeys to trigger realistic trading scenarios
pub fn handle_scenario_hotkeys(
    keyboard: Res<ButtonInput<KeyCode>>,
    feed: Res<crate::live_feed::LiveFeed>,
    mut sim: ResMut<MarketActivitySim>,
    mut controller: ResMut<MarketScenarioController>,
) {
    let any = [KeyCode::F2, KeyCode::F3, KeyCode::F4, KeyCode::F5]
        .iter()
        .any(|k| keyboard.just_pressed(*k));
    if any && !crate::live_feed::sim_only_guard(&feed, &mut controller) {
        return;
    }
    if keyboard.just_pressed(KeyCode::F2) {
        controller.switch_scenario(MarketScenario::FedShock, &mut sim);
    } else if keyboard.just_pressed(KeyCode::F3) {
        controller.switch_scenario(MarketScenario::SilverSqueeze, &mut sim);
    } else if keyboard.just_pressed(KeyCode::F4) {
        controller.switch_scenario(MarketScenario::LunchChop, &mut sim);
    } else if keyboard.just_pressed(KeyCode::F5) {
        controller.switch_scenario(MarketScenario::Normal, &mut sim);
    }
}

/// System: Ticks the scenario controller clock
pub fn update_scenario_controller(
    time: Res<Time>,
    mut controller: ResMut<MarketScenarioController>,
) {
    controller.update(time.delta_secs());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scenario_names() {
        assert_eq!(
            MarketScenario::FedShock.name(),
            "Fed Rate Decision (Macro Rotation)"
        );
        assert_eq!(
            MarketScenario::SilverSqueeze.name(),
            "Silver Short Squeeze (Liquidity Sweep)"
        );
        assert_eq!(
            MarketScenario::LunchChop.name(),
            "Lunchtime Chop (Dead Floor)"
        );
    }

    #[test]
    fn test_switch_to_fed_shock() {
        let mut sim = MarketActivitySim::default();
        let mut controller = MarketScenarioController::default();

        controller.switch_scenario(MarketScenario::FedShock, &mut sim);
        assert_eq!(controller.active_scenario, MarketScenario::FedShock);
        assert!(controller.get_delta(Instrument::Us30) < -0.80);
        assert!(controller.get_delta(Instrument::Gold) < -0.80);
        assert!(controller.get_volume_multiplier(Instrument::Gold) >= 3.0);
    }

    #[test]
    fn test_switch_to_lunch_chop() {
        let mut sim = MarketActivitySim::default();
        let mut controller = MarketScenarioController::default();

        controller.switch_scenario(MarketScenario::LunchChop, &mut sim);
        assert_eq!(controller.active_scenario, MarketScenario::LunchChop);
        for inst in Instrument::ALL.iter() {
            assert_eq!(controller.get_delta(*inst), 0.0);
            assert!(controller.get_volume_multiplier(*inst) <= 0.50);
        }
    }
}
