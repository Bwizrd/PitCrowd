# Roadmap

Where the project could go next. None of it is promised; it is a menu. Pieces marked
**good first issue** are small and self-contained, a good way to learn the code.

## 1. Use Jev better

Jev is TypeSafe's "System One" market classifier. The experiments here are a newcomer's first
attempt, and someone who knows Jev better may get much more from it.

- Better inputs: what the relay sends Jev for each read (`opencry-visual/src/jev.rs`) and how
  the reply is turned into a trader's decision.
- Jev as a **filter** on a strategy that already has an edge, rather than as a signal on its
  own. The evolving pit's "Jev twins" are set up for this (`evolve/README.md`).
- Cheaper reads: when to ask Jev, and when the free rule of thumb is enough.
- Score your changes with `jev_score` and the audit harness (`evolve/audit/`).

## 2. A floor you can try in the browser

- **A hosted demo on GitHub Pages.** The page already builds for wasm; it needs the synthetic
  market to run inside the page instead of in `floor_tape`, so it works with no server.
- A mobile-friendly layout for the panels.

## 3. More real data

- An adapter for Kraken's ETH/USD, next to `adapters/kraken_btc.py` (same public API, no
  key), plus what the tape and floor need to carry a tenth symbol.
- **good first issue:** an example adapter in another language (JavaScript, Go, ...) for the
  tape's ingest protocol (`docs/DATA_SOURCES.md`).
- Other exchanges' public order books driving the level-2 views.
- More CSV formats for replaying recorded history.

## 4. Make the pits more real with level 2

Ideas the order-book data already supports (`opencry-visual/docs/LEVEL2.md`):

- Resting orders as people queuing on the pit steps; bigger orders, more people.
- **good first issue:** pulled orders shown as a figure walking away, so spoofing is
  something you can watch (`BookEvent::Pulled` is the hook).
- Absorption: aggressive trades hitting a level that does not shrink, shown as a scrum.
- The crowd leaning toward the heavier side of the book.
- A depth ladder as a physical board beside the pulpit.

## 5. Traders and the evolving pit

- **good first issue:** a new hand-written paper trader (an archetype in
  `opencry-visual/src/paper.rs`) with a unit test of its rule.
- More realistic costs and fills.
- Walk-forward and out-of-sample checks built into the evolving pit's seating rules.

## 6. Engineering

- One Cargo workspace for the four crates.
- Profiling the wasm build on slower machines.
- A container image that runs the whole floor with one command.
