# OpenCry --- Weekend Graphical Prototype Plan

**Goal:** Have a recognisable, interactive OpenCry graphical prototype
running by the end of the weekend.\
**Technology:** Rust + Bevy\
**Scope:** Graphics and interaction only. Fake market activity. No live
feeds, Jev, backtesting, or real trading logic yet.\
**Learning goal:** Build enough of this yourself to understand the Bevy
ECS, camera, entities, components, systems, and basic rendering rather
than asking AI to generate the whole application.

------------------------------------------------------------------------

# 1. What You Should Have by Sunday Evening

The weekend prototype should open a desktop window showing an
isometric-style market floor containing multiple instrument pits.

A successful prototype should let you:

-   see a large market floor;
-   see at least 6--10 separate labelled pits;
-   recognise instruments such as Gold, Silver, GBP/USD, EUR/USD, DXY,
    S&P 500, Nasdaq, Oil and Bitcoin;
-   see simple trader entities moving inside/around each pit;
-   immediately see that some pits are much busier than others;
-   see activity change over time using fake data;
-   pan around the exchange;
-   zoom from whole-market view toward an individual pit;
-   click/select a pit;
-   visually highlight the selected pit;
-   display basic information for that pit;
-   maintain a smooth frame rate with hundreds of simple agents.

It does **not** need to look beautiful yet.

A good Sunday result is something where you can screen-record 20 seconds
and another person immediately understands:

> "It's a graphical trading floor where each pit represents a different
> market."

That is the objective.

------------------------------------------------------------------------

# 2. Explicitly Out of Scope This Weekend

Do not add:

-   live market feeds;
-   WebSockets;
-   Jev;
-   LLMs;
-   real strategies;
-   brokerage accounting;
-   real P&L;
-   historical replay;
-   databases;
-   order books;
-   research agents;
-   networking;
-   realistic humans;
-   complex 3D assets;
-   multiplayer;
-   WebAssembly.

If you finish early, improve the visual language and interaction before
adding backend complexity.

The weekend is about proving:

> **Can OpenCry work visually?**

------------------------------------------------------------------------

# 3. Recommended Visual Approach

For this first prototype, use **2D/2.5D isometric presentation**, not a
fully 3D world.

Reasons:

-   faster to build;
-   easier camera;
-   easier selection;
-   easier entity counts;
-   easier labels;
-   easier to understand Bevy;
-   enough to prove the concept.

Represent traders initially as:

-   circles;
-   small rectangles;
-   simple sprites;
-   coloured capsules.

Do not spend Saturday looking for character art.

The movement and density are what matter.

------------------------------------------------------------------------

# 4. Prototype Mental Model

The world should conceptually look like:

``` text
                  OPENCRY MARKET FLOOR

       GOLD                         SILVER
    +----------+                 +----------+
    | o o  o   |                 |oooooooooo|
    |  o   o   |                 |oooooooooo|
    | o     o  |                 |oooooooooo|
    +----------+                 +----------+

            DXY                      OIL
       +----------+             +----------+
       | o   o    |             | ooooo    |
       |    o     |             | oooooo   |
       +----------+             +----------+

    GBP/USD       EUR/USD       S&P 500       NASDAQ
   +-------+     +-------+     +-------+     +-------+
   | o     |     | o o   |     |oooo   |     |oooooo |
   |       |     |   o   |     |ooooo  |     |oooooo |
   +-------+     +-------+     +-------+     +-------+
```

But laid out diagonally/isometrically rather than as a flat dashboard.

The important visual contrast is:

``` text
SILVER
^^^^^^^^^^^^
FRANTIC
lots of movement
large crowd

GBP/USD
^^^^^^^^^^^^
QUIET
few traders
little movement
```

Even with primitive graphics, that should be obvious.

------------------------------------------------------------------------

# 5. Weekend Architecture

Keep the architecture tiny.

``` text
src/
|
|-- main.rs
|
|-- market.rs
|   |-- Instrument
|   `-- MarketActivity
|
|-- pit.rs
|   |-- Pit
|   `-- spawn_pits
|
|-- trader.rs
|   |-- Trader
|   |-- Velocity
|   `-- movement systems
|
|-- camera.rs
|   `-- pan / zoom
|
`-- ui.rs
    `-- selected pit information
```

Do not create the full production workspace yet unless you specifically
want to practise workspaces.

For this prototype, one Bevy application crate is enough.

------------------------------------------------------------------------

# 6. Core Domain Types

Start with a small enum.

``` rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Instrument {
    Gold,
    Silver,
    GbpUsd,
    EurUsd,
    Dxy,
    Sp500,
    Nasdaq,
    Oil,
    Bitcoin,
}
```

Create a method for display names:

``` rust
impl Instrument {
    fn name(&self) -> &'static str {
        match self {
            Instrument::Gold => "GOLD",
            Instrument::Silver => "SILVER",
            Instrument::GbpUsd => "GBP/USD",
            // ...
        }
    }
}
```

This is already useful Rust practice:

-   enums;
-   derives;
-   methods;
-   exhaustive matching.

------------------------------------------------------------------------

# 7. Representing a Pit

Each pit should be a Bevy entity.

Conceptually:

``` rust
#[derive(Component)]
struct Pit {
    instrument: Instrument,
    activity: f32,
}
```

You may also want:

``` rust
#[derive(Component)]
struct PitBounds {
    width: f32,
    height: f32,
}
```

Each pit needs:

-   world position;
-   dimensions;
-   instrument;
-   current activity level;
-   label;
-   selection state.

Do not put all pits inside one giant `MarketFloor` object.

Let ECS entities represent them.

------------------------------------------------------------------------

# 8. Representing a Trader

Each visible trader is another entity.

For the first version:

``` rust
#[derive(Component)]
struct Trader {
    instrument: Instrument,
}
```

Add:

``` rust
#[derive(Component)]
struct Velocity(Vec2);
```

Potentially:

``` rust
#[derive(Component)]
struct HomePit(Entity);
```

The trader does not need:

-   a strategy;
-   P&L;
-   brokerage;
-   AI;
-   order state.

Yet.

This weekend a trader exists to communicate **market activity
visually**.

------------------------------------------------------------------------

# 9. Saturday Morning --- Get Bevy Running

## Target

A Bevy window opens.

### Step 1

Create the project:

``` bash
cargo new opencry-visual
cd opencry-visual
```

Add Bevy using the current recommended dependency version from Bevy's
official documentation.

Then get the smallest possible application running.

Conceptually:

``` rust
use bevy::prelude::*;

fn main() {
    App::new()
        .add_plugins(DefaultPlugins)
        .run();
}
```

### Step 2

Add a camera.

### Step 3

Render one simple shape/sprite.

### Step 4

Move it using a system.

Do not proceed until you understand:

-   what `App` is;
-   what a plugin is;
-   what a system is;
-   what a component is;
-   what an entity is.

## Saturday morning success criterion

One visible object moves around a Bevy window.

------------------------------------------------------------------------

# 10. Saturday Late Morning --- Build One Pit

Create one large floor area representing Silver.

It can simply be:

-   a rectangle;
-   a border;
-   a dark/light contrasting surface;
-   a label.

Spawn:

``` text
SILVER
```

above it.

Then spawn perhaps:

``` text
30 traders
```

inside it.

Give each trader:

``` text
Trader
Velocity
Transform
```

Have them move randomly.

When they hit the pit boundary, reverse/change direction.

Do not worry about physically perfect collision.

## Success criterion

You have a labelled Silver pit containing 30 independently moving
entities.

At this point, stop for ten minutes and make sure you understand the ECS
relationship:

``` text
Entity 57
    Trader(Silver)
    Velocity(...)
    Transform(...)
    Sprite(...)
```

The entity is essentially an ID.

The data lives in components.

Systems operate over component combinations.

That mental model matters more than finishing quickly.

------------------------------------------------------------------------

# 11. Saturday Afternoon --- Multiple Pits

Create a configuration structure:

``` rust
struct PitDefinition {
    instrument: Instrument,
    position: Vec2,
    activity: f32,
}
```

Create definitions for:

``` text
Gold
Silver
GBP/USD
EUR/USD
DXY
S&P 500
Nasdaq
Oil
Bitcoin
```

Use a startup system to spawn them.

Lay them out in a visually interesting grid.

Example conceptual arrangement:

``` text
              GOLD        SILVER

        DXY                     OIL

   GBP/USD    EUR/USD     S&P     NASDAQ

                 BITCOIN
```

Think of asset classes spatially.

Possible future layout:

``` text
FX district
Metals district
Indices district
Energy district
Crypto district
```

That could eventually help visual cognition.

For the prototype, simply make it attractive and readable.

------------------------------------------------------------------------

# 12. Fake Activity Model

Create a component/resource representing fake activity.

For example:

``` rust
struct MarketActivity {
    level: f32,
}
```

or store activity directly on `Pit`.

Normalise activity:

``` text
0.0 = almost dead
1.0 = extremely active
```

Initially hard-code:

``` text
Gold       0.45
Silver     0.95
GBP/USD    0.12
EUR/USD    0.25
DXY        0.55
S&P 500    0.62
Nasdaq     0.80
Oil        0.70
Bitcoin    0.50
```

Map activity onto:

### Trader count

Higher activity -\> more visible traders.

### Movement speed

Higher activity -\> faster traders.

### Direction changes

Higher activity -\> more frantic movement.

### Animation later

Higher activity -\> more energetic animation.

For Saturday, trader count + speed is enough.

------------------------------------------------------------------------

# 13. Important Design Rule: Density Must Read at a Glance

Zoom all the way out.

Ask yourself:

> Without reading a single number, can I tell Silver is active and
> GBP/USD is quiet?

If not, improve the visual mapping.

The project depends on this.

Possible tools:

-   crowd size;
-   movement speed;
-   pit occupancy;
-   subtle pulsing;
-   agent clustering;
-   pit label activity indicator.

Avoid relying primarily on colour.

Movement and density are more interesting and align with the product
idea.

------------------------------------------------------------------------

# 14. Saturday Late Afternoon --- Camera

Implement:

-   zoom in;
-   zoom out;
-   pan left/right/up/down.

Possible controls:

``` text
Mouse wheel     Zoom
WASD            Pan
Arrow keys      Pan
Middle drag     Optional pan
```

Do not implement every camera interaction immediately.

Mouse wheel + WASD is enough initially.

## Zoom objective

At maximum zoom-out:

``` text
whole exchange visible
```

At close zoom:

``` text
one pit fills most of screen
individual traders clearly visible
```

This is essential to the OpenCry concept.

------------------------------------------------------------------------

# 15. Isometric Look

You do not need a true isometric renderer immediately.

The first goal is an **isometric visual impression**.

Possible approach:

-   arrange floor geometry diagonally;
-   use diamond/parallelogram pit surfaces;
-   offset rows;
-   position labels appropriately;
-   use simple shadows/depth ordering.

Conceptually convert logical grid coordinates:

``` text
grid x/y
   |
   v
isometric screen x/y
```

A classic conceptual transform is:

``` text
screen_x = (grid_x - grid_y) * tile_width / 2
screen_y = (grid_x + grid_y) * tile_height / 2
```

You do not have to build a tile engine.

A helper function converting logical floor positions to isometric
positions is sufficient.

## Exercise

Write the coordinate conversion yourself.

This is a good small Rust/math task and makes the floor immediately look
more game-like.

------------------------------------------------------------------------

# 16. Saturday Evening Target

By the end of Saturday aim for:

``` text
[✓] Bevy application
[✓] Camera
[✓] Multiple pits
[✓] Instrument labels
[✓] Hundreds of traders
[✓] Different activity levels
[✓] Trader movement
[✓] Zoom
[✓] Pan
[✓] Rough isometric appearance
```

If you have this, stop.

Do not spend midnight integrating Binance.

You have achieved Saturday's goal.

------------------------------------------------------------------------

# 17. Sunday Morning --- Make the Market Alive

Hard-coded activity is enough to prove rendering, but Sunday should make
the world change.

Create a fake market simulator.

Every few seconds adjust pit activity.

For example:

``` text
Silver

0.40
0.46
0.58
0.72
0.91
0.98
```

As activity increases:

-   more traders appear;
-   existing traders move faster.

As it decreases:

-   traders leave/despawn;
-   movement slows.

Now the exchange starts feeling alive.

## Important

Make changes gradual.

Do not randomly jump:

``` text
0.1 -> 0.9 -> 0.2
```

Use smooth transitions.

Eventually real market data will replace the fake activity source.

The renderer should not care where activity came from.

------------------------------------------------------------------------

# 18. Separate Data From Visualisation

This is worth doing even in the prototype.

Avoid:

``` text
random generator
    directly changes sprite
```

Prefer:

``` text
Fake Market Source
        |
        v
MarketActivity
        |
        v
Visualisation Systems
        |
        v
Trader count / speed / behaviour
```

Later:

``` text
Real Market Source
        |
        v
MarketActivity
        |
        v
same visualisation systems
```

That small separation will save a rewrite.

------------------------------------------------------------------------

# 19. Sunday Late Morning --- Pit Selection

Allow the user to click a pit.

When selected:

-   highlight its border/surface;
-   store selected instrument;
-   show information panel.

Example:

``` text
SILVER
XAG/USD

Activity        94%
Visible agents  87
State           VERY ACTIVE
```

This information is fake for now.

The interaction is real.

## ECS concept

Consider a resource:

``` rust
#[derive(Resource, Default)]
struct Selection {
    pit: Option<Entity>,
}
```

This is a good introduction to Bevy resources.

Learn the distinction:

``` text
Component
```

belongs to an entity.

``` text
Resource
```

is shared application-level state.

------------------------------------------------------------------------

# 20. Sunday Afternoon --- Semantic Zoom

Do not build a complex LOD engine.

Implement the first hint of semantic zoom.

## Far away

Show:

-   pits;
-   labels;
-   aggregate crowds.

Hide:

-   detailed trader labels;
-   unnecessary UI.

## Close

Show:

-   individual agents clearly;
-   selected pit details;
-   perhaps firm placeholders.

You can initially base this simply on camera zoom.

This proves one of OpenCry's most important UX ideas.

------------------------------------------------------------------------

# 21. Add Brokerage Colours If Time Allows

If the main prototype is working, add:

``` rust
enum Firm {
    Atlas,
    Redwood,
    Northstar,
}
```

Assign each trader a firm.

Visually distinguish firms.

Do **not** add strategies yet.

The only purpose is to demonstrate that a crowd can contain different
simulated organisations.

When zoomed into Silver you should be able to see:

``` text
Atlas agents
Redwood agents
Northstar agents
```

intermingled.

This is optional weekend polish.

------------------------------------------------------------------------

# 22. Optional Sunday Feature --- Activity Shock

Add a keyboard shortcut:

``` text
1 -> Silver activity shock
2 -> Gold activity shock
3 -> Nasdaq activity shock
```

When pressed, that market's activity rapidly increases.

This gives you an excellent demo.

Example:

Start with the entire floor.

Press:

``` text
1
```

Silver suddenly becomes busy.

Traders enter.

Movement increases.

The eye is immediately drawn toward Silver.

Then zoom into it.

This demonstrates the entire OpenCry visual hypothesis without requiring
real market data.

------------------------------------------------------------------------

# 23. Trader Movement

Do not implement sophisticated pathfinding.

For now:

1.  each trader has a target;
2.  target lies inside its pit;
3.  trader moves toward target;
4.  when close, select another target;
5.  activity influences speed and target-change frequency.

Conceptually:

``` rust
#[derive(Component)]
struct TraderMovement {
    target: Vec2,
    speed: f32,
}
```

This will look better than purely Brownian/random movement.

At high activity, agents can:

-   move faster;
-   change targets more frequently;
-   cluster toward an activity centre.

At low activity:

-   slower movement;
-   longer idle periods.

------------------------------------------------------------------------

# 24. A Better Fake Silver Scenario

Rather than completely random activity, script a short scenario.

## 0--10 seconds

Everything normal.

``` text
Silver 0.30
Gold   0.35
GBPUSD 0.20
```

## 10--20 seconds

Gold activity rises.

``` text
Gold 0.35 -> 0.70
```

## 20--35 seconds

Silver begins reacting.

``` text
Silver 0.30 -> 0.95
```

Silver crowd expands and accelerates.

## 35--45 seconds

Nasdaq quietens.

## 45--60 seconds

Silver remains dominant.

This gives the prototype a narrative.

Later real market data provides the narrative automatically.

------------------------------------------------------------------------

# 25. Visual Debugging Overlay

Add a developer toggle such as:

``` text
F1
```

Show:

``` text
FPS
Entity count
Trader count
Selected pit
Camera zoom
```

Potentially later:

``` text
ECS timings
market events/sec
```

This begins the observability mindset from the full OpenCry
specification.

------------------------------------------------------------------------

# 26. Performance Experiment

Once everything works, test:

``` text
100 traders
500 traders
1,000 traders
5,000 traders
10,000 traders
```

Record:

``` text
Trader count | FPS
-------------|----
100          |
500          |
1,000        |
5,000        |
10,000       |
```

Do not optimise yet.

Observe.

This is your first useful Bevy/Rust performance experiment.

Ask:

-   where does FPS begin falling?
-   is movement logic expensive?
-   is rendering expensive?
-   how much memory is being used?

This turns the prototype into a genuine Rust learning exercise.

------------------------------------------------------------------------

# 27. Rust Concepts You Should Learn This Weekend

Do not try to master the entire language.

Focus on what Bevy exposes naturally.

## Struct

``` rust
struct TraderMovement { ... }
```

## Enum

``` rust
enum Instrument { ... }
```

## Trait derives

``` rust
#[derive(Component)]
```

You do not need to understand procedural macros internally yet.

## References

Bevy queries will expose:

``` rust
&Trader
&mut Transform
```

Pay attention to why one is immutable and one mutable.

## Query

Understand conceptually:

``` rust
Query<(&Trader, &mut Transform)>
```

as:

> Give this system access to every entity containing both of these
> components.

## Resource

Understand:

``` rust
Res<T>
ResMut<T>
```

as shared read-only versus mutable resource access.

## Commands

Understand deferred entity creation/destruction.

## Option

Use for selected pit.

## Match

Use for instrument names and behaviour.

That is enough Rust focus for the graphics weekend.

------------------------------------------------------------------------

# 28. Questions to Ask Yourself While Coding

When writing a Bevy system, ask:

### What data does this system read?

Example:

``` rust
&Trader
```

### What data does it modify?

Example:

``` rust
&mut Transform
```

### Is this global state or entity state?

Global:

``` text
Resource
```

Entity-specific:

``` text
Component
```

### Should this system know where activity came from?

Usually no.

### Is this rendering logic or market logic?

Keep the distinction visible even in the prototype.

------------------------------------------------------------------------

# 29. How Much AI to Use This Weekend

You want visible progress, so it is reasonable to use more AI than in
the pure Rust exercises.

However, use this rule:

> AI may help with Bevy API details. You should understand the Rust
> architecture.

Good request:

> "What's the current Bevy API for spawning a 2D sprite?"

Good request:

> "Explain this Query type."

Good request:

> "My click coordinate conversion is wrong. Explain the coordinate
> spaces."

Less useful for learning:

> "Build the entire OpenCry graphical prototype."

If AI generates a system, read it and make sure you can answer:

-   what does the query borrow?
-   what components does it require?
-   what does it mutate?
-   when does it run?
-   why is it a system?

If you cannot answer those, slow down.

------------------------------------------------------------------------

# 30. Suggested Commit Sequence

Commit frequently so the weekend tells a story.

Suggested commits:

``` text
01 - initialise Bevy application

02 - add camera and basic rendering

03 - add Instrument and Pit components

04 - render labelled market pits

05 - add trader entities

06 - add trader movement

07 - add activity-driven trader density

08 - add camera pan and zoom

09 - add isometric floor layout

10 - add dynamic fake market activity

11 - add pit selection

12 - add market detail panel

13 - add semantic zoom

14 - add brokerage identities

15 - add diagnostics and performance test
```

Do not squash these for the learning repository.

The history is useful evidence of how you built it.

------------------------------------------------------------------------

# 31. Saturday Checklist

## Morning

-   [ ] Create project
-   [ ] Add Bevy
-   [ ] Open window
-   [ ] Add camera
-   [ ] Render one entity
-   [ ] Move one entity

## Midday

-   [ ] Create `Instrument`
-   [ ] Create `Pit`
-   [ ] Render Silver pit
-   [ ] Spawn 30 traders
-   [ ] Keep traders within pit

## Afternoon

-   [ ] Add 6--10 pits
-   [ ] Add labels
-   [ ] Add different activity levels
-   [ ] Spawn hundreds of traders
-   [ ] Make Silver obviously busier than GBP/USD

## Evening

-   [ ] Pan
-   [ ] Zoom
-   [ ] Rough isometric layout
-   [ ] Clean code
-   [ ] Commit

------------------------------------------------------------------------

# 32. Sunday Checklist

## Morning

-   [ ] Fake dynamic activity
-   [ ] Smooth activity transitions
-   [ ] Traders spawn/despawn based on activity
-   [ ] Movement speed follows activity

## Midday

-   [ ] Click/select pit
-   [ ] Highlight selected pit
-   [ ] Display pit information

## Afternoon

-   [ ] Basic semantic zoom
-   [ ] Improve layout
-   [ ] Add brokerage colours if time
-   [ ] Add scripted Silver activity shock

## Evening

-   [ ] Add debug overlay
-   [ ] Test 100--10,000 traders
-   [ ] Fix obvious bugs
-   [ ] Record short demo
-   [ ] Write README screenshot/demo section
-   [ ] Commit working weekend milestone

------------------------------------------------------------------------

# 33. Definition of Done

The weekend is complete when all of these are true:

### Visual

Someone looking at the application understands that each area represents
a financial instrument.

### Activity

Without reading a number, they can identify which market is currently
most active.

### Navigation

You can move from a view of the whole exchange to a close view of
Silver.

### Interaction

You can select Silver and see its information.

### Dynamics

Activity changes cause visible changes in crowd density and behaviour.

### Performance

Several hundred agents run comfortably.

### Learning

You can explain:

-   entity;
-   component;
-   system;
-   query;
-   resource;
-   `&T`;
-   `&mut T`;
-   `Res<T>`;
-   `ResMut<T>`.

### Architecture

Fake market activity is separate from rendering, so real data can
replace it later.

If all of those are true:

**stop adding features.**

The prototype has done its job.

------------------------------------------------------------------------

# 34. What Comes Immediately After This Weekend

The next milestone should **not** be AI.

Replace:

``` text
Fake Market Activity
```

with:

``` text
Canonical Market Activity
```

fed from the market-data layer described in the main OpenCry
specification.

The graphical application should ideally barely notice the change.

That will validate the architectural separation:

``` text
THIS WEEKEND

Fake activity
     |
     v
Market state
     |
     v
Bevy world


LATER

Live market data
     |
     v
Normalisation
     |
     v
Market state
     |
     v
same Bevy world
```

Once that works, the floor becomes a real market visualisation rather
than a graphical prototype.

------------------------------------------------------------------------

# 35. Start Here

For the first session, do only this:

``` text
1. cargo new opencry-visual
2. Add Bevy
3. Open a window
4. Add a camera
5. Render one circle
6. Make the circle move
```

Then commit.

Next:

``` text
7. Turn the circle into a Trader entity
8. Create a Silver Pit entity
9. Spawn 30 traders inside Silver
```

Do not start by designing the entire exchange.

Get one little trader moving around one little Silver pit.

Then multiply it.

By Sunday evening, the aim is to zoom out and watch **Silver going
berserk while GBP/USD has a handful of bored little traders wandering
around**.

If that works, the core visual idea behind OpenCry has been proven.
