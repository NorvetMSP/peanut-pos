# Money Newtype & Rounding Policy

## Overview

The `common-money` crate introduces a `Money` newtype wrapper around `BigDecimal` to centralize:

- Scale enforcement (2 decimal places for cents)
- Rounding policy (currently Half-Up)
- Future configurability (feature flag or environment-driven rounding modes)
- Comparison helpers (`nearly_equal`) and construction utilities (`from_major_minor`)

## Current Rounding Mode

We use Half-Up rounding to two decimal places:

- Values with a third decimal digit >= 5 round away from zero.
- Symmetric for negative numbers (e.g., `-1.235` -> `-1.24`).
- Examples:
  - `12.345` -> `12.35`
  - `12.344` -> `12.34`
  - `-2.505` -> `-2.51`
  - `-2.504` -> `-2.50`

Rationale:

- Matches typical POS domain expectations for receipt totals.
- Avoids bias introduced by truncation.
- Keeps implementation small and predictable while deferring more complex banker's rounding until a real requirement emerges.

## Money API (Phase 1)

```rust
pub struct Money(BigDecimal);
impl Money {
  pub fn new(raw: BigDecimal) -> Self;            // normalizes & rounds to scale 2
  pub fn from_major_minor(major: i64, minor: i64) -> Self; // helper: major dollars + minor cents
  pub fn inner(&self) -> &BigDecimal;             // access underlying value
}
impl From<BigDecimal> for Money
impl From<Money> for BigDecimal
```

`normalize_scale(&BigDecimal)` provides the same rounding normalization and is used internally.

## Comparison Helper

`nearly_equal(a, b, cents_tolerance)` normalizes both sides and compares the absolute difference in whole cents against a provided tolerance. This helps with defensive comparisons around computed totals.

## Integration Plan (Phase 1)

1. Replace raw `BigDecimal` fields used for prices/amounts with `Money` in product, order, and payment services.
2. Convert DB layer bindings to use `money.inner()` when passing parameters into SQLx macros.
3. Update serialization: either derive `Serialize/Deserialize` directly (already implemented) or implement custom serde if we later add metadata.
4. Adjust tests expecting legacy truncation (e.g., price `19.999` now rounds to `20.00`).

## Future (Phase 2) - Feature Flag / Runtime Policy

Implemented / In Progress:

- Configurable rounding modes available now: `HalfUp` (default), `Truncate`, `Bankers` (round half to even).
- Selected at runtime via environment variable `MONEY_ROUNDING=half-up|truncate|bankers`.
- Initialization helper: `init_rounding_mode_from_env()` (idempotent) + `log_rounding_mode_once()` for a single structured log line.
- Test-only helper: `set_rounding_mode_for_tests(mode)` (only effective before first global init).
- Audit logging: one-time log emitted when a service calls `log_rounding_mode_once()` (recommended early in `main`).

Example service startup snippet:

```rust
fn main() -> anyhow::Result<()> {
    // Initialize logging first so rounding mode announcement is visible
    tracing_subscriber::fmt::init();

    // Set rounding mode (if MONEY_ROUNDING unset or invalid defaults to HalfUp)
    common_money::log_rounding_mode_once();

    // ... remainder of service initialization
    Ok(())
}
```

Environment examples:

```bash
MONEY_ROUNDING=truncate ./services/order-service
MONEY_ROUNDING=bankers  ./services/payment-service
```


Invalid values fallback silently to HalfUp (with a warning planned for a later enhancement—currently silent to avoid noisy prod logs if unset).

Data model impact: None (still stored as numeric/decimal in DB). All rounding applied at service boundary or persistence points.

## Edge Cases Considered

- Negative half-up rounding symmetry (`-1.235` -> `-1.24`).
- Very large values: scaling avoids integer overflow by using string truncation post-adjustment.
- Construction from mixed sign major/minor (e.g., `from_major_minor(-10, 5)` -> `-9.95`).

## Testing Strategy

- Unit tests in `common-money` validate rounding positive/negative edge cases, large values, and constructor logic.
- Service round-trip tests will be updated after integration to assert post-rounding storage & JSON behavior.

## Migration Notes

Legacy behavior used truncation (`with_scale(2)`), which would produce `19.999` -> `19.99`. After migration to `Money`, the result becomes `20.00` (half-up). Update any snapshot or golden test data accordingly.

## Incremental vs Aggregate Rounding (Trade-offs)

When performing multi-line item computations (e.g., order totals), there are two primary strategies:

### 1. Incremental (current approach)

- Round each line item (or intermediate) immediately to scale 2.
- Pros: Predictable; each displayed line matches persisted value; no hidden fractions.
- Cons: Bias can accumulate over large sets (systematic half-up drift of fractions just over .005).

### 2. Aggregate (deferred rounding)

- Maintain higher precision (e.g., scale 4–6 or integer cents before rounding) for intermediate sums; round once at the end.
- Pros: Minimizes cumulative bias; can materially differ on large bulk operations (e.g., promotions + tax recompute).
- Cons: Line-level display may not match an internally recomputed grand total unless extra reconciling cents adjustments are made.

### Why incremental now

- Simplicity and alignment with receipt expectations where each line stands alone.
- Low risk of material bias in typical POS transaction sizes (dozens, not thousands, of lines).
- Reduced reconciliation complexity—no need for "penny balancing" across lines.

### When to reconsider

- Evidence of cumulative rounding discrepancies impacting financial audits.
- Introduction of tax or discount engines requiring fractionally precise intermediate bases.
- A compliance requirement specifying aggregate rounding semantics.

### If aggregate mode is introduced

- Add a `MONEY_AGGREGATE_ROUND=true` env switch (or feature flag) gating alternative accumulation helpers.
- Provide utility: `aggregate_round<I: IntoIterator<Item=Money>>(iter) -> Money` applying high-precision sum then rounding via current mode.
- Document reconciliation rule for distributing a 1‑cent adjustment if displayed line item sum differs from aggregate by a cent.

### Recommendation

- Use incremental rounding everywhere for consistency.
- For analytics or end-of-day settlement exports, if higher precision becomes necessary, derive values from raw source events rather than re-summing already-rounded Money instances.

## Penny Balancing & Reconciliation Patterns

When aggregate rounding (round-once) differs from the sum of individually rounded line items, the discrepancy is typically 1–2 cents. Strategies:

1. Silent Absorption: Attribute the cent difference to the merchant margin (acceptable for non-regulated contexts).
2. Distributed Adjustment: Add 1 cent to the highest-value (or earliest) eligible line item until totals match.
3. Dedicated Adjustment Line: Insert a synthetic line item labeled "Rounding Adjustment" (+/-0.01) for explicit auditability.

Suggested algorithm (distributed):

1. Compute incremental_total = sum(rounded lines)
2. Compute aggregate_total = round(sum(unrounded lines))
3. diff_cents = aggregate_total.cents - incremental_total.cents (can be -2..2, usually -1..1)
4. If diff_cents != 0:

- Select candidate lines (exclude tax-only or zero-value) sorted by absolute line value desc then stable index.
- Apply +1 or -1 cent adjustments to first |diff_cents| candidates.

This keeps the displayed line items consistent with the final aggregate while distributing bias predictably.

Auditing Tip: Log (incremental_total, aggregate_total, diff_cents, adjustment_line_ids) at debug level when non-zero.

Future Utility (backlog): `reconcile_pennies(lines: &mut [Money], target: Money)` returning applied delta count for observability.

## Open Questions (Phase 2)

- Do we need per-currency scale? (Out of scope for MVP; all amounts assumed USD 2 decimal places.)
- Should we prevent arithmetic directly on `Money` until we define invariants? (Potential to add `Add`, `Sub`, etc., with normalization.)
- Do we log when a value required rounding vs already normalized? (Could aid auditability.)

---

Maintainer: TBD
Status: Phase 1 in progress (Money type introduced; integration pending).

## Capabilities (Consolidated)

The crate currently provides:

- Enforced scale=2 normalization on construction & arithmetic
- Runtime-configurable rounding modes: HalfUp (default), Truncate, Bankers via `MONEY_ROUNDING`
- One-time initialization & structured logging (`log_rounding_mode_once`)
- Arithmetic traits (Add/Sub/AddAssign/SubAssign/Mul&lt;i32&gt;/Sum) with post-op normalization
- Minor unit helpers: `from_cents`, `as_cents`
- Nearly-equal comparison helper (`nearly_equal`)
- Deterministic test matrix across rounding modes (ties, negatives, idempotence)
- Criterion benchmarks (rounding + accumulation)
- Aggregate rounding helper (`aggregate_rounding_sum`) and accumulation baseline doc
- Property & fuzz midpoint tests (proptest)
- Optional Prometheus gauge (`money_rounding_mode`) behind `prometheus-metrics` feature
- Integer cents RFC + serialization strategy section
- Penny balancing guidance & reconciliation patterns

## Risks & Mitigations (Snapshot)

| Risk | Impact | Mitigation |
|------|--------|-----------|
| Silent cumulative bias | Minor cent drift on large item sets | Aggregate helper + penny balancing guidance |
| Integer-cents migration complexity | API churn / divergence | Feature gate + differential tests before switch |
| Mode regression | Incorrect rounding in prod | CI matrix + property/fuzz tests |
| Performance regression | Slower high-volume totals | Benchmarks + future regression guard |
| Serialization divergence | Client confusion | Document encoding modes; dual-field strict validation |

## Future Directions

Planned / exploratory items now tracked as issues instead of roadmap doc:

1. Public API stability gating (`cargo public-api` integration)
2. Integer-cents internal representation feature (`integer-cents`) + differential test harness
3. Dual serializer feature (`money-serde-cents`) for `{ "amount": "12.34", "cents": 1234 }`
4. Penny balancing utility function (`reconcile_pennies`) with deterministic policy
5. Performance regression alerting (compare benches against stored baseline)
6. Per-currency scale RFC (deferred until requirement emerges)
7. SAFETY / determinism audit and `SAFETY.md` note

## Historical Roadmap Consolidation

The standalone `money_phase2_roadmap.md` document has been retired; its content was merged here (2025-10-01). Strategic evolution now lives in:

- This capabilities & future directions section
- `CHANGELOG.md` (history)
- `docs/rfcs/money_integer_cents.md` (representation evolution)
- `docs/financial/benchmarks/money_rounding_baseline.md` (performance baseline)

For long-term planning, create or update GitHub Issues labeled `money` / `performance` / `serialization`.
