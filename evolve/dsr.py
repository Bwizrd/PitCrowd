"""The referee: the Deflated Sharpe Ratio (Bailey & López de Prado, 2014), as the evolving pit
uses it to decide whether a genome's backtest beats the best of N lucky tries.

    deflated_sharpe(returns, n_tried)  -> probability in [0, 1] that the Sharpe is real
    expected_max_sharpe(n)             -> E[max of n standard normals]: the luck benchmark

This is the same arithmetic as the private research harness the pit was first run against
(so its numbers carry over), in the stdlib only. Note the form: the Sharpe's t-statistic,
SR / se(SR), is compared with the expected maximum of N independent standard normals,
z = SR / se(SR) - E[max_N], with se(SR) from the skew and kurtosis of the returns. It
assumes the N tries are independent, which a genetic search's are not, so it is generous;
evolve/audit/snooping.py measures how generous.
"""
import math
from statistics import NormalDist

_N = NormalDist()
EULER = 0.5772156649


def _mean(x):
    return sum(x) / len(x) if x else 0.0


def _std(x):
    if len(x) < 2:
        return 0.0
    m = _mean(x)
    return math.sqrt(sum((v - m) ** 2 for v in x) / (len(x) - 1))


def sharpe(returns, periods_per_year=252):
    """Annualised Sharpe of per-trade (or per-period) returns. `periods_per_year` must be
    how many of these returns the strategy makes in a year."""
    s = _std(returns)
    if s == 0 or not returns:
        return 0.0
    return (_mean(returns) / s) * math.sqrt(periods_per_year)


def _skew_kurt(returns):
    n = len(returns)
    if n < 4:
        return 0.0, 3.0
    m, s = _mean(returns), _std(returns)
    if s == 0:
        return 0.0, 3.0
    skew = sum(((v - m) / s) ** 3 for v in returns) / n
    kurt = sum(((v - m) / s) ** 4 for v in returns) / n
    return skew, kurt


def expected_max_sharpe(n):
    """E[max of n standard normals]: the Sharpe t-statistic pure luck reaches after n tries."""
    if n <= 1:
        return 0.0
    return (1 - EULER) * _N.inv_cdf(1 - 1.0 / n) + EULER * _N.inv_cdf(1 - 1.0 / (n * math.e))


def deflated_sharpe(returns, n_candidates_tried, periods_per_year=252):
    """Probability that this Sharpe is real rather than the best-of-N fluke; > 0.95 is the
    honest bar. Punishes high win-rate, fat-left-tail return shapes through skew and kurtosis.
    The annualisation cancels: only the per-period Sharpe enters."""
    n = len(returns)
    if n < 20 or n_candidates_tried < 1:
        return 0.0
    sr = sharpe(returns, periods_per_year) / math.sqrt(periods_per_year)
    skew, kurt = _skew_kurt(returns)
    var_sr = (1 - skew * sr + (kurt - 1) / 4 * sr ** 2) / (n - 1)
    if var_sr <= 0:
        return 0.0
    return _N.cdf(sr / math.sqrt(var_sr) - expected_max_sharpe(n_candidates_tried))


# Older name, kept for the audit scripts.
_expected_max_sharpe = expected_max_sharpe


if __name__ == "__main__":
    import random
    rng = random.Random(1)
    noise = [rng.gauss(0, 1) for _ in range(500)]
    edge = [rng.gauss(0.25, 1) for _ in range(500)]
    print("E[max] of 1000 tries:", round(expected_max_sharpe(1000), 3))
    print("noise, 1000 tries:   ", round(deflated_sharpe(noise, 1000), 4))
    print("real edge, 1000 tries:", round(deflated_sharpe(edge, 1000), 4))
