# Changelog

All notable changes to the `common-money` crate (and related monetary infrastructure) will be documented here.

## Unreleased

### Added

- Rounding modes: HalfUp (default), Truncate, Bankers selectable via `MONEY_ROUNDING`.
- Global initialization helpers (`init_rounding_mode_from_env`, `log_rounding_mode_once`).
- Arithmetic trait implementations with enforced post-operation normalization.
- Minor unit helpers: `from_cents`, `as_cents`.
- Nearly-equal comparison helper for tolerance checks.
- Comprehensive test matrix (half-up / truncate / bankers) including negative symmetry and idempotence.
- Property & fuzz tests (midpoint boundaries) using proptest.
- Benchmark suite (`benches/rounding.rs`) for rounding performance.
- Integer cents RFC skeleton (`docs/rfcs/money_integer_cents.md`).
- Aggregate rounding helper (`aggregate_rounding_sum`) with bias illustration test.
- CI Rounding Mode Matrix workflow (`.github/workflows/money-rounding-matrix.yml`).
- Debug tracing instrumentation stub for per-normalization events (metrics placeholder).

### Changed

- Documentation (`docs/financial/money.md`) expanded with runtime initialization, rounding trade-offs, and mode semantics.
- Consolidated roadmap (`docs/financial/money_phase2_roadmap.md`) reflecting completed core Phase 2 primitives.
- Roadmap file removed; content merged into `docs/financial/money.md` (Capabilities & Future Directions) on 2025-10-01.

### Pending / Planned

- Large vector accumulation benchmark & baseline capture.
- Serialization strategy exploration (dual decimal/integer export).
- Public API stability gating.
- Metrics gauge export for active rounding mode.

---

## 0.1.0 (Initial)

- Placeholder initial version (pre-CHANGELOG tracking).
