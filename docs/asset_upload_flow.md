# Asset Upload Service & Pre-Signed URL Flow

## Goals
- Allow admins to attach product images without exposing the product-service to large file uploads.
- Use cloud object storage + CDN for serving images across POS/e-commerce channels.
- Capture audit events whenever product media changes.

## Core Components
- **Asset service (new microservice)**: issues pre-signed upload URLs, validates file metadata, and records asset metadata (tenant, file key, checksum).
- **Object storage bucket**: stores originals and derived thumbnails. Versioning enabled for rollback.
- **CDN**: fronts the bucket for fast delivery and cache control.
- **Product service**: stores the public CDN URL, publishes audit events (`product_audit_log`).
- **Admin portal**: consumes pre-signed URLs for direct upload from the browser.

## Sequence (Upload)
1. Admin clicks "Upload image" on the product screen.
2. Frontend calls `POST /assets/uploads` with tenant + optional product ID.
3. Asset service validates actor, file intent, and responds with:
   - `upload_url`: pre-signed PUT/POST to object storage.
   - `asset_url`: CDN URL that will serve the uploaded object.
   - `headers`: any required headers for the upload (content-type, ACL, etc.).
4. Frontend uploads the file directly to the storage service using `upload_url`.
5. On success, frontend calls `PUT /products/{id}` with the returned `asset_url`.
6. Product service updates the record, emits `product_audit_log` entry and `product.updated` event.

## Asset Service APIs (initial)
- `POST /assets/uploads`
  - Request: `{ "tenant_id": UUID, "file_name": string, "content_type": string, "product_id"?: UUID }`
  - Response: `{ "upload_url": string, "asset_url": string, "headers": Record<string,string>, "expires_in": number }`
- `DELETE /assets/{asset_id}` (future): revoke asset + clean storage, optionally cascade to product.
- `GET /assets/{asset_id}`: metadata (size, checksum, linked products).

## Storage Layout
```
/tenant/{tenant_id}/products/{product_id}/{uuid}.original.ext
/tenant/{tenant_id}/products/{product_id}/{uuid}.thumb-400.jpg
/tenant/{tenant_id}/shared/{uuid}.png
```
- Lifecycle rule keeps originals, auto-expires temporary uploads after 24h if never linked.
- Derived thumbnails generated via async worker (Lambda/Cloud Function triggered on `ObjectCreated`).

## Security & Governance
- Authentication: asset service validates the same JWT as other services.
- Authorization: verify tenant context + role (only manager/admin roles can upload).
- Signed URLs TTL: 5-10 minutes. Enforce `content-type` and max size via signature conditions.
- Virus/malware scanning: optional async job before activating asset (quarantine bucket).
- Audit: asset service emits `asset.created`, `asset.deleted`; product service logs actor and image changes.

## Admin Portal Integration
- Replace the current URL text field with:
  1. File picker -> call asset service for pre-signed URL.
  2. Upload progress indicator.
  3. On success, call product API with the returned `asset_url`.
- Keep fallback "Paste URL" option for external images.
- Display audit history (image change log) by querying `/product_audit` endpoint (future).

## Product Service Enhancements
- `/products` API now records audit logs and expects optional `X-User-ID`, `X-User-Name`, `X-User-Email` headers.
- Add read-only endpoint `/products/{id}/audit` to expose history for admin UI (follow-up task).

## Migrations & Ops
- New table `product_audit_log` seeded by current change.
- Asset service requires secrets for storage (bucket name, IAM credentials) delivered via secret manager.
- Terraform/IaC changes: bucket, CDN distribution, Cloud Function for thumbnails, service deployment.

## Next Steps
1. Implement asset-service skeleton (Rust or Node) with `POST /assets/uploads` and JWT validation.
2. Provision object storage bucket + IAM role (read/write limited to asset service).
3. Update admin portal to use the new API and show upload progress.
4. Extend product-service with `/products/{id}/audit` endpoint for UI consumption.
5. Add monitoring: count uploads, failures, storage size, audit log volume.
