# Money Phase 2 Roadmap

Persistent reference for configurable rounding + monetary ergonomics. This mirrors the tracked tasks so it survives chat resets.

## Scope Overview

Enhance `common-money` with configurable rounding, improved arithmetic ergonomics, CI enforcement, and strategic forward-looking artifacts (RFC + benchmarks).

## Task List

1. Config rounding enum expansion
   - Add variants: `HalfUp` (default), `Truncate`, `Bankers` (round half to even).
   - Implement `FromStr` + env parsing (`MONEY_ROUNDING`).
   - Fallback to HalfUp with warning if invalid value.
2. Global rounding init
   - `OnceLock<RoundingMode>`; `init_rounding_mode_from_env()` called in each service `main`.
   - Test-only setter `set_rounding_mode_for_tests()` under `cfg(test)`.
3. Rounding test matrix
   - Table-driven tests covering positive/negative ties: 1.005, 2.675, 0.005, -1.005, -2.505, large integers, already-normalized values.
   - Assert: idempotence and negative symmetry.
4. Logging & metrics
   - Log chosen mode once at startup.
   - Optional: expose mode label via metrics (if metrics infra exists later).
5. Money arithmetic traits
   - Implement `Add`, `Sub`, `AddAssign`, `SubAssign`, `Mul<i32>`, `Sum<Money>`.
   - Normalize after each op; document potential cumulative rounding vs aggregate rounding strategy.
6. Minor units helpers
   - `from_cents(i64)`, `as_cents(&self) -> i64`.
   - Clarify behavior for negatives (two's complement vs mathematical).
7. Code cleanup
   - Remove unused imports (payment-service `normalize_scale`).
   - Annotate intentionally unused fields with `#[allow(dead_code)]` or refactor.
8. Docs update Phase 2
   - Expand `money.md`: env variable usage, example initialization snippet, per-mode examples, rounding comparison table, arithmetic trait examples, gotchas (unit vs aggregate rounding).
9. CI enhancements
   - Matrix job: run `MONEY_ROUNDING` = `half-up|truncate|bankers` for `common-money` tests.
   - Add `sqlx prepare` diff guard (fail if uncommitted change).
10. Integration test gating
    - Feature flag ignored DB-dependent tests (e.g., `integration` cargo feature) or provide docker-compose ephemeral services.
11. Changelog entry
    - Add/Update `CHANGELOG.md` summarizing Phase 1 (Money + HalfUp) and planned Phase 2 release.
12. Performance benchmark
    - Criterion benches: rounding overhead vs naive BigDecimal `.with_scale()` truncation; arithmetic accumulation.
13. Integer cents RFC
    - `docs/rfcs/money_integer_cents.md`: Compare BigDecimal vs i64 minor units (precision, perf, migration complexity).
14. Cross-service drift audit
    - Simple test crate or script verifying identical `normalize_scale` output across services (or just rely on shared crate; document rationale).

## Acceptance Criteria

- MONEY_ROUNDING env variable switches behavior & all mode tests pass.
- Startup log includes selected rounding mode.
- All arithmetic trait implementations are covered by tests & maintain normalization invariants.
- Documentation updated and discoverable.
- CI matrix validates each rounding mode.

## Open Questions

- Should per-currency scaling (e.g., JPY scale=0) be deferred until explicit requirement? (Current answer: yes, defer.)
- Do we enforce aggregate rounding (sum raw, round once) vs incremental? (Current: incremental; document trade-off.)
- Do we need a serialization flag to export integer cents for external APIs? (Potential Phase 3.)

## Timeline Suggestion (If Sequenced)

Week 1: Tasks 1–4.
Week 2: Tasks 5–8.
Week 3: Tasks 9–11.
Week 4: Tasks 12–14 + RFC review.

Maintainer: TBD
Status: Planned (no implementation yet for Phase 2 tasks at time of creation).
