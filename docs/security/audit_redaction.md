# Audit Event Redaction (TA-AUD-6)

This document describes the redaction tagging and masking layer added to the audit ingestion pipeline.

## Goals

- Prevent storage or exposure of sensitive Personally Identifiable Information (PII) / financial tokens in clear form.
- Provide deterministic, centrally configurable masking rules without changing producer code paths.
- Supply metrics for observability and future alerting.

## Configuration

Environment variables (consumer side):

- `AUDIT_REDACTION_MODE` (default: `log`)
  - `off`  : No masking; events stored as-is.
  - `log`  : Masked copy stored; original value replaced with mask token pattern; metrics incremented.
  - `enforce`: Same as `log`; reserved for future stricter behaviors (e.g., reject if mask fails).
- `AUDIT_REDACTION_FIELDS` (optional): Comma-separated list of JSON pointer-like paths to redact. Paths are evaluated against both `payload` and `meta` objects. Example: `$.customer.email,$.payment.card_last4,$.customer.phone`.
  - Minimal pointer grammar: `$.segment.subsegment` (no array wildcard resolution implemented yet).
  - Case-sensitive on segment keys.
- `AUDIT_REDACTION_MASK` (optional): Replacement token; default `"***REDACTED***"`.

## Behavior

1. After deserializing an `AuditEvent`, prior to insertion, the consumer attempts to traverse each configured path for both `payload` and `meta`.
2. If a field is found and is a primitive (string/number/bool/null) or object/array, it is replaced by the mask token (always a JSON string) leaving structure above intact.
3. The masking process is idempotent; already masked fields remain masked.
4. Failures to parse paths are logged once (rate-limited by absence of dynamic counters presently) but do not block ingestion.
5. Metrics increment for each successful field redaction.

## Metrics (Prometheus)

- `audit_events_redacted_total` (counter) — total masked field occurrences.
- `audit_redaction_last_timestamp_ms` (gauge) — unix ms of last successful redaction operation.
- `audit_redaction_mode` (gauge, const 1 with label `mode`) — indicates active mode for quick dashboards.

## Limitations / Future Enhancements

- No array index wildcard or deep pattern matching (future: implement JSONPath subset or compiled trie for field names).
- No hashing/tokenization; currently pure masking. (Future: `AUDIT_REDACTION_HASH_SALT` to enable HMAC tokenization of selected fields.)
- No per-tenant override yet; global config only.

## Example

```bash
AUDIT_REDACTION_MODE=log
AUDIT_REDACTION_FIELDS=$.customer.email,$.customer.phone
AUDIT_REDACTION_MASK=***MASK***
```
Before:
{
  "payload": {"customer": {"email": "alice@example.com", "phone": "+15551234567"}},
  "meta": {}
}
After storage:
{
  "payload": {"customer": {"email": "***MASK***", "phone": "***MASK***"}},
  "meta": {}
}

## Testing Approach

- Unit-style masking logic exercised implicitly via ingestion path; can be extracted later into a pure function for direct tests.
- Manual validation: produce event with sensitive fields, observe masked values in `audit_events` table and metrics increments.

## Operational Guidance

- Start with `log` mode; once validated in lower environments, switch to `enforce` (behavior presently identical) to standardize naming before adding stricter validations.
- Maintain a runbook entry mapping field paths and business owners.

