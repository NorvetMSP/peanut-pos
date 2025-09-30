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

Planned enhancements:

- Configurable rounding modes: `HalfUp`, `Truncate`, `Bankers` (Round Half to Even).
- Selection via:
  - Environment variable `MONEY_ROUNDING=half-up|truncate|bankers`
  - Optional feature flags for compile-time restriction (e.g., `--features bankers_rounding`).
- Expose `set_rounding_mode()` (gated for tests / startup only) to ensure deterministic initialization.
- Add audit logging when non-default rounding mode is used.

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

## Open Questions (Phase 2)

- Do we need per-currency scale? (Out of scope for MVP; all amounts assumed USD 2 decimal places.)
- Should we prevent arithmetic directly on `Money` until we define invariants? (Potential to add `Add`, `Sub`, etc., with normalization.)
- Do we log when a value required rounding vs already normalized? (Could aid auditability.)

---

Maintainer: TBD
Status: Phase 1 in progress (Money type introduced; integration pending).
