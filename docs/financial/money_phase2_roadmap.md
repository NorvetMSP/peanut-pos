# Money Roadmap

Persistent, implementation-agnostic plan for monetary handling across services. Consolidates completed work (Phase 1 + early Phase 2) and defers future phases for revisit.

## 1. Current State (As of 2025-10-01)

- `common-money` crate provides:
    - `Money` newtype with enforced scale=2 and normalization.
    - Configurable runtime rounding modes: `HalfUp` (default), `Truncate`, `Bankers` via `MONEY_ROUNDING` env.
    - Global initialization (`init_rounding_mode_from_env`, `log_rounding_mode_once`).
    - Arithmetic traits (Add/Sub/AddAssign/SubAssign/Mul&lt;i32&gt;/Sum) with post-op normalization.
    - Minor unit helpers: `from_cents`, `as_cents`.
    - Nearly-equal comparison helper.
    - Bankers, HalfUp, Truncate deterministic test matrix.
    - Benchmarks scaffold (Criterion) for rounding performance.
    - RFC draft for integer cents representation.
    - Documentation updated: runtime init, trade-offs (incremental vs aggregate rounding), mode semantics.
    - Warning log on invalid `MONEY_ROUNDING` fallback.

## 2. Recently Completed (Since Phase 2 Outline)

| Area | Deliverable |
|------|-------------|
| Rounding Modes | Enum + env parsing + fallback warning |
| Global Init | `OnceLock` + logging helper |
| Tests | Matrix for ties, idempotence, negative symmetry |
| Arithmetic | Trait impls + normalization invariants |
| Minor Units | `from_cents`, `as_cents` canonical semantics |
| Docs | Expanded `money.md` + trade-offs section |
| Benchmarks | `benches/rounding.rs` added |
| RFC | `docs/rfcs/money_integer_cents.md` skeleton |
| Tooling | Lint fixes, environment logging, invalid env warnings |
| Integration Gating | Feature flag for integration tests (services) |
| CI | Rounding mode test matrix workflow (`money-rounding-matrix.yml`) |
| Testing | Property + fuzz midpoint tests (proptest) |
| Aggregation | `aggregate_rounding_sum` helper + bias example tests |
| Instrumentation | Debug tracing stub per normalization (metrics placeholder) |
| Metrics | Optional Prometheus gauge (feature `prometheus-metrics`) |
| Benchmarks | Accumulation benchmark (incremental vs aggregate vs integer-cents simulation) + baseline doc |
| Docs | Penny balancing & reconciliation patterns added to `money.md` |
| RFC | Serialization strategy section appended |

## 3. Backlog (Deferred / Not Started)

Priority Legend: (H) High – valuable next; (M) Medium – opportunistic; (L) Low / future.

1. (H) Public API Stability Check
   - Introduce `cargo public-api` (or similar) gating changes before Phase 3.
2. (M) Per-Currency Scale Exploration
   - RFC stub for variable scale (e.g., JPY scale=0, crypto >2). Defer until requirement appears.
3. (M) Penny Balancing Utility
    - Function to distribute a 1–2 cent discrepancy across line items for aggregate rounding mode.
4. (L) Unsafe / Determinism Audit Note
    - `SAFETY.md` confirmation of no unsafe usage & determinism guarantees.
5. (L) Feature Flag: `integer-cents` Implementation
    - Transition internal representation to `i64` cents behind opt-in.
6. (L) Differential Testing Harness
    - Compare BigDecimal vs integer-cents outputs across 100k random cases.

## 4. Acceptance Criteria (Backlog Completion Definition)

- CI matrix validates all rounding modes each PR touching `common-money` (DONE).
- Aggregate rounding helper present with docs & tests (DONE initial version; docs expansion pending).
- Fuzz/property tests produce no failures across 50k randomized midpoint cases (DONE initial; may expand sample size).
- Benchmarks recorded (baseline then tracked) and integer-cents RFC updated with measured deltas.
- CHANGELOG entry summarizing rounding mode introduction & trade-offs.
- Optional metrics export integrated (non-breaking if metrics stack absent).

## 5. Risks & Mitigations

| Risk | Impact | Mitigation |
|------|--------|------------|
| Silent cumulative bias | Minor cent drift on large item sets | Provide aggregate helper & doc criteria |
| Integer-cents migration complexity | Potential API churn | Feature gate + differential tests first |
| Mode regression unnoticed | Production rounding divergence | CI matrix + property tests |
| Performance regression in rounding | Latency in price-heavy endpoints | Criterion benchmark gate (compare median vs baseline) |

## 6. Open Questions

1. Do we guarantee stable string formatting (e.g., always two decimals) across all serialization layers? (Current: yes via normalization.)
2. Should integer-cent export be opt-in per endpoint vs global config? (Leaning per-endpoint.)
3. Do we need an optional audit log for each rounding adjustment? (Probably only at debug log level if enabled.)

## 7. Parking Lot (Ideas, Not Yet Prioritized)

- Currency type abstraction & FX conversion pipeline.
- Tax engine integration hooks (pre-total vs post-discount application order invariants).
- Locale-aware formatting module (display only; separate from value model).
- Streaming accumulation API for large batch imports.

## 8. How to Resume

When picking this back up:

1. Add public API stability tooling (`cargo public-api` or similar) and gate CI.
2. Design penny balancing utility function with deterministic selection policy.
3. Prototype integer-cents internal feature behind `integer-cents` and differential tests.
4. Implement dual serializer feature (`money-serde-cents`).
5. Add performance regression guard comparing new integer-cents path vs baseline.

## 9. Reference Artifacts

- Code: `services/common/money/src/lib.rs`
- Benchmarks: `services/common/money/benches/rounding.rs`
- RFC: `docs/rfcs/money_integer_cents.md`
- Docs: `docs/financial/money.md`
- Integration gating workflow: `.github/workflows/integration-tests.yml`

## 10. Status

Phase 1 complete; core Phase 2 primitives plus CI matrix, fuzz + property tests, aggregation helper, and instrumentation implemented. Remaining backlog shifts to CHANGELOG/documentation, performance benchmarking depth, and representation evolution.

Maintainer: TBD
Next Recommended Action: Draft CHANGELOG and add large vector accumulation benchmark.
