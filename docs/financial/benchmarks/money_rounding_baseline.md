# Money Accumulation Benchmark Baseline

Date: 2025-10-01
Target: `common-money` (BigDecimal internal representation)
Rounding Mode: Default (HalfUp)
Sample Size: 20 (criterion adjusted for speed)

## Scenarios

Three accumulation strategies over synthetic patterned monetary values:

- incremental: construct `Money` for each element (round per-item) and sum (current production pattern)
- aggregate: sum raw `BigDecimal` values then round once via `aggregate_rounding_sum`
- integer_cents_sim: simulate integer-cents accumulation (convert each to Money to normalize, sum cents)

## Results (µs unless noted)

| Size | incremental | aggregate | integer_cents_sim |
|------|-------------|-----------|-------------------|
| 100  | ~265 µs     | ~2.96 µs  | ~190 µs           |
| 1k   | ~2.51 ms    | ~16.95 µs | ~1.92 ms          |
| 10k  | ~26.0 ms    | ~162 µs   | ~19.6 ms          |

## Observations

- Aggregate rounding (single normalization) is ~80-90x faster than incremental for large vectors because it performs one rounding pass instead of N.
- Simulated integer-cents accumulation outperforms incremental but still incurs per-item normalization cost due to current simulation approach (normalizes each element). A true integer-cents internal implementation should approach or beat aggregate performance while preserving per-item semantics.
- Incremental vs simulated integer cents gap (26.0 ms vs 19.6 ms at 10k) suggests potential ~25% improvement without deeper optimization.

## Next Steps

1. Implement real `integer-cents` feature and re-run benchmarks.
2. Add variance tracking (store previous best and alert if regression >10%).
3. Explore hybrid approach: keep per-line incremental rounding for display but accumulate unrounded fractions in parallel for final total & reconciliation.

## Raw Criterion Output Reference

See benchmark run in CI artifacts or reproduce locally:

```bash
cargo bench -p common-money --bench accumulation -- --sample-size 20
```
