# Audit Event Schema Versioning

This document defines how audit event payloads evolve across NovaPOS services.

## Goals

- Backward compatibility for downstream consumers (SIEM, data lake) during rolling deploys.
- Deterministic detection of breaking vs additive changes.
- Simple upgrade guidance per service release.

## Version Field

Each audit event includes:

- `schema_version` (integer) — monotonically increasing per logical event type family.
- `action` — stable identifier of the event (e.g. `inventory.reservation.expired`).

Example (current):

```json
{
  "action": "inventory.reservation.expired",
  "schema_version": 1,
  "tenant_id": "<uuid>",
  "order_id": "<uuid>",
  "product_id": "<uuid>",
  "quantity": 3,
  "expired_at_epoch": 1759339552
}
```

## Change Categories

| Change Type | Allowed Without Bump | Requires Version Bump | Notes |
|-------------|----------------------|------------------------|-------|
| Add optional field | ✅ | No | Field must be semantically optional and non-breaking to parsers |
| Add required field | ❌ | Yes | Increment version and document migration path |
| Remove field | ❌ | Yes | Prefer deprecate + null until major bump |
| Rename field | ❌ | Yes | Emit both old+new for one version window when possible |
| Type widening (int32->int64) | ❌ | Yes | Avoid; add new field instead |
| Enum new variant | ✅ | No | Consumers must treat unknown as opaque |
| Enum variant rename | ❌ | Yes | Add new + mark old deprecated for one cycle |

## Bumping Procedure

1. Implement change, add new version integer (previous_version + 1).
2. Update this document: add a row to Changelog table.
3. (If breaking) maintain dual emission for one deploy cycle if feasible.
4. Notify consumers (Slack #platform-audit and release notes).

## Consumer Guidance

- Use `action` + `schema_version` as a composite key for parser selection.
- Ignore unknown fields; treat missing optional fields as None.
- Log and surface unknown `schema_version` to alert on forward compatibility gaps.

## Testing Strategy

- Unit tests asserting field presence for current version.
- Snapshot tests (JSON) per version variant (kept under `tests/audit_snapshots/`).
- Backward compatibility test ensuring last version payload still deserializes with current parser (when dual emission skipped, keep copy in fixture).

## Deprecation Window

Breaking change with dual emission should keep old version for at least one minor release or two weeks (whichever longer) before removal.

## Changelog

| Version | Action | Change Summary | Date | Notes |
|---------|--------|----------------|------|-------|
| 1 | inventory.reservation.expired | Initial emission | 2025-10-01 | Baseline |

---
Ownership: Platform Engineering. Propose changes via PR updating this file plus associated code/tests.
