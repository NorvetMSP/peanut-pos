$ErrorActionPreference = 'Stop'

# Use provided tenant via env var or generate new
if (-not $env:SEED_TENANT_ID -or [string]::IsNullOrWhiteSpace($env:SEED_TENANT_ID)) {
  $TenantId = [guid]::NewGuid().ToString()
} else {
  $TenantId = $env:SEED_TENANT_ID
}

$sku1 = 'SKU-SODA'
$sku2 = 'SKU-WATER'
$p1 = [guid]::NewGuid().ToString()
$p2 = [guid]::NewGuid().ToString()

$ensureTable = @"
CREATE TABLE IF NOT EXISTS products (
  id uuid PRIMARY KEY,
  tenant_id uuid NOT NULL,
  name text NOT NULL,
  price numeric(10,2) NOT NULL,
  description text NOT NULL DEFAULT '',
  image text NOT NULL DEFAULT '',
  active boolean NOT NULL DEFAULT true,
  sku text NULL,
  tax_code text NULL
);
"@

$insert = @"
INSERT INTO products (id, tenant_id, name, price, description, active, image, sku, tax_code)
VALUES ('$p1', '$TenantId', 'Soda Can', 1.99, '', true, '', '$sku1', 'STD')
ON CONFLICT (id) DO NOTHING;
INSERT INTO products (id, tenant_id, name, price, description, active, image, sku, tax_code)
VALUES ('$p2', '$TenantId', 'Bottle Water', 1.49, '', true, '', '$sku2', 'EXEMPT')
ON CONFLICT (id) DO NOTHING;
"@

Write-Host "Ensuring products table exists..."
docker compose exec -T postgres psql -U novapos -d novapos -c $ensureTable | Out-Null
Write-Host "Seeding products for tenant $TenantId..."
docker compose exec -T postgres psql -U novapos -d novapos -c $insert | Out-Null

$headers = @{}
$headers['X-Tenant-ID'] = $TenantId
$headers['X-Roles'] = 'admin'
$headers['X-Tax-Rate-Bps'] = '800'

$bodyObj = [pscustomobject]@{
  items = @(
    [pscustomobject]@{ sku = $sku1; quantity = 2 },
    [pscustomobject]@{ sku = $sku2; quantity = 1 }
  )
  discount_percent_bp = 1000
}
$bodyJson = $bodyObj | ConvertTo-Json -Depth 5

Write-Host "POST /orders/compute for tenant $TenantId ..."
$resp = Invoke-WebRequest -Method Post -Uri 'http://localhost:8084/orders/compute' -Headers $headers -ContentType 'application/json' -Body $bodyJson -UseBasicParsing
$resp.Content
