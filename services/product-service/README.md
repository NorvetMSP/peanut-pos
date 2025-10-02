# Product Service

## Audit Events Redacted View (TA-AUD-7)

The endpoint `GET /audit/events` supports role-based redaction of sensitive fields.

### Query Parameters

- `limit` (int, optional, default 50, max 200)
- Filtering params: `before`, `before_event_id`, `actor_id`, `action`, `entity_type`, `entity_id`, `severity`, `trace_id`
- `include_redacted` (bool, optional, default false):
  - `false` (default): sensitive fields are omitted entirely for non-privileged roles
  - `true`: sensitive fields are present but values are masked with a placeholder ("****")

### Role Privileges

- Privileged: `Admin` (full visibility)
- Non-privileged example roles: `Support`, `Manager`, `Inventory` (receive redacted payload/meta values)

### Response Additions

Each event object now includes:

- `redacted_fields`: array of field paths actually redacted for this viewer
- `include_redacted`: echo of the request choice
- `privileged_view`: boolean indicating if full visibility applied

### Metrics

Prometheus exposition (`/internal/metrics`):

- `audit_view_redactions_total` (counter)
  - Unlabeled total line
  - Per-field labeled lines with `{tenant_id, role, field}` (interim in-memory tally)

### Configuration

- `AUDIT_VIEW_REDACTION_PATHS` (comma-separated dot paths, e.g. `payload.customer.email,payload.payment.card_last4`)
  - Applies to both `payload` and `meta` roots (first segment selects root object)

### Example

```bash
GET /audit/events?limit=20&include_redacted=true
```

Sample event snippet (non-privileged):

```json
{
  "event_id": "...",
  "action": "order.create",
  "payload": { "order_id": "...", "customer": { "email": "****" } },
  "redacted_fields": ["payload.customer.email"],
  "include_redacted": true,
  "privileged_view": false
}
```

### Future Enhancements

- Array / wildcard path matching
- Per-field role exceptions
- Transition labeled metrics to a real Prometheus registry with bounded cardinality
- View access audit logging

---
Refer to root backlog `docs/development/tenancy_audit_backlog.md` item `TA-AUD-7` for tracking details.
