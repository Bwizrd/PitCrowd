# Web search: trading strategies that use Jev

Asked 2026-10-25: *search for successful trading strategies that use Jev, the AI tool from typesafe.ai.*

## Answer
No source shows a trading strategy using Jev that has been proven profitable. Jev (TypeSafe's "System One"
model, released 2026-09-15) had dozens of trading bots built on it within weeks [1][9][13]. The only
independent check, a review of 19 finance projects, found no one making money; in all four projects that
compared Jev with a simpler alternative, Jev did not come out ahead [18]. The careful builders share one
design: **"Jev judges, code executes"**, with Jev as a cheap, fast filter or veto on a strategy that already
exists, not the source of the edge [1][2][7].

## Key facts
- **What Jev is.** A model that does not generate text: you send a state (text or JSON) and typed questions
  (Choice, Score, or Noul true/false) and get back probabilities and a confidence score. Version jev-1.13 [2][9].
- **Speed and price.** Vendor quotes 70–500 ms and $0.042 per million input tokens, output free [2][5].
  TypeSafe says its own benchmarks are likely at the high end of real results [2][9].
- **Known weaknesses for trading.** Not a calculator, does not count reliably, reads dates as text, works
  better with labels than raw numbers, cannot read chart images, knows nothing of the current market [2].
  One tester found backtests improved with timestamps removed and only one or two indicators sent [13].
- **Common architecture.** Code computes features, compresses them into a small labelled state, asks narrow
  questions, applies thresholds, and hard risk rules veto. No serious project lets Jev place orders [1][2][7][19].
- **Strategy types seen:** pre-trade gate (QuantDinger; on timeout the order goes ahead) [1][7][18];
  on-chain market making (jarrodwatts/jev-trader, public version is a dry run) [12][18]; multi-question policy
  with Brier/calibration logging (buberlo/jev-trader, synthetic data only) [1][7][18]; auditor of another
  model's trade thesis (not yet switched on; the author's prop account hit max drawdown) [6];
  confidence-gated signals [8][11][14].
- **Measured results, all from projects' own logs:** Hong Kong next-day direction 45% right (54/120) against
  51.7% for always predicting down [2][7][18]; 18 SOL scalping experiments all unprofitable, a simple rule
  matched Jev's best [18]; simple rules matched or beat Jev in 40 of 47 finance demos, price calls 42–52% [18];
  the one positive claim (profit factor 3.53 vs 1.74) has no supporting data in its repo [18].
- **Where Jev does well:** classification, not prediction: 92.4% on banking-intent classification against
  93.66% for fine-tuned BERT [1][18]; 100% on one tax-form corpus [1].

## Disagreements and gaps
- Social-media wins ($148 to $276,000 on Polymarket; 82% testnet win rate) come with no verifiable data [13].
- The cTrader store listing is dated two months before Jev's public release [8][9].
- Source [19] is promotional.
- The audit mentions a follow-up on 181,290 Bitcoin candles testing QuantDinger's gate that was not fetched [18].
- Jev's calibration has not been independently shown [4]; the headline ~68% accuracy is agreement with
  frontier models, not ground truth [17].
- jev-ai.pro is a third-party site, not TypeSafe [10]; TypeSafe has no official trading bot [7].

## Sources
1. Jev (TypeSafe System One) finance & trading projects, gist, 2026-09-21: https://gist.github.com/drillan/6916b16e8ea31a8ec36c8f59d6483150
2. Jev AI for Trading: what it can and cannot do, Apex Logic, 2026-09-26: https://apex-logic.net/news/jev-ai-model-for-trading-2026
3. TypeSafe AI home: https://typesafe.ai/
4. TypeSafe Jev Review, Kingy AI, 2026-09-15: https://kingy.ai/blog/typesafe-jev-review-the-ai-model-that-doesnt-generate-text
5. Introducing System One Models & Jev, TypeSafe blog: https://typesafe.ai/blog/introducing-system-one-models-and-jev
6. JEV assisted LLM Trading, DEV, 2026-09-20: https://dev.to/nodefiend/jev-assisted-llm-trading-ofa
7. Jev Trading Bots on GitHub: 5 projects, Apex Logic, 2026-09-26: https://apex-logic.net/news/jev-trading-bots-github-2026
8. Jev Trend bot, cTrader store: https://ctrader.com/products/5533
9. Jev (AI model), Wikipedia: https://en.wikipedia.org/wiki/Jev_(AI_model)
10. jev-ai.pro, Scamadviser: https://scamadviser.com/check-website/jev-ai.pro
11. zadescoxp/Jev-Trades: https://github.com/zadescoxp/Jev-Trades
12. jarrodwatts/jev-trader: https://github.com/jarrodwatts/jev-trader
13. 26 Jev trading builds, madewithjev.com: https://madewithjev.com/categories/trading-and-markets
14. Jev-trading/Jev-trading: https://github.com/Jev-trading/Jev-trading
15. Jev for trading bots, Jev Builds: https://jev.openchamber.dev/use-cases/trading-bots
16. TypeSafe AI Jev Review, actionbox: https://actionbox.cloud/blog/typesafe-ai-jev-review
17. What everyone is getting wrong about Jev, KDnuggets: https://kdnuggets.com/what-everyone-is-getting-wrong-about-typesafe-ais-jev
18. Jev trading bots: I checked 19 projects for real results, DEV, 2026-09-30: https://dev.to/moonthetrain/jev-trading-bots-i-checked-19-projects-for-real-results-odb
19. How to Use Jev to Build a 24/7 HFT Trading System, X: https://x.com/i/article/2101311813908652069
20. Building a harness with Jev, LangChain: https://langchain.com/blog/building-a-harness-with-jev
21. How to use Jev AI, PickMyTrade: https://blog.pickmytrade.trade/how-to-use-jev-ai
22. Opus 5.5 + Jev, X: https://x.com/milesdeutscher/article/2104937986869567565
23. Backtest a Jev strategy in Python, Apex Logic: https://apex-logic.net/news/backtest-jev-trading-strategy-python-2026
24. TypeSafe docs: https://docs.typesafe.ai/introduction
25. 24/7 AI trading system with Jev, raycfu: https://raycfu.com/guides/jev-ai-trading-system
