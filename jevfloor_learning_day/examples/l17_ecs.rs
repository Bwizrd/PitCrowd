// #### Step 1: Types & System

use bevy_ecs::prelude::*;

// 1. Components (data attached to entities):
#[derive(Component)]
struct Trader;

#[derive(Component)]
struct Activity(u8);

// 2. Resource (global world state):
#[derive(Resource)]
struct MarketPulse(u8);

// 3. System: queries entities with Trader + Activity, reads the MarketPulse resource:
fn pulse_activity(pulse: Res<MarketPulse>, mut query: Query<&mut Activity, With<Trader>>) {
    for mut activity in &mut query {
        activity.0 += pulse.0;
    }
}

// #### Step 2: main (Headless World & Schedule)

fn main() {
    // 1. Initialize the ECS World:
    let mut world = World::new();

    // 2. Insert the global resource:
    world.insert_resource(MarketPulse(3));

    // 3. Spawn a Trader entity with Activity starting at 0:
    let trader_entity = world.spawn((Trader, Activity(0))).id();

    // 4. Create a Schedule and register our system:
    let mut schedule = Schedule::default();
    schedule.add_systems(pulse_activity);

    // 5. Run the schedule once (activity: 0 + 3 = 3):
    schedule.run(&mut world);
    let act1 = world.get::<Activity>(trader_entity).unwrap();
    println!("activity={}", act1.0);

    // 6. Run the schedule a second time (activity: 3 + 3 = 6):
    schedule.run(&mut world);
    let act2 = world.get::<Activity>(trader_entity).unwrap();
    println!("activity={}", act2.0);
}
