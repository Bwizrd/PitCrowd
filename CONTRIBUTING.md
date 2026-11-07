# Contributing

Thanks for looking. This started as one person's project to learn Rust and Jev, and it has gone
as far as one person can take it. It gets better with other people's ideas, so contributions of
any size are welcome: a typo, a new feed, a better trader, a new way to use Jev, a bug report.

It is maintained in spare time. Reviews may take a few days, and that is not a lack of
interest.

## Where to start

- [`ROADMAP.md`](ROADMAP.md) lists the directions the project could go, and the smaller
  pieces marked **good first issue**.
- Issues labelled `good first issue` on GitHub are the same pieces, ready to pick up.
- Have a bigger idea? Open a Discussion or an issue first, so we can agree on the shape before
  you spend time on it.

## Running it

The [README's quick start](README.md#quick-start) is all you need: one script, no account, no
API key. Everything runs offline on a synthetic market. [`docs/SERVICES.md`](docs/SERVICES.md)
explains each service and how to run it by hand.

## Before you open a pull request

Run the same checks CI runs, in each crate you touched
(`floor_tape`, `jev_relay`, `opencry-visual`, `jevfloor_learning_day`):

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets
```

Plus, for the browser build and the scripts:

```sh
(cd opencry-visual && cargo clippy --target wasm32-unknown-unknown --lib --bin opencry-visual -- -D warnings)
node ci/check_pages.mjs
ci/smoke.sh
```

The [Checks](README.md#checks) section of the README has the details.

## House rules

These keep the project easy to run and safe to share:

1. **It must keep running offline with no key.** A new feature that needs an account or a paid
   service is fine as an option, but the default path stays free and local.
2. **No secrets in code, logs or the browser.** API keys live in git-ignored `.env` files and
   only the relay (`jev_relay`) reads the Jev key. Never print a key.
3. **Tests never make a paid call.** No test may call Jev or any other metered API.
4. **No real orders.** This is paper trading and visualisation. A pull request that places
   orders at a real broker will not be merged.
5. **Keep the logic testable.** Pure functions (sizing, fills, book updates, rules) get unit
   tests; Bevy systems stay thin around them.
6. **Say what a result means and what it doesn't.** If you add a strategy or a Jev experiment,
   say how it was tested and on what data. The synthetic market has no edge by design, so a
   profit there is luck, not a finding.

## Licence

By contributing you agree that your work is dual licensed under MIT or Apache-2.0, like the rest
of the project (see the [Licence](README.md#licence) section).

## Conduct

Be kind, be specific, and assume good intent. Critique code and ideas, not people. Harassment
or abuse of any kind gets you removed from the project.
