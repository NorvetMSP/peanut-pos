# Tenancy & Security Middleware Unification Plan (Draft)

Goal: Eliminate duplicated per-service tenancy extraction, role enforcement scatter, and uneven audit coverage by introducing a shared `common-tenancy` (or extend `common_auth`) crate that provides a consistent pipeline: Request -> (AuthN, Tenant Resolve, RBAC Check, Audit Context) -> Handler.

## Current Pain Points

- Repeated calls to `tenant_id_from_request(&headers, &auth)` across many handlers.
- Role arrays duplicated (ADMIN/MANAGER/CASHIER sets) leading to drift.
- Audit not consistently emitted (some services plan for it, others silent).
- No single place to plug future policy gating (e.g., feature flags by tenant, subscription tier, data residency constraints).

## Objectives

1. Centralize tenant resolution & validation.
2. Provide declarative RBAC guard macros (attribute-like) or lightweight function wrappers.
3. Inject an `ExecutionContext` (tenant_id, actor_id, roles, correlation_id) into handlers.
4. Standardize audit event emission shape & convenience macros.
5. Make extension for multi-tenant rate limiting / quota feasible.

## Proposed Architecture

```text
+--------------------+        +---------------------+        +------------------+
| Incoming Request   |  -->   | Auth Extract (JWT)  |  -->   | Tenancy Resolver |
 
+--------------------+        +---------------------+        +------------------+
                                                             | (header > token  |
                                                             |  > inferred)     |
                                                             +---------+--------+
                                                                       v
 
                                                   +---------------------------+
                                                   | RBAC Guard (declarative)  |
                                                   +-------------+-------------+
                                                                 v
                                                   +---------------------------+
                                                   | Audit Context Injection   |
                                                   +-------------+-------------+
                                                                 v
 
                                                   +---------------------------+
                                                   |    Service Handler        |
                                                   +---------------------------+
```

## Crate: `common_tenancy` (new) vs Extend `common_auth`

- Keep `common_auth` focused on JWT verification & claims.
- Introduce `common_tenancy` depending on `common_auth` for higher-level features.

### Data Structures

```rust
pub struct ExecutionContext {
    pub actor_id: Uuid,
    pub tenant_id: Uuid,
    pub roles: Arc<[String]>,
    pub correlation_id: Uuid,
    pub request_id: Uuid,
    pub issued_at: DateTime<Utc>,
}
```

### Extraction Layer

Axum extractor implementing `FromRequestParts` to build `ExecutionContext`:

- Reads `x-tenant-id` header (if present) and validates consistency with token claim.
- Generates correlation & request IDs (header passthrough if already present e.g., `x-correlation-id`).
- (Later) can perform tenant activity/plan status check via cache.

### RBAC

Provide macros or helper:

```rust
ensure_roles!(ctx, [ROLE_ADMIN, ROLE_MANAGER]);
// or attribute style (proc macro future):
#[require_roles(ROLE_ADMIN, ROLE_MANAGER)]
async fn handler(ctx: ExecutionContext, Json(p): Json<Payload>) -> ...
```

### Audit Helper

Ergonomic macro to emit enriched event:

```rust
audit_event!(ctx, "inventory.adjustment", { "product_id": pid, "delta": delta });
 - Outputs canonical fields: timestamp, actor_id, tenant_id, action, severity, payload, correlation_id.

## Phased Delivery

1. Crate Bootstrap: ExecutionContext extractor + role guard helpers (no audit yet). Pilot in `inventory-service` + `product-service`.
2. Add audit event macro & wire to Kafka topic `audit.events`. Introduce shared schema JSON in crate.
3. Remove direct `tenant_id_from_request` usage from pilot services -> replace with `ExecutionContext` arg.
4. Roll out to remaining services (orders, loyalty, payments, returns). Provide codemod guidelines.
5. Add optional policy hooks (feature gating / subscription tiers) inside extractor.
6. Introduce request-scoped metrics (counter per role/action) leveraging the same context.

## Compatibility & Migration Strategy

- Dual path: keep old helpers available (deprecated) for 1–2 releases.
- Lint (clippy + custom deny) banning new direct calls to deprecated helpers after pilot success.
- Provide sample diff + documentation for converting a handler.

## Risks / Mitigations

| Risk | Mitigation |
|------|------------|
| Large PR touching many handlers | Pilot + incremental batches |
| Hidden side-effects (auth differences) | Golden tests on representative endpoints before/after |
| Performance overhead of extra extraction | Keep extractor minimal; reuse parsed claims |
| Audit volume explosion | Rate limit or sample low-value events initially |

## Open Questions

- Should correlation_id be client-provided first wins or always regenerated? (Proposed: trust first non-empty header `x-correlation-id`).
- Severity taxonomy centralization (info, security, compliance, anomaly)?
- Do we need tenant-level circuit breakers integrated here?

## Immediate Next Steps (Post Multi-Location)

1. Scaffold crate layout (`services/common/tenancy` or new top-level `services/common-tenancy`).
2. Implement `ExecutionContext` + extractor (reads JWT claims + header).
3. Replace in one handler per pilot service; validate logs.
4. Add audit macro + Kafka producer config.
5. Document migration guide in `docs/security/tenancy_unification_plan.md` (this file evolves).

---
Draft complete; iterate with team feedback before refactor rollout.
