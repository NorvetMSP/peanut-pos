# Scripts

Quick helpers to demo and validate NovaPOS flows locally on Windows PowerShell.

- try-compute.ps1
  - Seeds a single product and calls order-service /orders/compute with a header tax override, then upserts a tenant DB tax override and computes again.
  - Params: -TenantId, -ProductServiceUrl, -OrderServiceUrl, -JwtIssuer, -JwtAudience, -JwtPemPath, -Kid, -Token

- try-precedence.ps1
  - Demonstrates tax override precedence: upserts tenant, location, and POS overrides, then calls /orders/compute with location_id and pos_instance_id.
  - Params: -TenantId, -TenantRateBps, -LocationRateBps, -PosRateBps, -ProductServiceUrl, -OrderServiceUrl, -Token

- seed-skus-and-order.ps1
  - Upserts two products with SKUs (taxable + exempt), computes totals using SKUs, and optionally creates a paid order via /orders/sku with a plaintext receipt.
  - Params: -TenantId, -DiscountPercentBp, -HeaderTaxBps, -PaymentMethod cash|card, -CreateOrder, -ProductServiceUrl, -OrderServiceUrl, -Token

- run-demos.ps1
  - Orchestrator that runs smoke-check first, then presents an interactive menu to run the above demos.
  - Params: -TenantId, -ProductServiceUrl, -OrderServiceUrl, -DatabaseUrl, -TestDatabaseUrl, -JwtIssuer, -JwtAudience, -JwtPemPath, -Kid, -Token

All scripts can mint a dev JWT automatically if -Token is not provided (uses scripts/mint-dev-jwt.js and jwt-dev.pem with kid=local-dev). Pass -TenantId to reuse an existing tenant; otherwise a GUID is generated.

## Examples

```powershell
# Basic compute + DB override demo
./try-compute.ps1

# Precedence demo with custom rates
./try-precedence.ps1 -TenantRateBps 700 -LocationRateBps 825 -PosRateBps 950

# Seed SKUs and create a cash order with receipt
./seed-skus-and-order.ps1 -CreateOrder -PaymentMethod cash

# Interactive orchestrator (runs smoke-check first)
./run-demos.ps1
```

## Keep the Service matrix in sync

We auto-generate the Service matrix in `docs/Architecture/Architecture_10_7_2025.md` from `docker-compose.yml`, `.env`, and `docs/topics-map.json`.

- To generate manually:
  - `./scripts/generate-service-matrix.ps1`

- Pre-commit hook (local):
  - Install: `./scripts/install-git-hooks.ps1`
  - On each commit, the hook runs the generator and blocks the commit if the doc changes. Commit the updated doc and retry.

- CI check (GitHub Actions):
  - Workflow `.github/workflows/verify-service-matrix.yml` re-runs the generator and fails if the file changes, preventing drift on PRs.

Topic configuration lives in `docs/topics-map.json`. Update it when services publish/consume new topics.
