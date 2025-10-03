Integration Gateway
===================

This service brokers external client requests (API key or JWT authenticated) to internal NovaPOS services while applying rate limiting, metrics, and optional Kafka event publication.

New Environment Variable
------------------------

PAYMENT_SERVICE_FALLBACK_AUTH

Purpose: When an external caller uses an API key (no Authorization header) to initiate a payment or void request, the gateway previously forwarded the request to the payment-service without credentials. The payment-service then rejected the request, preventing downstream Kafka events (e.g. payment.completed) from being emitted in real-broker integration tests.

Set this variable to a value that will be accepted by the payment-service's auth layer (typically a Bearer token value of an internal service principal).

Format: `Bearer YOUR_TOKEN_VALUE`

Behavior:

1. If the incoming request already includes an Authorization header (JWT bearer flow), that header is forwarded unchanged.
2. If no Authorization header is present and PAYMENT_SERVICE_FALLBACK_AUTH is set, the gateway injects this value as the Authorization header when calling the payment-service.
3. If neither is present, the request is forwarded without Authorization (legacy behavior) and may be rejected by payment-service.

Security Considerations:

* Scope the fallback token to only required payment actions (least privilege) if the payment-service supports capability scoping.
* Rotate the token regularly and store it in a secrets manager / Vault rather than a plain .env file for production deployments.
* Monitor gateway metrics for payment failures to validate the fallback is operating as expected.

Metrics Impact:
Successful usage of the fallback path should correlate with payment.completed events even for API-key initiated flows, reducing false negatives in Kafka-driven test harnesses.

Rollback:
Unset PAYMENT_SERVICE_FALLBACK_AUTH to restore prior behavior (no injected Authorization for API key flows).
