$ErrorActionPreference = 'Stop'

# Tenant for this run
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
Write-Host "Ensuring payments table exists..."
$ensurePayments = @"
CREATE TABLE IF NOT EXISTS payments (
  id UUID PRIMARY KEY,
  tenant_id UUID NOT NULL,
  order_id UUID NOT NULL,
  method TEXT NOT NULL,
  amount NUMERIC NOT NULL,
  status TEXT NOT NULL,
  change_cents INTEGER NULL,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
"@
docker compose exec -T postgres psql -U novapos -d novapos -c $ensurePayments | Out-Null
Write-Host "Seeding products for tenant $TenantId..."
docker compose exec -T postgres psql -U novapos -d novapos -c $insert | Out-Null

$headers = @{}
$headers['X-Tenant-ID'] = $TenantId
$headers['X-Roles'] = 'admin'
$headers['X-Tax-Rate-Bps'] = '800'

# First compute to get total
$computeBody = [pscustomobject]@{
  items = @(
    [pscustomobject]@{ sku = $sku1; quantity = 2 },
    [pscustomobject]@{ sku = $sku2; quantity = 1 }
  )
  discount_percent_bp = 1000
}
$computeJson = $computeBody | ConvertTo-Json -Depth 5

Write-Host "Computing order totals for tenant $TenantId ..."
$comp = Invoke-RestMethod -Method Post -Uri 'http://localhost:8084/orders/compute' -Headers $headers -ContentType 'application/json' -Body $computeJson
$comp | ConvertTo-Json -Depth 5

$total = [int]$comp.total_cents
$tender = $total + 100  # give an extra dollar for change demo

# Create order from SKUs with payment
$orderBody = [pscustomobject]@{
  items = @(
    [pscustomobject]@{ sku = $sku1; quantity = 2 },
    [pscustomobject]@{ sku = $sku2; quantity = 1 }
  )
  discount_percent_bp = 1000
  payment_method = 'cash'
  payment = @{ method = 'cash'; amount_cents = $tender }
}
$orderJson = $orderBody | ConvertTo-Json -Depth 6

Write-Host "Creating PAID order from SKUs (cash) for tenant $TenantId ..."
if (-not $env:AUTH_BEARER -or [string]::IsNullOrWhiteSpace($env:AUTH_BEARER)) {
  Write-Warning "AUTH_BEARER not set. Skipping order creation. Set a valid JWT in AUTH_BEARER to create the order."
  exit 0
}
$headers['Authorization'] = "Bearer $($env:AUTH_BEARER)"
$order = Invoke-RestMethod -Method Post -Uri 'http://localhost:8084/orders/sku' -Headers $headers -ContentType 'application/json' -Body $orderJson
$order | ConvertTo-Json -Depth 6

# Fetch receipt text
$rid = $order.id
Write-Host "Receipt for $rid"
$receipt = Invoke-WebRequest -Method Get -Uri ("http://localhost:8084/orders/$rid/receipt?format=txt") -Headers $headers -UseBasicParsing
$receipt.Content
