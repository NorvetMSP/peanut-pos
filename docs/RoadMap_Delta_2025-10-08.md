# Roadmap Delta — 2025-10-08

This memo summarizes what we added to the roadmap based on the Journey Map analysis, why it matters, and who benefits.

## What changed (new phases and sub-phases)

- Phase 0.1 KPI & Messaging Observability — checkout SLOs, Kafka lag, POS offline queue telemetry
- Phase 2.1 Security Hardening & Data Privacy — mTLS, per-tenant keys, key rotation/DSR, immutable audit
- Phase 3.1 Returns Policy & Fraud Controls — restock compensation, link to original lines, fraud thresholds
- Phase 4.1 Loyalty Offline & 360 — offline cache + reconciliation, reversals/adjustments, Customer 360 API
- Phase 5.1 Order Integrity — idempotency storage + outbox/inbox; price-lock snapshots
- Phase 6.1 Tender Orchestration — split tenders, auth/capture/void, gateway reconciliation
- Phase 8.1 BOPIS SLA — SLA dashboards and auto-release
- New phases 11–18 — Variants/Serials, Approvals, Devices, Promotions, Data & Analytics plumbing, E‑receipts, Tasking, Identity at scale

## Why it matters

- Checkout resilience & integrity (Phases 0.1, 5.1, 6.1): Prevent duplicates, ensure price-lock, support real tender flows; protects revenue and cashier UX.
- Observability & SLOs (Phase 0.1): Lets us measure and enforce <2s/<5-tap targets per tenant/store; proactive alerting.
- Security & privacy (Phase 2.1): mTLS, tenant keys, rotation, and immutable audit strengthen compliance posture and trust.
- Returns & fraud (Phase 3.1): Policy-aware returns, restock correctness, and fraud gates reduce losses.
- Loyalty value (Phase 4.1): Accurate balances online/offline; 360 improves service and personalization.
- BOPIS (Phase 8.1): SLAs and no-show release prevent inventory lock-ups and missed commitments.
- Catalog depth (Phase 11): Variants/serials unlock apparel/electronics.
- Operational excellence (Phases 12–17): Approvals, device reliability, promo governance, analytics, notifications, and tasking deliver store readiness.
- Enterprise identity (Phase 18): SSO/SCIM/ABAC reduce admin toil and improve security.

## Quick next steps (suggested sequencing)

1. Phase 0.1 — instrument KPIs and lag; publish starter dashboards (included in repo)
2. Phase 5.1 — idempotency + outbox/inbox in Order/Payment
3. Phase 6.1 — payment state machine and split tenders
4. Phase 3.1 — restock rules and linkage for returns
5. Phase 2.1 — mTLS and tenant keys expansion

## Acceptance framing

Each item has clear Actions/Acceptance in the checklist; dashboards and alerting are pre-seeded to visualize progress.
