//! Writes `static/explainer.html`: the guide the app's "Guide [G]" button opens in a new tab.
//!
//! The prose lives here; every table (pits, traders, Jev's questions, the rules) is rendered
//! from the same code the floor runs, so re-running `cargo run --bin explain` after a change
//! to `paper.rs` or `jev.rs` keeps the page honest. trunk copies the file next to the app
//! (`<link data-trunk rel="copy-file" href="static/explainer.html">`), so it is served at
//! `http://<host>:8180/explainer.html`, from any machine on your network.

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "static/explainer.html".to_string());
    let html = page::render();
    std::fs::write(&out, &html).unwrap_or_else(|e| panic!("cannot write {out}: {e}"));
    println!("wrote {out} ({} bytes)", html.len());
}

#[cfg(not(target_arch = "wasm32"))]
mod page {
    use std::fmt::Write as _;

    use opencry_visual::jev::{
        DeterministicModel, JEV_CADENCE_SECS, JevDesk, JevQuestion, all_questions,
    };
    use opencry_visual::levels::tape_symbol;
    use opencry_visual::market::Instrument;
    use opencry_visual::paper::{
        Archetype, ENTRY_FROM, ENTRY_UNTIL, FLAT_AT, JEV_CREW, MIN_STOP_SPREADS,
        MIN_TARGET_SPREADS, READ_FRESH_SECS, STAKE_PER_TICK, Session,
    };

    fn esc(s: &str) -> String {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
    }

    fn hm(minute: i64) -> String {
        format!("{:02}:{:02}", minute / 60, minute % 60)
    }

    fn sessions(s: &[Session]) -> String {
        s.iter()
            .map(|w| format!("{}-{}", hm(w.from), hm(w.until)))
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// Today's date from the system clock, no crate needed (civil-from-days, Howard Hinnant).
    fn today() -> String {
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let z = secs.div_euclid(86_400) + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z - era * 146_097;
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let y = yoe + era * 400;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let d = doy - (153 * mp + 2) / 5 + 1;
        let m = if mp < 10 { mp + 3 } else { mp - 9 };
        let y = if m <= 2 { y + 1 } else { y };
        format!("{y:04}-{m:02}-{d:02}")
    }

    const CSS: &str = r##"
:root { --bg:#0b0d12; --panel:#12151d; --line:#262b38; --fg:#d8dce6; --dim:#9aa3b5; --gold:#f2d35b; --green:#5fd08a; --red:#f06a6a; --blue:#6fb6ff; --pink:#ff7ad0; }
* { box-sizing:border-box; }
body { margin:0; background:var(--bg); color:var(--fg); font:15px/1.55 -apple-system, "Segoe UI", Helvetica, Arial, sans-serif; }
main { max-width:1080px; margin:0 auto; padding:28px 20px 80px; }
h1 { font-size:30px; margin:0 0 6px; color:#fff; }
h2 { font-size:22px; margin:44px 0 10px; color:var(--gold); border-bottom:1px solid var(--line); padding-bottom:6px; }
h3 { font-size:17px; margin:26px 0 8px; color:#fff; }
p, li { color:var(--fg); }
.dim { color:var(--dim); }
code, kbd { font:13px/1.4 ui-monospace, SFMono-Regular, Menlo, monospace; background:#1a1e29; padding:1px 5px; border-radius:4px; color:#e9ecf3; }
kbd { border:1px solid #3a4152; }
table { border-collapse:collapse; width:100%; margin:12px 0 18px; font-size:14px; }
th, td { text-align:left; vertical-align:top; padding:7px 9px; border-bottom:1px solid var(--line); }
th { color:var(--dim); font-weight:600; white-space:nowrap; }
tr:hover td { background:#141823; }
.box { background:var(--panel); border:1px solid var(--line); border-radius:10px; padding:16px 18px; margin:16px 0; }
.box.gold { border-color:#5a4d1e; }
.box.red { border-color:#5a2a2a; }
.swatch { display:inline-block; width:12px; height:12px; border-radius:3px; vertical-align:-1px; margin-right:6px; }
.toc { columns:2; column-gap:30px; font-size:14px; }
.toc a { color:var(--blue); text-decoration:none; }
.toc li { break-inside:avoid; }
a { color:var(--blue); }
.small { font-size:13px; }
.yes { color:var(--green); } .no { color:var(--dim); }
"##;

    pub fn render() -> String {
        let mut h = String::with_capacity(120_000);
        let _ = write!(
            h,
            "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>OpenCry Floor Guide</title><style>{CSS}</style></head><body><main>"
        );
        h.push_str("<h1>OpenCry Floor: the guide</h1>");
        let _ = write!(
            h,
            "<p class=\"dim\">What you are looking at, how Jev works, how the paper traders trade, who they are, how the evolving pit breeds new ones and lets Jev vote on their trades, and where to look to see them trading. Generated {} from the code that runs the floor (<code>cargo run --bin explain</code>).</p>",
            today()
        );

        // ---- the 30-second version -------------------------------------------------
        h.push_str(&r##"<div class="box gold"><h3 style="margin-top:0">The thirty-second version</h3>
<ul>
<li><b>The prices come from the tape</b> (<code>floor_tape</code>): a synthetic market by default, or your own recordings or live feed (<code>docs/DATA_SOURCES.md</code>). The synthetic market has no edge by construction, so on it the experiment is a null test. The crowd inside each pit is theatre: generated figures reacting to the tape's ticks.</li>
<li><b>The traders are paper accounts</b> trading those real prices with honest costs (they buy the ask and sell the bid). They are the experiment. {n_traders} were written by hand (the author's desk ran only four of them from 25 October: Gus, Lena, Tomasz and Colin; <code>--keep</code> chooses), and the rest of the desk is the <b>evolving pit</b>: rules generated by the thousand, backtested, and seated only if they pass a gate written down in advance (<a href="#evolve">section 6</a>). Colin flips a coin: everyone else has to beat him after costs, over weeks, or they have no edge.</li>
<li><b>Jev twins.</b> Every seated genome has a twin (name ending <code>-J</code>) that asks Jev one question as the genome enters and takes the same trade only on a yes. The pair is how we find out whether Jev can improve a strategy, rather than be one.</li>
<li><b>Jev</b> is a classifier you pay per call, and optional: without a key, every read comes from the free floor rule of thumb instead and is labelled FLOOR RULE. The desk sends it the last bars, the tape, the book and the nearest levels once a minute per pit, and again the moment price touches a level (and 30 s later), and asks {n_questions} typed questions in one call: five about the pit, and one per crew question. {n_jev} of the {n_traders} hand-written traders were built on its answers; on the desk today that is Gus and Lena. So far it has shown no edge of its own (<a href="#jev">section 3</a>).</li>
<li><b>Where to see the trading:</b> the Traders panel (top left) is the live scoreboard; the figure labels on the floor show each trader's day P&amp;L and a LONG/SHORT line while they hold something; click a figure or a row for the full picture; the Pit deals toggle shows prints on the chart. The crowd in the pit does not show the paper traders' trades, because the paper traders do not trade with the crowd (see <a href="#pits">why the traders never enter a pit</a>).</li>
<li><b>Entry hours</b> are 07:00-20:30 UTC (08:00-21:30 London time while the clocks are on BST) and everyone is flat by 20:55 UTC. The board is quiet outside those hours by design.</li>
</ul></div>"##.replace("{n_traders}", &Archetype::ALL.len().to_string()).replace("{n_questions}", &all_questions().len().to_string()).replace("{n_jev}", &Archetype::ALL.iter().filter(|a| a.uses_jev()).count().to_string()));

        // ---- contents -------------------------------------------------------------
        h.push_str(
            r##"<ul class="toc">
<li><a href="#floor">1. The floor and its services</a></li>
<li><a href="#screen">2. Reading the screen</a></li>
<li><a href="#jev">3. How Jev works</a></li>
<li><a href="#paper">4. How paper trading works</a></li>
<li><a href="#traders">5. The hand-written traders</a></li>
<li><a href="#evolve">6. The evolving pit and the Jev twins</a></li>
<li><a href="#trader-panel">7. Reading one trader</a></li>
<li><a href="#pits">8. Why the traders never enter a pit</a></li>
<li><a href="#state">9. Where things stand and known gaps</a></li>
<li><a href="#glossary">10. Glossary</a></li>
</ul>"##,
        );

        // ---- 1. floor -------------------------------------------------------------
        h.push_str("<h2 id=\"floor\">1. The floor and its services</h2>");
        h.push_str("<p>The floor is a Bevy app compiled to WebAssembly and served by <code>trunk</code> on port 8180. It is a visualiser and an attention router: it never places an order and it is not the alpha. A handful of small services feed it, all on this machine, none needing an account.</p>");
        h.push_str("<table><tr><th>Service</th><th>Port</th><th>What it does</th></tr>");
        h.push_str("<tr><td>Market tape</td><td>8083</td><td><code>floor_tape</code>: prices for the nine symbols, synthetic by default or from your CSVs or live feed. Broadcasts every tick and the five-level book over a websocket, builds 1m bars from ticks (<code>/bars</code>), serves Bitcoin's Level 2 book in Kraken's message format (<code>/l2</code>), and a retail order book in OANDA's shape for gold, silver, GBP/USD and EUR/USD (synthetic symbols only).</td></tr>");
        h.push_str("<tr><td>Jev relay</td><td>8099 (on macOS a node door in front of the relay on 127.0.0.1:8100)</td><td>Holds the Jev API key, if you have one, so it never reaches the browser, forwards <code>POST /jev</code> with a 2 s minimum gap (or answers 503 at once without a key), logs every inference, and keeps append-only logs of paper trades and decisions. Serves the desk's board at <code>GET /paper/board</code>.</td></tr>");
        h.push_str("<tr><td>Paper desk</td><td>-</td><td>A native binary running the same paper book as the browser, around the clock, on the tape, asking Jev (or, without a key, the floor rule) about all nine pits once a minute each. It runs the hand-written traders (<code>--keep</code> picks some) plus whatever genomes and twins sit in the pit file (<code>--pit evolve/pit.json</code>), which it re-reads every minute. While it is alive the browser shows its board and opens no positions of its own, so nothing is double counted.</td></tr>
<tr><td>Evolve</td><td>-</td><td>A Python loop (<code>evolve/evolve.py loop --every 3600</code>) that every hour judges the seated genomes on their forward trades, cuts the failures, generates and backtests new ones (<code>genome_backtest</code>), and seats any that pass the gate by writing <code>pit.json</code>.</td></tr>");
        h.push_str("<tr><td>trunk serve</td><td>8180</td><td>Serves the app (and this page) to any browser on the machine or on your network.</td></tr></table>");
        h.push_str("<p>Start, stop and check them with <code>./start-floor.sh --status</code> in the JevFloor folder. The nine pits:</p>");
        h.push_str("<table><tr><th>Pit</th><th>Tape symbol</th><th>Retail order book</th></tr>");
        for inst in Instrument::ALL {
            let oanda = matches!(
                inst,
                Instrument::Gold | Instrument::Silver | Instrument::GbpUsd | Instrument::EurUsd
            );
            let _ = write!(
                h,
                "<tr><td>{}</td><td><code>{}</code></td><td class=\"{}\">{}</td></tr>",
                esc(inst.symbol()),
                tape_symbol(inst),
                if oanda { "yes" } else { "no" },
                if oanda {
                    "yes (synthetic on the synthetic tape)"
                } else {
                    "none (OANDA publishes no book for it)"
                }
            );
        }
        h.push_str("</table>");
        h.push_str("<p>On the floor the pits stand in three rows by district: the two metals at the back, the four indices across the middle, the two FX pairs at the front with Bitcoin set apart at the end of their row. Each row has its district name on the concourse behind it, and the wall board above the selected pit carries that pit's symbol in large letters.</p>");

        // ---- 2. screen ------------------------------------------------------------
        h.push_str("<h2 id=\"screen\">2. Reading the screen</h2>");
        h.push_str(r##"<table><tr><th>Panel</th><th>Where</th><th>What it shows</th></tr>
<tr><td>Traders</td><td>top left</td><td>The scoreboard. While the desk is alive it reads "DESK (24h, all pits)": each row is one trader's last 24 hours, realised R, trades, win rate, max drawdown, spread paid, and the open position marked live at the tape. The desk posts every 3 s and the page polls every 2 s, so a close shows within about five seconds. Click a row to open that trader's panel. Sorted best first. The full desk, every close and equity curve over today, 24 h or 7 d, is its own page at <a href=\"/desk.html\">/desk.html</a>, updating live. If the header ever says THIS PAGE IS OUT OF DATE, the desk is running traders this build does not know and the page reloads itself.</td></tr>
<tr><td>Pit status</td><td>under Traders</td><td>The selected pit's live state: price, ticks in, upticks, sound, the crowd tier (quiet, steady, busy, frantic), paper, how loud the pit is.</td></tr>
<tr><td>Board chart</td><td>top right</td><td>Candles of the selected pit built from the tape, with the level bands (swings, order blocks, gaps, round numbers, OANDA clusters), Jev's latest read, the tick delta and ticks-per-bar strips. <kbd>E</kbd> expands it.</td></tr>
<tr><td>Wall board</td><td>in the 3D scene</td><td>The same candles as a board behind the selected pit's crowd. Clicking empty floor takes it down; picking a pit brings it back.</td></tr>
<tr><td>Details, Role key</td><td>right column</td><td>Diagnostics, and the legend for the crowd's costumes.</td></tr>
<tr><td>Trader panel</td><td>bottom left</td><td>Opens when you click a figure on the floor or a row in Traders. See <a href="#trader-panel">section 7</a>.</td></tr>
<tr><td>Figure labels</td><td>over each named trader</td><td>Name and day P&amp;L in R since 00:00 UTC (green up, red down), and a second line, LONG or SHORT and the pit, while they hold a position.</td></tr>
</table>
<p>Every panel has an X; the bottom row of buttons reopens them and the layout is remembered in the browser.</p>
<h3>Keys</h3>
<table><tr><th>Key</th><th>Does</th></tr>
<tr><td><kbd>Space</kbd> / click a pit</td><td>Inspect that pit (camera flies to it); <kbd>Esc</kbd> or <kbd>M</kbd> back to the whole floor</td></tr>
<tr><td><kbd>Tab</kbd></td><td>Next pit</td></tr>
<tr><td><kbd>T</kbd></td><td>Traders panel</td></tr>
<tr><td><kbd>E</kbd></td><td>Expand / restore the board chart</td></tr>
<tr><td><kbd>B</kbd> <kbd>N</kbd> <kbd>H</kbd></td><td>Wall board, board chart, web chart on/off</td></tr>
<tr><td><kbd>C</kbd></td><td>Cycle the timeframe (1m, 5m, 15m)</td></tr>
<tr><td><kbd>O</kbd></td><td>Level bands on/off (all kinds together, OANDA included)</td></tr>
<tr><td><kbd>J</kbd></td><td>Jev reads of the focused pit on/off (the browser's own reads; the desk keeps reading regardless)</td></tr>
<tr><td><kbd>K</kbd></td><td>Role key</td></tr>
<tr><td><kbd>L</kbd></td><td>Cycle the feed mode (live tape / simulated)</td></tr>
<tr><td><kbd>W</kbd> <kbd>A</kbd> <kbd>S</kbd> <kbd>D</kbd> / arrows</td><td>Move the camera</td></tr>
<tr><td><kbd>G</kbd></td><td>This guide</td></tr>
<tr><td><kbd>F1</kbd></td><td>Debug overlay</td></tr>
<tr><td><kbd>F2</kbd>-<kbd>F5</kbd>, <kbd>1</kbd>-<kbd>8</kbd></td><td>Crowd scenarios and manual shocks (theatre only; they never touch prices or the paper book)</td></tr>
</table>"##);

        // ---- 3. Jev ---------------------------------------------------------------
        h.push_str("<h2 id=\"jev\">3. How Jev works</h2>");
        h.push_str("<p>Jev is TypeSafe's \"System One\" classifier. It is not a chat model and it does not write prose: it is given a short structured state and a set of <em>typed</em> questions, and it returns a probability for every allowed answer to each one. We never let it free-text a trade idea; the traders read the numbers.</p>");
        h.push_str("<h3>What Jev is shown</h3><p>One observation per call, for one pit, built by <code>JevDesk::observe</code>:</p><ul>");
        h.push_str("<li>the instrument, tick size, time and timeframe;</li><li>the last bars, oldest first, each with open, high, low, close, the <em>delta</em> (upticks minus downticks inside the bar) and the <em>sweep</em> (largest single move in ticks);</li><li>the net change over those bars;</li><li>the tape of the last 20 seconds: ticks per second, uptick share, largest single move;</li><li>the five-level book either side (sizes), the spread and the real spread in ticks;</li><li>the pit's mood as the crowd simulation reads the tape (buyers lifting, locals widening, and so on);</li><li>the six nearest levels with their signed distance (negative means the level is below price): swings, session extremes, order blocks, fair value gaps, round numbers, OANDA clusters.</li></ul>");
        h.push_str("<p>About 1,900 input tokens a call. Every call is logged in full (request and answer) to <code>jev_relay/log/inferences.jsonl</code>.</p>");
        h.push_str("<h3>The questions</h3><p>Five are the floor's (direction, conviction, continuation, exhaustion, level reaction). The rest belong to crew members: each is one trader's own question, narrow enough to be scored on its own. Those marked in the Traders section as touch questions are only meaningful on a read the desk asked because price had just touched a level; the state then carries a <code>TRIGGER</code> line naming the level and the side price came from.</p><table><tr><th>Question</th><th>Type</th><th>Asked as</th><th>Answers</th></tr>");
        let desk = JevDesk::with_model(Box::new(DeterministicModel));
        let mut qs: Vec<(String, JevQuestion)> = desk.questions().into_iter().collect();
        qs.sort_by(|a, b| a.0.cmp(&b.0));
        for (name, q) in qs {
            let (kind, instr, answers) = match q {
                JevQuestion::Noul { instructions } => (
                    "yes/no probability".to_string(),
                    instructions,
                    "a single probability (0..1)".to_string(),
                ),
                JevQuestion::Choice {
                    instructions,
                    criteria,
                } => {
                    let mut c: Vec<(String, String)> = criteria.into_iter().collect();
                    c.sort();
                    (
                        "choice".to_string(),
                        instructions,
                        c.iter()
                            .map(|(k, v)| format!("<b>{}</b>: {}", esc(k), esc(v)))
                            .collect::<Vec<_>>()
                            .join("<br>"),
                    )
                }
                JevQuestion::Score {
                    instructions,
                    criteria,
                } => (
                    "score 0..2".to_string(),
                    instructions,
                    criteria
                        .iter()
                        .enumerate()
                        .map(|(i, c)| format!("<b>{i}</b>: {}", esc(c)))
                        .collect::<Vec<_>>()
                        .join("<br>"),
                ),
            };
            let _ = write!(
                h,
                "<tr><td><code>{}</code></td><td>{}</td><td>{}</td><td class=\"small\">{}</td></tr>",
                esc(&name),
                kind,
                esc(&instr),
                answers
            );
        }
        h.push_str("</table>");
        h.push_str("<h3>What comes back</h3><p>A read (<code>MarketInterpretation</code>) with: <b>direction</b> (up / down / flat) and its <b>confidence</b> (the probability of the chosen direction); <b>conviction</b> as an expected score 0..2; <b>continuation</b> and <b>exhaustion</b> as probabilities; the <b>level reaction</b> (absorbing / rejecting / sweeping / breaking / no_level) with its confidence. The board line under the chart prints exactly these numbers, then a sentence composed from them (never from Jev's own words).</p>");
        let _ = write!(
            h,
            "<h3>Cadence, freshness, cost</h3><ul><li>The desk asks about each of the nine pits once a minute (pits staggered, so a call every 6 or 7 seconds). The browser asks about the pit you are looking at every {JEV_CADENCE_SECS:.0} s while Jev is on (<kbd>J</kbd>).</li><li>A trader only acts on a read younger than the freshness limit ({READ_FRESH_SECS:.0} s in the browser; the desk's limit is its cadence plus 5 s, so 65 s). Between reads the pass reason says \"no fresh Jev read\".</li><li>Reads are per pit and shared by every trader, so adding a trader costs nothing: about 540 calls an hour at the one-minute cadence, plus the touch reads. A Jev twin is the exception: it makes one call of its own each time its genome enters.</li></ul>"
        );
        h.push_str(r##"<h3>What Jev actually says</h3><p>From 9,515 real reads in the 24 hours to 2026-10-22 17:00 UTC:</p>
<table><tr><th>Field</th><th>Distribution</th></tr>
<tr><td>Direction</td><td>flat 48%, down 34%, up 17%</td></tr>
<tr><td>Direction confidence</td><td>median 0.28, upper quartile 0.41, top decile 0.55 (it is rarely sure)</td></tr>
<tr><td>Conviction (0..2)</td><td>median 0.37, upper quartile 0.62, top decile 0.94</td></tr>
<tr><td>Continuation</td><td>median 0.42, top decile 0.52 (it almost always leans to reversal)</td></tr>
<tr><td>Exhaustion</td><td>median 0.48, top decile 0.56</td></tr>
<tr><td>Level reaction</td><td>absorbing 38%, no_level 28%, sweeping 20%, breaking 8%, rejecting 6%</td></tr>
</table>
<p>This matters for the traders: a rule that asks for conviction above 1.0 or exhaustion above 0.65 is asking for something Jev gives a handful of times a day. The original three Jev traders (Vinnie, Grace, Harold) were written before this was measured, which is why they book so little. The twenty crew members were calibrated against these numbers.</p>
<div class="box red"><b>Is Jev any good?</b> As a trading signal on its own, no. The scorer (<code>cargo run --release --bin jev_score -- --last 24h</code>) on the first 1,105 live reads found a 5-minute direction hit rate of 50% and a Brier score worse than guessing uniformly. By 25 October, on about 28,000 live reads, its direction calls did worse than chance, its win probabilities carried no information, its chart descriptions were a fuzzy copy of what plain code already computes, and two news tests (GDELT tone, Guardian headlines) found no bias that held up across halves of the sample. The desk over the same period: 1,024 trades at -0.11R a trade (t -4.9), 10 of 39 traders positive and none at t 2 or better. What is still open is whether Jev can <em>filter</em> a strategy that has an edge without it. That is the Jev twins' job (<a href="#evolve">section 6</a>).</div>"##);

        // ---- 4. paper -------------------------------------------------------------
        h.push_str("<h2 id=\"paper\">4. How paper trading works</h2>");
        let _ = write!(
            h,
            r##"<p>The paper book (<code>paper.rs</code>) is one piece of code run in two places: in the browser tab, and in the desk around the clock. Each trader is an <em>archetype</em>: a pure rule that looks at a <em>view</em> of one pit (bid, ask, spread, the last 1m bars, the tick flow, the levels, Jev's latest read) and answers either "enter, this way, with these exits" or "pass, because ...". The book runs every trader over every pit on every tick.</p>
<h3>Fills and costs</h3><ul>
<li>A buy fills at the <b>ask</b>, a sell at the <b>bid</b>, so every trade starts behind by the spread, exactly as a real one would. There is no slippage model beyond that and no commission (spread-bet symbols carry their cost in the spread).</li>
<li>Stake is flat: £{STAKE_PER_TICK:.0} per tick in every pit, so pits are comparable.</li>
<li>Results are kept in <b>R</b>: profit divided by the initial stop distance. +1R means the trade made what it risked. The spread paid at entry is also tracked in R so you can see how much of a trader's result the spread ate.</li>
<li>One position per trader at a time, anywhere on the floor; a per-pit cooldown after each trade.</li>
</ul>
<h3>Exits</h3><p>Every entry names its stop and at least one way out: a <b>target</b>, a <b>trail</b> (once a unit onside the stop locks in and follows the best price), and a <b>time stop</b>. Everyone is flattened at {flat} UTC regardless.</p>
<h3>The cost rules</h3><p>The first replay showed the spread, not the ideas, deciding the table. So every trader works under two floors: the initial stop is never less than <b>{stop} spreads</b> (a tighter stop is widened and R is measured on the widened stop), and an entry whose own target is less than <b>{target} spreads</b> away is vetoed. A pass reason like "target 3.0 ticks away is under 4 spreads" is this rule.</p>
<h3>Hours</h3><p>Entries are allowed {from}-{until} UTC. Each original archetype further names the windows its idea belongs to (an opening-range breakout at 19:00 is not the same idea as at 07:20). The crew keep the whole window. Outside its hours a trader's pass reason is "outside session hours".</p>
<h3>The control</h3><p>Colin flips a coin every few minutes inside the union of everyone else's hours, with the same fills, costs and exits. He is not meant to make money; he is the bar. A trader whose edge is real beats Colin after costs over <em>weeks</em>; over a day the difference is noise. The replay reports (<code>replay_out/</code>) compute the proper statistics (t-stat, probabilistic and deflated Sharpe, a bootstrap) and print a verdict per trader.</p>
<h3>The board</h3><ul>
<li>The Traders panel ranks the last 24 hours (rolling, so it reads the same at 03:00 and 15:00). The figure labels and the trader panel's "Today" block count since 00:00 UTC.</li>
<li>Every entry, exit and closed trade is appended to <code>jev_relay/log/paper_trades.jsonl</code> and <code>paper_decisions.jsonl</code> (with <code>"source":"desk"</code> or the tab). The record outlives any browser tab.</li>
<li>The desk posts a heartbeat every 3 s with the board, each trader's open position, last ten trades, day stats, and the reason it is passing in each pit. The browser polls it every 2 s. If the desk dies the tab's own book takes over.</li>
</ul>"##,
            flat = hm(FLAT_AT),
            stop = MIN_STOP_SPREADS,
            target = MIN_TARGET_SPREADS,
            from = hm(ENTRY_FROM),
            until = hm(ENTRY_UNTIL)
        );

        // ---- 5. traders -----------------------------------------------------------
        let _ = write!(
            h,
            "<h2 id=\"traders\">5. The {} hand-written traders</h2>",
            Archetype::ALL.len()
        );
        let _ = write!(
            h,
            "<p>Three groups. The <b>originals</b> (nine): three on Jev, five on market structure with no Jev at all, and the control. Six of the <b>Jev crew</b> trade Jev's common read (Tess takes every lean, Gus fades every lean, Ivo fades the unconvinced ones, Sol trades the flat calls, Lena fades the lean when Jev sees absorption at a level, Kit when it sees a sweep; Lena and Kit were reinstated on 24 October after leading the desk under the 20-spread stops). The other {} are the <b>question traders</b>: each owns one question Jev is asked on every read, and a rule that turns the answer into a side. Vera, Wren, Xi and Yusuf ask about a level price has just touched (what it will have done in fifteen minutes; whether size is absorbing; whether it was a stop run; whether a break would carry) and so trade only on touch reads. Zara asks about the last twenty seconds of tape, Ana and Ben ask the same question about the shape of the last twenty bars and trade different answers, Cy asks which level price reaches first, Dee asks whether the move is thinning out. The scorer grades each question on its own, so a question earns or loses its trader.</p>",
            JEV_CREW.iter().filter(|s| s.ask.is_some()).count()
        );
        h.push_str("<p><b>Only four of these still trade on the desk</b>: Gus, Lena, Tomasz and Colin (the control). The others were taken off on 25 October after the desk's first 1,024 trades showed none of them at t 2 or better; their rows stay here so their past records still read, and their figures are hidden on the floor. Colours below are the jackets on the floor. \"Hours\" are UTC entry windows.</p>");
        h.push_str("<table><tr><th>#</th><th>Trader</th><th>Label</th><th>Jev</th><th>Hours</th><th>Idea</th></tr>");
        for (i, arch) in Archetype::ALL.iter().enumerate() {
            let c = arch.color().to_srgba();
            let rules = arch.rules();
            let _ = write!(
                h,
                "<tr><td>{}</td><td><span class=\"swatch\" style=\"background:rgb({:.0},{:.0},{:.0})\"></span><b>{}</b></td><td>{}</td><td class=\"{}\">{}</td><td class=\"small\">{}</td><td class=\"small\"><i>{}</i><br>{}</td></tr>",
                i + 1,
                c.red * 255.0,
                c.green * 255.0,
                c.blue * 255.0,
                esc(arch.name()),
                esc(arch.label()),
                if arch.uses_jev() { "yes" } else { "no" },
                if arch.uses_jev() { "yes" } else { "no" },
                sessions(rules.sessions),
                esc(arch.describe()),
                esc(&arch.strategy_text())
            );
        }
        h.push_str("</table>");
        let _ = write!(
            h,
            "<p class=\"small dim\">{} crew members. To change one, edit its row in <code>JEV_CREW</code> in <code>opencry-visual/src/paper.rs</code>, rebuild the desk and restart it (<code>./start-floor.sh --restart desk</code>); trunk rebuilds the browser on its own.</p>",
            JEV_CREW.len()
        );

        // ---- 6. evolving pit --------------------------------------------------------
        h.push_str("<h2 id=\"evolve\">6. The evolving pit and the Jev twins</h2>");
        h.push_str(r##"<p>Hand-writing traders and waiting weeks for each verdict is slow. The evolving pit (<code>evolve/</code>, rules in <code>evolve/README.md</code>, written before anything traded) does the search by machine and keeps the forward test honest.</p>
<h3>What a genome is</h3><p>A small rule on 15-minute bars (<code>src/genome.rs</code>): one <b>signal</b> from fourteen families (a breakout, a band touch, an RSI extreme, a run of bars, an opening-range break, yesterday's or last week's high or low, the Asian range, a moving-average cross, a squeeze, a rejection wick, an engulfing bar, a drive off the session open, or the entries of one of edge-lab's intraday strategies, run in edge-lab's own Python on its own 15m, 30m or 1h bars), <b>follow</b> it or <b>fade</b> it, optional <b>filters</b> (trend, volatility, the day's direction), a session <b>window</b>, and an <b>exit</b> (stop in ATR, target in R, a time stop in bars), on a set of symbols. Each gets a generated name with a number (Jax9). The same code runs the backtest and the live desk, so a genome trades exactly what it was tested on.</p>
<h3>The gate</h3><ul>
<li><b>Backtest</b> on edge-lab's 15m history for eight symbols (no Bitcoin, which has no 15m history), at desk costs. In-sample (before 2025-01-01): at least 200 trades, mean at least +0.03R, t at least 2. Holdout (2025 on): at least 60 trades, mean above 0, t at least 1. At least one trade per weekday on average.</li>
<li><b>One seat per idea</b>: two genomes with the same entries and different exits are one idea. At most 40 seats.</li>
<li><b>Cuts</b>, on forward desk trades only: t at or below -2 after 30 trades; mean below 0 after 60; below +0.05R after 150; fewer than 20 trades in 21 days. A cut genome finishes any open trade, then leaves.</li>
<li><b>Graduation</b>: 300 forward trades, mean at least +0.05R, and a deflated Sharpe of 0.95 or better counting every genome ever seated. Graduating means it is put forward for a closer look; nothing here places an order, and what happens next is up to whoever runs the floor.</li>
</ul>
<h3>The twins</h3><p>Every seated genome gets a twin, named with <code>-J</code>. When the genome enters, the twin sends Jev the last 24 bars (no symbol, no dates, prices rebased so the last close is 1000) and asks one question: for a follower, is this move strong and likely to carry on; for a fader, is it tiring and likely to turn. On a yes (0.5 or more) the twin takes the same trade seconds later with the genome's stop, target and remaining time; on a no, it stays out. Every answer is logged (<code>opencry-visual/log/genome_jev.jsonl</code>), so each of the genome's trades is tagged yes or no.</p>
<p>The verdict comes at 300 tagged trades, with checks at 60 and 150. Jev is said to add something only if it said no to at least 15% of trades, the yes trades average at least +0.05R, and the yes-minus-no gap clears the bar after allowing for how many twins have been judged. Twins are never cut on their own record; a twin leaves with its genome.</p>
<h3>Where to see it</h3><p>The Traders panel ends with the pit's summary (genomes tried, seated, retired) and a row per genome and twin, in the genome's colour, with its forward record and how far it is from each cut and from graduation. Each genome takes one of 40 spare bodies on the floor, repainted in its colour, and gives it back when it retires. Clicking a row or a body opens its panel, with the genome's rule in words and, for the pair, the running Jev yes/no split. <code>python3 evolve/evolve.py status</code> prints the same from the command line.</p>"##);
        let _ = write!(
            h,
            "<p class=\"dim small\">As of {}: about 5,000 genomes tried, one seated (Jax9, a New York opening-range follower on US500, NAS100, US30 and UK100) with its twin Jax9-J. The search keeps running every hour; this paragraph is a snapshot, the board is live.</p>",
            today()
        );

        // ---- 7. one trader --------------------------------------------------------
        h.push_str("<h2 id=\"trader-panel\">7. Reading one trader</h2>");
        h.push_str(r##"<p>Click a figure on the floor, or a row in the Traders panel. The panel (bottom left) shows:</p><ul>
<li>the portrait in the trader's colour, name, label and day P&amp;L;</li>
<li>the creed (one line) and the <b>strategy</b> in full: every filter and every exit, in words;</li>
<li>the rules it works under: hours, cost floors, whether it uses Jev;</li>
<li>the <b>outlook</b>: Jev's latest read of the pit the trader is in (or last looked at), and the trader's own take on it, in because / out because;</li>
<li>today's stats since 00:00 UTC;</li>
<li>the <b>open position</b> with entry, stop, target, mark and open R, and why it was taken;</li>
<li><b>Passing now</b>: for every pit, the reason the trader's rule gave for not entering at its last look. This is the line to read when a trader seems idle. "no fresh Jev read" means the next read is up to a minute away; "outside session hours" means the clock;</li>
<li>the last ten closed trades, newest first, with the exit reason and the entry reason.</li>
</ul><p><kbd>Esc</kbd> or the X closes it.</p>"##);

        // ---- 8. pits --------------------------------------------------------------
        h.push_str("<h2 id=\"pits\">8. Why the traders never enter a pit</h2>");
        h.push_str(r##"<p>Because there is nobody in there for them to trade with. The figures inside each pit are the crowd simulation (<code>pit_engine</code>): brokers, locals, runners and loungers generated by the floor and driven by the real tape, so that a burst of upticks looks like buyers lifting and a wide book looks like locals stepping back. They are a way of <em>seeing</em> the tape. They hold no accounts, they quote no prices, and nothing they do changes a price.</p>
<p>The paper traders trade the real market instead: a buy fills at the live ask on the tape, a sell at the live bid, the moment the rule fires. There is no auction to join, no broker to find, no hand signal to make. So walking into the crowd would be a lie about where the fill came from.</p>
<p>What the figures do instead is tell you their state at a glance:</p><ul>
<li>holding a position: they walk to the rail of that pit and <b>step down onto its top step</b>, inside the ring with the crowd, facing the well with their hands up on the side of the trade (green palms long, red short). Several at one pit spread round the step rather than stacking. The label's second line reads LONG or SHORT and the pit;</li>
<li>flat, but recently traded: they go and <b>look at the pit of their last trade</b>;</li>
<li>flat and idle: they <b>wander the concourse</b> between pits.</li>
</ul>
<p>They route with A* on a grid of the concourse (pit interiors are blocked cells, so a walk never cuts through a pit) and take the last few units from the rail down onto the step in a straight line. Stepping in changes nothing about the fill; it is how you see who is in the market right now.</p>
<h3>The sound</h3><p>There is none. The floor had crowd recordings that followed each pit's pace; they were removed on 2026-10-25 because one of them carried clear speech and the sound added little.</p>"##);

        // ---- 9. state -------------------------------------------------------------
        h.push_str("<h2 id=\"state\">9. Where things stand and known gaps</h2>");
        h.push_str(r##"<ul>
<li><b>Jev has shown no edge on its own</b> (section 3). Everything on the floor is built so that this question gets answered with real costs and a control, not so that it looks busy. Whether it helps as a filter is the open question, and the twins answer it.</li>
<li><b>The pit is thin.</b> One genome in about 5,000 has passed the gate. The answer to that is new kinds of idea (other timeframes, edge-lab's intraday strategies as genomes), not a looser gate.</li>
<li><b>Forward evidence is slow.</b> A genome needs 300 desk trades to graduate and a twin 300 tagged trades for a verdict: weeks to months at one trade a day or so. Until then the board shows progress, not proof.</li>
<li><b>Evenings are quiet.</b> Most originals stop entering at 16:30 UTC; the crew and the control stop at 20:30; all are flat by 20:55. Overnight the board shows the previous 24 hours and nothing new.</li>
<li><b>OANDA bands</b> exist only for gold, silver, GBP/USD and EUR/USD, as OANDA publishes books only for those. The bands show where retail resting orders cluster, as a percentage of all orders, and update every 20 minutes. On the synthetic tape the book is synthetic too (round numbers swell); a tape on real data serves one only if it has a source for it.</li>
<li><b>Restarting the tape</b> used to freeze the desk (fixed 2026-10-22). If the Traders panel ever shows the same numbers for many minutes while the pits tick, <code>./start-floor.sh --status</code> and the desk log <code>opencry-visual/log/paper_desk.out</code> are the places to look.</li>
<li><b>Only a desk-side change changes the trading.</b> The browser shows the desk's board; editing a hand-written rule needs the desk rebuilt and restarted (section 5). Genomes come and go without a restart: the desk re-reads <code>pit.json</code> every minute.</li>
</ul>"##);

        // ---- 10. glossary ---------------------------------------------------------
        h.push_str("<h2 id=\"glossary\">10. Glossary</h2>");
        h.push_str(r##"<table>
<tr><td><b>Tick</b></td><td>The smallest price step of the pit (gold 0.01, EUR/USD 0.00001, indices 0.1 or 1).</td></tr>
<tr><td><b>Spread</b></td><td>Ask minus bid, in ticks. Paid on every entry.</td></tr>
<tr><td><b>Unit</b></td><td>The mean high-to-low range of the last 20 one-minute bars, in ticks, floored at 4. The crew size stops and targets in units so a rule means the same thing in a quiet pit and a wild one.</td></tr>
<tr><td><b>R</b></td><td>Profit divided by the initial risk (entry to stop). +1R = made what was risked.</td></tr>
<tr><td><b>Delta</b></td><td>Upticks minus downticks inside a bar, from the tape.</td></tr>
<tr><td><b>Sweep</b></td><td>The largest single move in ticks inside a bar; a stop run shows as a big sweep.</td></tr>
<tr><td><b>Order block, fair value gap</b></td><td>ICT-style structure: the last opposing candle before a strong move, and a three-bar gap the move left behind. Drawn as bands; Priya and Tomasz trade them.</td></tr>
<tr><td><b>Level reaction</b></td><td>Jev's answer on how price behaves at the nearest level: absorbing (orders soak up the push), rejecting (it bounces), sweeping (it pokes through and comes back), breaking (it goes through), or no_level.</td></tr>
<tr><td><b>Conviction</b></td><td>Jev's 0..2 score for how one-sided tape, pit and bars are.</td></tr>
<tr><td><b>Control</b></td><td>Colin: coin flips with the same costs. The bar.</td></tr>
<tr><td><b>Genome</b></td><td>A machine-generated rule from the evolving pit: signal, follow or fade, filters, window, exit, symbols (section 6).</td></tr>
<tr><td><b>Twin</b></td><td>A genome's copy (name ending <code>-J</code>) that takes the genome's entry only when Jev says yes.</td></tr>
<tr><td><b>In-sample, holdout</b></td><td>The backtest history before 2025 (where a genome is found) and from 2025 on (where it must still hold up). Forward means live desk trades after seating.</td></tr>
<tr><td><b>ATR</b></td><td>Average true range of the last bars: the genomes size their stops in it.</td></tr>
<tr><td><b>DSR, PSR</b></td><td>Deflated and probabilistic Sharpe ratio: the replay report's way of saying whether a trader's result survives the number of traders tried and the shortness of the sample.</td></tr>
</table>"##);

        h.push_str("</main></body></html>");
        h
    }
}
