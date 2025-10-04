$ErrorActionPreference = 'Stop'

param(
  [string]$TenantId
)

if (-not $TenantId) {
  $TenantId = [guid]::NewGuid().ToString()
}

$sku1 = 'SKU-SODA'
$sku2 = 'SKU-WATER'
$p1 = [guid]::NewGuid().ToString()
$p2 = [guid]::NewGuid().ToString()

$insert = @"
INSERT INTO products (id, tenant_id, name, price, description, active, image, sku, tax_code)
VALUES ('$p1', '$TenantId', 'Soda Can', 1.99, '', true, '', '$sku1', 'STD'),
       ('$p2', '$TenantId', 'Bottle Water', 1.49, '', true, '', '$sku2', 'EXEMPT');
"@

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
