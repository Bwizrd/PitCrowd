# Audit of the evolving pit, 2026-10-26

The question: can the pit's search tell an edge from noise, and is what it seated real?
Everything here was run on a frozen snapshot of edge-lab's 15m cache (`data/real/`, taken
2026-10-26 15:3x UTC) and the 15,000 genomes tried up to then (`trials_15000.jsonl`). On the
snapshot all 13,670 genomes with trades reproduce their recorded gate verdict exactly. Data
and outputs are git-ignored; the scripts rebuild them:

| step | script | output |
|---|---|---|
| 1 null worlds | `null_worlds.py make/search/summary/gates/ideas` | `out/search_*.jsonl`, `out/ideas.txt`, `out/gates.txt` |
| 1 monkeys | `monkeys.py <genome-id>` | `out/monkeys_<id>.json` |
| 2 trial corrections | `.venv/bin/python snooping.py` (numpy, scipy, arch 8.0) | `out/snooping.json` |
| 3 second feed, costs | `oanda.py fetch`, `oanda.py cache`, `costs.py` | `data/oanda/`, `out/costs.json` |
| 4 folds | `walkforward.py` | `out/walkforward.json` |
| 5 forward rule | `forward_rules.py <backtest output>` | (printed) |

`out/trials_trades.jsonl` (every trial's trades on the snapshot) is made by piping the
genomes of `trials_15000.jsonl` through `genome_backtest --cache data/real --edge-dir
../edge_lab/signals`. The venv: `python3 -m venv .venv && .venv/bin/pip install arch`.

## The seated three
| | idea | backtest IS | holdout |
|---|---|---|---|
| Jax9 g00815 | NY 60-min opening range, follow, US500/NAS100/US30/UK100, stop 3 ATR, 3 R | +0.069 R, t 2.08, n 1054 | +0.068, t 1.41, n 473 |
| Rex119 g10941 | RSI(3) beyond 5/95, follow, NY window, same basket, stop 1.5 ATR, 1.5 R | +0.131, t 3.70 | +0.057, t 1.10 |
| Edda157 g14368 | Rex119 plus a 50-bar trend filter | +0.128, t 3.61 | +0.062, t 1.19 |

Rex119 and Edda157 share 99.1% of their entries (daily R correlation 0.989): one trader in
two seats. The idea rule counts the filter as a different idea, though it almost never binds.

## Findings
**Null worlds** (`null_worlds.py`): the search as it runs (400 a cycle, 30% mutants of the 30
best in-sample, the same gate and N), 15,000 genomes a world, edge-lab genomes left out (their
entries come from edge-lab's Python). Passers that share half their entries are one idea.

| worlds | genomes | passes per 1,000 | independent passing ideas | per 1,000 |
|---|---|---|---|---|
| real data: the real search | 15,000 | 0.40 | 2 (ORB x4, RSI x2) | 0.13 |
| real data: the search re-run, another seed | 15,000 | 3.33 | 1 (RSI x50) | 0.07 |
| **flip** x3: every bar mirrored at random (no drift, no direction in anything) | 45,000 | 0.20 | 2 (Asian-range fade on FX x6; band follow at London on indices x3) | 0.044 |
| block x3: 2-hour runs of real bars, by time of day (keeps drift and short-range structure) | 45,000 | 14.0 | 8, 7 of them in one world (follow-mode NY index ideas, RSI among them, x623) | 0.18 |

- On clean noise the gate passes a genome in two of three searches. Those passers look exactly
  like the seated ones: in-sample t 2.0-2.8, holdout t 1.0-1.8, 1,100-1,300 trades.
- Genomes passing the gate's in-sample half (1+ a weekday, 200+ trades, t >= 2, +0.03 R): 5,
  45 and 6 in the flip worlds, 114 in the real search and 304 in the re-run. Real data has far
  more in-sample structure than noise; the holdout then lets through about the same number of
  ideas from either.
- At the flip rate a 15,000-genome search expects 0.66 passing ideas from noise alone; the real
  search found 2. Two or more happens by chance with probability 0.14. Not a discovery.
- The block worlds keep the real drift and two hours of real structure at a time, and pass
  follow-mode NY index ideas (RSI, runs, breakouts) in quantity: the family Rex119 and Edda157
  belong to is what drift and short-range structure produce. The ORB was never passed in a
  null world.

What other thresholds would have done (`out/gates.txt`, passes per 1,000; flip = the clean null):

| gate | real search | re-run | flip | block |
|---|---|---|---|---|
| as written | 0.40 | 3.33 | 0.20 | 14.0 |
| IS t >= 1.5 | 0.80 | 4.33 | 0.38 | 15.1 |
| IS t >= 2.5 | 0.13 | 2.00 | 0.04 | 12.8 |
| IS t >= 3.0 | 0.13 | 0.27 | 0 | 10.6 |
| HO t >= 0.5 | 2.07 | 8.00 | 0.38 | 16.5 |
| HO t >= 1.5 | 0 | 1.13 | 0.07 | 8.8 |
| 0.5 trades a weekday | 2.67 | 4.40 | 0.38 | 16.8 |
| 0.25 trades a weekday | 3.13 | 4.73 | 0.87 | 17.4 |

**Monkeys** (1,000 each; the genome's exits, window, basket and trade count, entries moved):
each seated genome beats all but 0-13 of 1,000 monkeys of every kind (same bars with a coin-flip
side; whole days moved, own side; both). Its signal picks direction and day better than its
exposure does. This is a test of one genome, not of the search: the best of 15,000 tries would
be expected to look like this. The null worlds are the test that includes the selection.

**Corrections for the search** (`snooping.py`):
- effective independent trials, from the eigenvalues of the 13,670 genomes' daily R: 227
  (participation ratio), 1,187 (95% of variance), 2,261 (Li-Ji), against 15,000 tried;
- Hansen's SPA and Romano-Wolf StepM (`arch`, stationary bootstrap of days, 4,529 genomes
  trading 1+ a weekday): consistent p = 0.61 on the whole history, 0.39 in-sample, 0.94 on
  the holdout. StepM finds no genome better than not trading;
- deflated Sharpe, in-sample: the trials' t-statistics spread with a standard deviation of
  1.89 (not the 1.0 edge-lab's harness assumes), centred at -1.05. With that spread the DSR of
  all three is 0.00-0.05 at any N from 227 to 15,000. The harness form gives Jax9 0.03-0.23
  and Rex119 0.40-0.81, still short of 0.95;
- PBO by CSCV (16 blocks, 12,870 splits): 0.036 over the weekday-eligible genomes, 0.09 over
  all. The search's in-sample best usually stays above the median out of sample (median
  +0.057 R a day; below zero in 20% of splits): rankings persist, mostly because costs and
  frequency persist. Low PBO says the ranking is not random; it does not say the best is good.

**Second feed and costs** (`costs.py`, OANDA 15m bid/ask, 108 paced requests to the practice
API): the two feeds' 15m bars correlate 0.991-0.996. On OANDA's bars all three stay positive
(Jax9 +0.061 R, Rex119 +0.086, Edda157 +0.083) and at OANDA's own per-bar spreads all three
still pass the gate, Jax9 by nothing (IS t 2.00). One extra tick against every fill, in and
out, and none passes: Jax9 +0.048 (IS t 1.40), Rex119 +0.074 with holdout t 0.52. OANDA's
spreads at the genomes' entries are at or under the backtest's typical ones (US500 1.6 ticks,
NAS100 2.0-2.8 against 4.4); the desk's own cTrader fills match the typical spreads (they are
where they came from). The demo account's `/deals` was not read: it is on the live bridge,
which this project does not touch. So slippage beyond the quote is unmeasured, and an edge of
one to two ticks a trade is what it would have to leave.

**Folds** (`walkforward.py`, half-years): Jax9 is positive in 9 of 12 half-years, none above
t 2.0; Rex119's in-sample t leans on 2021H1 (+0.376 R, t 3.7). All three are negative so far
in 2026H2 (82-86 trades, t -0.7 to -0.8). The selection walked forward, anchored (the gate's
in-sample rules on all data before each half-year, 5 days embargoed): the best pick earned
+0.033 R a trade in the half-year after (1,405 trades, t 1.01); every pick together +0.025.
The picks' half-years 2025H2, 2026H1 and 2026H2 are all negative (t -2.2, -1.5, -2.9).

**Forward rule** (`forward_rules.py`): an edge the size of the seated three's needs 1,500 to
2,500 forward trades for a fixed 95% test at 80% power, 6 to 9 years at five a week. The
first forward cuts killed a genome with its own holdout edge 81-87% of the time within about
70 trades. Replaced by Wald's SPRT on 2026-10-26 (README, "Forward cuts"; commit 2095d95),
before any forward trade was looked at: it cuts a no-edge genome 90-94% of the time and keeps
a true one 71-80%, at a median of 850-1,300 trades.

## What this says
- **The gate is not tight enough for a search this size.** Its thresholds are fixed while
  the search grows; on clean noise it passes about one idea per 23,000 genomes, and the pit
  tries 9,600 a day. Its passers are indistinguishable by their numbers from noise passers,
  and SPA/StepM over everything tried finds nothing better than not trading.
- **What it found is real structure, but small, fragile and fading.** The seated genomes beat
  matched random entries at p <= 0.001 each, replicate on OANDA's feed, and survive OANDA's
  spreads. They do not survive one extra tick a side, and the gate's picks have been negative
  in every half-year since mid-2025.
- **Of the three, Jax9's idea is the one the nulls never produced.** Rex119 and Edda157 are one
  trader, in the family the drift-keeping null produces in quantity.
- **Forward trades will decide, slowly.** The SPRT is in place; an edge this size takes three
  to five years to confirm, and no backtest test here can shorten that.

The thresholds are Paul's call (README: changed only in the open, with the reason). The
numbers above would support, in order: (1) an in-sample bar that rises with the effective
number of genomes tried (the README's own principle, today applied only to seating order);
IS t >= 3.0 is where clean noise stopped passing in 45,000 genomes; (2) a cost stress, the
gate passed at one extra tick a side, which none of the three meets; (3) one seat per idea by
shared entries (half or more) rather than by the genome's fields. Nothing has been changed.
