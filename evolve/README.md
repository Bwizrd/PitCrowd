# The evolving pit

Paper traders made of data: each **genome** is one idea on 15m bars (signal, follow or fade,
filters, entry hours, exit, basket of symbols; see `opencry-visual/src/genome.rs`). `evolve.py`
spawns them, `genome_backtest` tests them on edge-lab's 15m history (2021-01 to now, eight desk
symbols, the desk's own costs and rules), and the paper desk seats the survivors on the live
tape. Losers are cut on their forward record and their seats refilled. Nothing here touches the
live book.

**In this repo it starts empty and offline.** The history is `data/cache` (`floor_tape
--write-cache`, written by `start-floor.sh` on first run: synthetic, 2021 to now), the referee is
`dsr.py` (the deflated Sharpe, stdlib only), and the edge-lab families (`edge_lab.py`,
`holders.py`) are skipped unless `EDGE_LAB_DIR` points at an edge-lab checkout. On synthetic
history the pit is a null test: the market has no edge, so every genome that passes the gate is
a measured false positive. Point `OPENCRY_EDGE_CACHE` at your own 15m bars (format in
`docs/DATA_SOURCES.md`) to search real data. The numbers and dates below are from the author's
run on a broker's history.

## Why the rules matter
Spawn enough traders and some will be up by luck: the first desk had 10 of 39 in profit with
no edge anywhere. So the bar rises with the number of genomes tried, and only forward trades
can graduate one. These rules were written down on 2026-10-25, before the first real cycle.
Change them only in the open, with the reason, never because a result came out a certain way.

## Seating gate (backtest)
In-sample is before 2025-01-01, the holdout is 2025-01-01 onwards. A genome is seated only if:
- in-sample: at least 200 trades, mean at least +0.03 R per trade, t at least 2.0;
- holdout: at least 60 trades, mean above 0, t at least 1.0;
- at least 1.0 trade per weekday on average, so the forward verdict comes in weeks.

Passers are seated best first by in-sample deflated Sharpe, which counts every genome ever
tried. 40 seats. No two seated genomes may be the same idea: the same entries (signal, mode,
filters, hours, symbols), whatever their exit. (Clarified 2026-10-25, before any genome had
traded: the first cycle seated a copy of the first genome that differed only in its target.)

## Forward cuts (paper desk, trades after seating only)
- **Wald's sequential test (SPRT)** on the genome's forward R per trade: H0 a mean of 0, H1 the
  genome's own holdout mean at seating, normal with its backtest's standard deviation;
  alpha 0.05, beta 0.20. After each trade the log-likelihood ratio
  `sum(mu1 / sd^2 * (r - mu1 / 2))` is compared with ln(0.20/0.95) = -1.56 and
  ln(0.80/0.05) = +2.77. At or below the lower bound the genome is cut ("accepts no edge"); at
  or above the upper it is marked **confirmed**, and the test is over (it stays seated, and
  graduation below still decides whether it is a candidate);
- a safety cut: t at or below -3 after 30 or more trades (a genome that is broken, not unlucky);
- idle: fewer than 20 trades in its first 21 days.

(Changed 2026-10-26, before any forward trade of any genome was looked at. The first rules
were: t <= -2 after 30 trades, negative after 60, under +0.05 R after 150. Simulated on
resampled backtest trades of the three seated genomes (`audit/forward_rules.py`), they cut a
genome whose true edge is its own holdout mean 81-87% of the time, at a median of about 70
trades, because +0.06 R per trade with a standard deviation of 1.1 R cannot show itself in 150
trades. The SPRT cuts a no-edge genome 90-94% of the time and keeps a true edge of that size
71-80% of the time, but a verdict takes a median 850-1,300 trades, three to five years at five
trades a week. That is how long an edge this size takes to see; the old rules only looked
quicker.)

## Graduation
At least 300 forward trades, a mean of at least +0.05 R per trade, and a deflated Sharpe on the
forward trades of at least 0.95, where N is every genome ever seated. A graduate keeps trading
on paper and is flagged as a candidate for live trading; that step is a human decision,
never automatic.

## Jev twins
Every seated genome gets a twin (`Name-J`, switched on 2026-10-25). Whenever the genome enters,
the twin asks Jev one question about the 15m chart, through the floor's relay: for a genome
that follows moves, whether the move is strong and likely to carry on; for one that fades, whether
the move is tiring and likely to turn (`genome::jev_question`). Jev sees the last 24 bars
with no symbol and no dates, prices rebased to 1000.00, the side and the hour. If Jev says yes
(0.5 or more) the twin takes the same trade, seconds later; if not, it stays out. So the twin's
trades are the genome's trades minus the ones Jev doubted, and every answer is logged
(`opencry-visual/log/genome_jev.jsonl`).

The measure is the genome's own forward trades, each tagged with Jev's answer at its entry:
- reported at 60, 150 and 300 tagged trades: R per trade when Jev said yes, when Jev said no,
  and the Welch t of the difference;
- **Jev adds something** to a genome only if, at 300 or more tagged trades, Jev said no to at
  least 15% of them, the yes trades average at least +0.05 R, and the t of yes minus no,
  less the expected maximum of N standard normals (N = twins judged at 300 so far), is at
  least 1.645 (the same 0.95 bar as graduation);
- a twin is never cut on its own; it leaves with its genome. Its own P&L is shown, not judged.

These rules were written before any twin traded.

## Idea families
The first 5,000 genomes drew on six signal families: an n-bar breakout, a band around the
average, an RSI extreme, a run of closes, the London/New York opening range, and yesterday's
high and low. One genome passed. On 2026-10-25, after that, seven families were added, with
no change to the gate, the cuts or graduation, and every genome tried before or after still
counts in N:
- **last week's high and low**, taken out for the first time this week (weeks start Sunday,
  UTC): followed on a close beyond, faded on a bar that trades through and closes back inside;
- **the Asian range** (bars opening 00:00 to 07:00 UTC), broken or swept from 07:00, in the
  same two ways (the London breakout, and the sweep of the Asian high or low);
- **a moving-average cross** (fast 8, 20 or 50 bars over slow 32, 100, 200 or 400);
- **a squeeze**: the previous bar was the narrowest of the last 4, 7, 12 or 20, and this bar
  closes beyond it;
- **a rejection wick**: a bar of at least half an ATR whose wick is 50, 60 or 70% of it and
  which closes in the far third;
- **an engulfing bar** of at least half an ATR;
- **a session-open drive**: price at least 0.5 to 2 ATR from the London or New York open
  after the first 1, 2 or 4 bars.

Also added: a **day filter** (the trade with or against the move since the day's first open),
longer horizons on 15m bars (192-bar breakouts and bands, an 800-bar trend filter, a 64-bar
time stop), and new families drawn twice as often as old ones. A genome is still 15m bars
and one position at a time, flat by the desk's close, so "a longer timeframe" here means a
longer lookback, not an overnight hold. Before the switch, 300 old genomes were backtested
with the old and new code and gave identical trades.

## edge-lab strategies
Also on 2026-10-25: edge-lab's own intraday strategies became one more family, `edge_lab`.
Each is a `positions(bars)` function edge-lab wrote from a trading video. It runs on its own
timeframe, edge-lab's best for it (15m, 30m or 1h), on bars built from the 15m ones on UTC
boundaries; a genome of this kind enters at the close of the 15m bar that closes a bar where
the strategy's position turns long or short, and everything after that
(follow or fade, window, filters, exit, basket, costs, gate, cuts) is an ordinary genome's.
The strategy's own exit is not used. `edge_lab.py` decides which strategies qualify, before
any genome is built from them:
- edge-lab's best timeframe for it was 15m, 30m or 1h, and its edge-lab verdict was any that
  means it ran cleanly (noise, weak, promising, SHORTLIST, is_killed; not rejected, too_few,
  suspicious or too_slow). edge-lab chose that timeframe on its in-sample, the first 70% of
  its history, which ends before our holdout starts, so the choice never saw 2025;
- price only: no volume profile, and the bars' volume column is always zero, on the desk
  as in the backtest. A strategy whose positions on US500's history change when the volume
  column is filled with random numbers reads volume, and is left out. (Amended the same day,
  before any edge-lab genome was judged (one was backtested only to check that its trades open
  on its signal bars, its results unread): the first build passed five-column bars, which
  127 strategies could not unpack; the zero column and this check replaced that.);
- **replay check**: on 60 sampled bars each of US500 and EURUSD (half where it enters), the
  strategy run on only the 2,500 bars the desk keeps gives the same entry as on the whole
  history at least 95% of the time, and one such call takes under a second. This is what
  makes the live desk trade what the backtest tested; a strategy that fails is left out;
- it enters at least once on the eight symbols.

On the desk the entry comes from the same Python, run on the desk's 2,500 bars at each 15m
close (`edge_lab.py live`).

(Amended the same day, after the first cycles: the first build ran every strategy on 15m bars,
so a strategy edge-lab found on 1h bars had lookbacks a quarter as long, which is a different
strategy. 67 genomes were tried that way and stay in N; their signals are kept as `key@15` so
they can be re-run. From then on a strategy is drawn only on its own timeframe, `key@30` or
`key@60` built and checked the same way. The change was made because of what the strategies
are, not because of any result; none of the 67 passed the gate.) Every edge-lab genome counts in N like any other, and edge-lab's
own 6,000-odd tries do not, because none of its verdicts is used as a pass.

## Show floor
Written 2026-10-27, before any show seat traded. Paul asked for a busy floor of different bots to
watch, and for some of them to change tactics as the market changes. The gate above stays as it
is; the show floor is a second set of seats beside it, in `pit.json` under `show`:
- **nothing on it counts**: show seats are not in `seated_ever`, never graduate, and are labelled
  "SHOW FLOOR" on the board. A show seat's SPRT can confirm it (logged `show_confirmed`), which
  makes it worth a look by Paul, nothing more;
- **show genomes (16)**: ordinary genomes that made money in both halves: in-sample at least 200
  trades and a mean above 0, holdout at least 60 trades and a mean above 0, at least 0.5 trades
  per weekday, not edge-lab genomes, not an idea the gate has seated. Best first by the smaller
  of their two t's; at most two of any signal kind; one seat per idea. Id `s_` + the genome's id.
  A show genome the gate later seats leaves the show floor;
- **holders (24)**: one of edge-lab's 4h or daily strategies, traded the way edge-lab wrote it,
  its own entries and its own exits, holding overnight, on one symbol (`holders.py`). Its bars
  are built from the 15m history on UTC boundaries; a change in its position for a finished bar
  is acted on at the next 15m bar (a bar cut short by a market close is acted on when the desk
  learns it is over, at the close of the next 15m bar). One R is two ATR(14) of its own bars at
  entry; a safety stop at 5 R is the only exit that is not the strategy's (at 2 R it took 21% of
  exits in a sample, at 5 R 5%), and after it the holder waits for the strategy's next change.
  Costs are the desk's spreads; overnight financing is not charged. Eligible: in-sample at least
  20 trades and a mean above 0, holdout at least 8 trades and a mean above 0, at least 4 trades a
  year; the strategy may not read volume and its position on the history up to a bar must equal
  its position on the whole history at 95% of 80 sampled bars (no look-ahead). Best first by the
  smaller of the two t's; one per strategy; at most five per symbol; none sharing half its
  entries with another holder on its symbol. On a restart the desk restores open holder
  positions and the strategy states it acted on (`log/holders_state.json`); with no saved state a
  holder joins at its strategy's next change, never mid-trade;
- **adapters (4 pairs)**: each carries 5 tactics, show-pool genomes of different signal kinds.
  Every UTC day at the first 15m close from 06:45 it scores each tactic over the last 20 days of
  the desk's own bars (`genome::backtest` on them). The code adapter takes the best R per trade
  among tactics with 3 or more trades (none: it keeps its tactic); the Jev adapter (`-J`) shows
  Jev the markets (1, 5 and 20-day changes, up days, range against its average) and the same
  records, asks how likely each tactic is to make money today, and takes the highest. The two
  of a pair have the same tactics, so the pair shows whether Jev chooses better than the
  scoreboard. Every choice is logged (`log/adapt.jsonl`). A new pair starts from the two show
  genomes with the best forward record (20 or more trades, mean above 0), then fresh draws: the
  floor's own results feed the next generation;
- **cuts**: show genomes and holders by the SPRT above (H1 their holdout mean, sigma their
  backtest's); every show seat at t <= -2.5 after 40 forward trades. An adapter pair leaves
  together. Seats are refilled from the pools each cycle; a retired seat never returns.

## Files
- `genomes.jsonl`: every genome tried and its backtest numbers (the N the bar counts)
- `pit.json`: the seated genomes, which the desk reloads every minute
- `registry.jsonl`: every event (tried, seated, cut, graduated) and the rule that fired
- `holders.py`, `holders/`: the holders' backtests (`results.jsonl`, per-trade files; git-ignored)
