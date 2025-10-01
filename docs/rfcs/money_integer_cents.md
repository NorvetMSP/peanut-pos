# RFC: Money Representation Using Integer Cents

Status: Draft
Author: TBD
Target Version: Phase 3 (Post Phase 2 Rounding Enhancements)
Last Updated: 2025-10-01

## 1. Summary

Switch (optionally) from `BigDecimal`-backed `Money` to an internal integer minor-unit representation (i64 cents) to improve performance, reduce allocation, and simplify arithmetic while preserving external decimal semantics.

## 2. Motivation

- Performance: Frequent normalization & string-based rounding impose overhead.
- Predictability: Integer arithmetic removes ambiguity around decimal scale handling.
- Interop: Many payment APIs and tax engines operate in integer minor units.
- Memory: Avoids heap allocations for many `BigDecimal` operations.

## 3. Goals

- Preserve public API semantics (`Money` remains externally decimal-friendly).
- Avoid precision loss for scale=2 amounts.
- Backward-compatible database persistence (still store as DECIMAL(precision,2) or NUMERIC(2)).
- Allow incremental opt-in (feature flag `integer-cents`).

## 4. Non-Goals

- Multi-currency scale polymorphism (deferred).
- Arbitrary precision beyond two fractional digits.
- FX conversion logic (future concern).

## 5. Current State

`Money(BigDecimal)` with rounding applied on construction and arithmetic normalization. Rounding modes: HalfUp, Truncate, Bankers.

## 6. Proposed Design

### 6.1 Internal Representation


```rust
struct Money { cents: i64 }
```

Invariants: `cents` always represents scale=2. Conversion to `BigDecimal` only when required for serialization or interop.


### 6.2 Construction

- From decimal string: parse, scale *100, apply rounding mode, store integer.
- From major/minor: `major * 100 + sign(minor) * abs(minor)`.
- From existing BigDecimal: reuse current rounding logic; internal fast path if already scale=2.

### 6.3 Arithmetic

All `Add/Sub/Mul<i32>/Sum` operate directly on `i64` with overflow checks (`checked_add`). Overflow → error or panic (TBD policy). Potential alternative: saturating operations under feature flag.

### 6.4 Rounding Modes

Rounding implemented on scaling during parse / conversion. Bankers mode implemented via integer midpoint detection: value * 10^extra % 10 == 5 plus remainder zero logic.

### 6.5 Serialization

- JSON: Emit decimal string (e.g., "12.34").
- DB Binding: Convert to `BigDecimal` lazily for sqlx encode.

### 6.6 Feature Flag Strategy

`integer-cents` gate new implementation. Fallback to legacy BigDecimal when flag not enabled.

## 7. Alternatives Considered

1. Dual representation enum (either BigDecimal or i64) – added complexity.
2. Always store as string – inefficient for arithmetic.
3. Rely on database numeric coercion – pushes correctness out of application layer.

## 8. Performance Considerations

Benchmarks to compare:

- Construction throughput (string -> Money)
- Summation of large vectors
- Rounding mode overhead (existing vs integer path)
Target: >=2x improvement in construction & bulk arithmetic vs BigDecimal baseline.


## 9. Migration Plan

1. Introduce feature flag & parallel implementation.
2. Run dual benchmarks in CI (matrix: legacy vs integer).
3. Audit for any reliance on BigDecimal-specific APIs.
4. Flip default after confidence period (announce in CHANGELOG).

## 10. Risks

- Overflow edge cases for extremely large amounts (mitigate with bounds check + documented limits).
- Divergence in rounding semantics if integer midpoints mis-implemented.
- Accidental mixing of Money representations across feature boundary in multi-crate builds.

## 11. Open Questions

- Should we expose raw `cents()` accessor publicly? (Leans yes for performance-critical paths.)
- Do we support serializing both decimal and integer forms via a serde feature switch?
- Maximum safe absolute value? (e.g., reserve headroom below i64::MAX/100.)

## 12. Test Strategy

- Property tests: decimal <-> cents round trip.
- Fuzz midpoints around .0045–.0055 intervals.
- Differential tests: compare legacy BigDecimal vs integer-cents outputs over random dataset.

## 13. Observability

- Counter/log when fallback to legacy path is triggered (if hybrid mode used).
- Benchmark HTML reports stored as artifacts.

## 14. Appendix: Sample Implementation Sketch
 
```rust
#[cfg(feature = "integer-cents")]
impl Money {
    pub fn from_str_decimal(s: &str) -> Result<Self, ParseMoneyError> { /* parse, round, scale */ }
    pub fn as_cents(&self) -> i64 { self.cents }
}
```

---
End of RFC.
\n## 15. Serialization Strategy Exploration (Decimal vs Integer)

We may expose alternative JSON encodings driven by API consumer requirements or performance considerations.

### 15.1 Options

| Strategy | JSON Example | Pros | Cons |
|----------|--------------|------|------|
| Decimal String (current) | "12.34" | Human-readable, backward compatible | Parse cost on clients needing integers |
| Integer Cents (alt) | 1234 | Fast for machine use, no parse rounding | Less readable; risk of unit confusion |
| Object Wrapper | {"amount":"12.34","cents":1234} | Dual form, explicit units | Larger payloads |
| Tagged Variant | {"decimal":"12.34"} or {"cents":1234} | Flexible negotiation | Complex client logic |

### 15.2 Proposed Path

Phase 1 keep decimal string only. Introduce optional serde feature `money-serde-cents` enabling dual-field object output for services where integer-centric downstream processing is critical.

Example (dual mode):

```json
{"amount":"12.34","cents":1234}
```

Round-tripping: On deserialize, prefer integer cents if present (authoritative), fall back to parsing decimal string.

### 15.3 Compatibility & Versioning

- Document encoding mode in service OpenAPI specs.
- Add CHANGELOG entries when enabling new serde features.
- Avoid silent switches: require explicit feature flag in crates depending on `common-money`.

### 15.4 Security & Validation

- Enforce that `cents` and `amount` (if both present) reconcile; reject mismatch.
- Limit absolute cents value to a configured maximum to mitigate overflow or abuse (e.g., > 10^13 cents reject).

### 15.5 Next Steps

1. Implement feature-gated dual serializer.
2. Add differential tests ensuring both encodings round-trip equivalently to internal cents.
3. Provide migration guide section in README & money.md.
