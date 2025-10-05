param(
  [string]$TenantId,
  [string]$ProductServiceUrl,
  [string]$OrderServiceUrl,
  [string]$JwtIssuer,
  [string]$JwtAudience,
  [string]$JwtPemPath,
  [string]$Kid,
  [int]$DiscountPercentBp,
  [int]$HeaderTaxBps,
  [ValidateSet('cash','card')][string]$PaymentMethod,
  [switch]$CreateOrder,
  [string]$Token
)

$ErrorActionPreference = 'Stop'

# Defaults (set after param for maximum PS 5.1 compatibility)
if (-not $ProductServiceUrl) { $ProductServiceUrl = "http://localhost:8081" }
if (-not $OrderServiceUrl) { $OrderServiceUrl = "http://localhost:8084" }
if (-not $JwtIssuer) { $JwtIssuer = "https://auth.novapos.local" }
if (-not $JwtAudience) { $JwtAudience = "novapos-frontend,novapos-admin,novapos-postgres" }
if (-not $JwtPemPath) { $JwtPemPath = (Join-Path $PSScriptRoot "..\jwt-dev.pem") }
if (-not $Kid) { $Kid = "local-dev" }
if (-not $DiscountPercentBp) { $DiscountPercentBp = 1000 }
if (-not $HeaderTaxBps) { $HeaderTaxBps = 800 }
if (-not $PaymentMethod) { $PaymentMethod = 'cash' }
if (-not $TenantId) { $TenantId = ([guid]::NewGuid()).ToString() }

function Step($msg) { Write-Host "`n==> $msg" -ForegroundColor Cyan }

# Mint a token if not provided
if (-not $Token) {
  Step "Minting dev JWT via scripts/mint-dev-jwt.js"
  if (-not (Test-Path $JwtPemPath)) { throw "PEM not found at $JwtPemPath" }
  $node = (Get-Command node -ErrorAction SilentlyContinue)
  if (-not $node) { throw "Node.js not found in PATH. Install Node or pass -Token manually." }
  $UserId = [guid]::NewGuid().ToString()
  Push-Location (Join-Path $PSScriptRoot "..")
  try {
    $Token = node .\scripts\mint-dev-jwt.js --tenant $TenantId --sub $UserId --roles Admin,Cashier --iss $JwtIssuer --aud $JwtAudience --audMode single --kid $Kid
  } finally { Pop-Location }
}

Step "Using tenant $TenantId"

# Helper to upsert a product by SKU
function Set-ProductBySku {
  param(
    [string]$Sku,
    [string]$Name,
    [decimal]$Price,
    [string]$TaxCode = 'STD'
  )
  $headers = @{
    'Content-Type' = 'application/json'
    'X-Tenant-ID'  = $TenantId
    'X-Roles'      = 'Admin'
    'Authorization'= "Bearer $Token"
  }
  $findUrl = ("{0}/products?sku={1}" -f $ProductServiceUrl.TrimEnd('/'), [uri]::EscapeDataString($Sku))
  $existingArr = @()
  try {
    $resp = Invoke-RestMethod -Method Get -Uri $findUrl -Headers $headers
    if ($resp -is [Array]) { $existingArr = $resp }
    elseif ($resp) { $existingArr = @($resp) }
  } catch { $existingArr = @() }
  $match = @($existingArr | Where-Object { $_.sku -eq $Sku })
  if ($match.Count -gt 0) {
    $p = $match[0]
    $updUrl = ("{0}/products/{1}" -f $ProductServiceUrl.TrimEnd('/'), $p.id)
    $body = @{ name=$Name; price=[decimal]$Price; description=$p.description; active=$true; image=$p.image; sku=$Sku; tax_code=$TaxCode } | ConvertTo-Json -Depth 5
    return Invoke-RestMethod -Method Put -Uri $updUrl -Headers $headers -Body $body
  } else {
    $body = @{ name=$Name; price=[decimal]$Price; description=''; image=$null; sku=$Sku; tax_code=$TaxCode } | ConvertTo-Json -Depth 5
    $createUrl = ("{0}/products" -f $ProductServiceUrl.TrimEnd('/'))
    return Invoke-RestMethod -Method Post -Uri $createUrl -Headers $headers -Body $body
  }
}

Step "Seeding products with SKUs"
$p1 = Set-ProductBySku -Sku 'SKU-SODA' -Name 'Soda Can' -Price 1.99 -TaxCode 'STD'
$p2 = Set-ProductBySku -Sku 'SKU-WATER' -Name 'Bottle Water' -Price 1.49 -TaxCode 'EXEMPT'
Write-Host "Seeded: $($p1.id) SKU-SODA, $($p2.id) SKU-WATER"

# Build a local SKU->ID map from seeded responses to avoid downstream re-fetch mismatches
$skuToIdSeeded = @{
  'SKU-SODA' = $p1.id
  'SKU-WATER' = $p2.id
}

# Build compute/order payload
$items = @(
  @{ sku = 'SKU-SODA'; quantity = 2 },
  @{ sku = 'SKU-WATER'; quantity = 1 }
)

# First: compute via /orders/compute using SKUs
$headersCompute = @{
  'Content-Type'   = 'application/json'
  'X-Tenant-ID'    = $TenantId
  'X-Roles'        = 'admin,cashier'
  'Authorization'  = "Bearer $Token"
  'x-tax-rate-bps' = [string]$HeaderTaxBps
}
$computeBody = @{ items = $items; discount_percent_bp = $DiscountPercentBp } | ConvertTo-Json -Depth 5
$computeUrl = ("{0}/orders/compute" -f $OrderServiceUrl.TrimEnd('/'))
Step "Computing totals from SKUs with header tax=$HeaderTaxBps bps, discount=$DiscountPercentBp bps"
try {
  $comp = Invoke-RestMethod -Method Post -Uri $computeUrl -Headers $headersCompute -Body $computeBody
} catch {
  $body = $_.ErrorDetails.Message
  if ($body -and $body -like '*product_not_found*') {
    Write-Warning "SKU compute returned product_not_found. Falling back to product_id-based compute."
    # Use IDs from the seeded responses to avoid any filtering inconsistencies
    $itemsById = @()
    foreach ($it in $items) {
      if ($skuToIdSeeded.ContainsKey($it.sku)) {
        $obj = New-Object psobject
        $obj | Add-Member -NotePropertyName 'product_id' -NotePropertyValue $skuToIdSeeded[$it.sku]
        $obj | Add-Member -NotePropertyName 'quantity' -NotePropertyValue $it.quantity
        $itemsById += $obj
      }
    }
    if ($itemsById.Count -eq 0) { throw }
  $computeBody2 = @{ items = $itemsById; discount_percent_bp = $DiscountPercentBp } | ConvertTo-Json -Depth 5
  Write-Host ("Fallback compute with product_ids: {0}" -f (($itemsById | ForEach-Object { $_.'product_id' }) -join ',')) -ForegroundColor Gray
    try {
      $comp = Invoke-RestMethod -Method Post -Uri $computeUrl -Headers $headersCompute -Body $computeBody2
    } catch {
      $body2 = $_.ErrorDetails.Message
      if ($body2 -and $body2 -like '*product_not_found*') {
        Write-Error "Order-service could not find products even by id. Likely the services point to different databases or schemas."
        Write-Host "\nTroubleshooting tips:" -ForegroundColor Yellow
        Write-Host "  - Ensure product-service and order-service share the same DATABASE_URL (host, db name, user)."
        Write-Host "  - Run migrations and restart services so both see the same 'products' table."
        Write-Host "  - Alternatively, run the other demos [1] and [2] which don't require SKU lookups in order-service."
        throw
      }
      throw
    }
  } else { throw }
}
Write-Host ("Subtotal={0}  Discount={1}  Tax={2}  Total={3}" -f $comp.subtotal_cents, $comp.discount_cents, $comp.tax_cents, $comp.total_cents)

if ($CreateOrder) {
  Step "Creating order via /orders/sku with $PaymentMethod"
  $headersOrder = @{
    'Content-Type'   = 'application/json'
    'X-Tenant-ID'    = $TenantId
    'X-Roles'        = 'admin,manager,cashier'
    'Authorization'  = "Bearer $Token"
  }
  $payment = $null
  if ($PaymentMethod -eq 'cash') {
    $payment = @{ method='cash'; amount_cents = [int]$comp.total_cents + 100 } # tender a bit extra to show change
  } elseif ($PaymentMethod -eq 'card') {
    $payment = @{ method='card'; amount_cents = [int]$comp.total_cents }
  }
  $orderBodyObj = @{ items = $items; discount_percent_bp = $DiscountPercentBp; payment_method = $PaymentMethod; payment = $payment }
  # pass header tax rate via explicit field as well to lock behavior
  $orderBodyObj.tax_rate_bps = $HeaderTaxBps
  $orderUrl = ("{0}/orders/sku" -f $OrderServiceUrl.TrimEnd('/'))
  try {
    $order = Invoke-RestMethod -Method Post -Uri $orderUrl -Headers $headersOrder -Body ($orderBodyObj | ConvertTo-Json -Depth 6)
  } catch {
    $body = $_.ErrorDetails.Message
    if ($body -and $body -like '*product_not_found*') {
      Write-Warning "Order /orders/sku returned product_not_found. Falling back to /orders with product_id items."
      # Build items by id including unit_price and line_total using compute response (cents)
      $itemsId2 = @()
      $compItemsByPid = @{}
      if ($comp -and $comp.items) {
        foreach ($ci in $comp.items) { $compItemsByPid[$ci.product_id] = $ci }
      }
      foreach ($it in $items) {
        if ($skuToIdSeeded.ContainsKey($it.sku)) {
          $pid2 = $skuToIdSeeded[$it.sku]
          $ci = $compItemsByPid[$pid2]
          $unitCents = if ($ci) { [int]$ci.unit_price_cents } else { 0 }
          $lineCents = if ($ci) { [int]$ci.line_subtotal_cents } else { [int]$unitCents * [int]$it.quantity }
          $obj2 = [pscustomobject]@{
            product_id   = $pid2
            product_name = $null
            quantity     = [int]$it.quantity
            unit_price   = [decimal]($unitCents / 100.0)
            line_total   = [decimal]($lineCents / 100.0)
          }
          $itemsId2 += $obj2
        }
      }
      # Total from compute to keep parity with tax/discount used in compute
      $totalCents = if ($comp -and $comp.total_cents) { [int]$comp.total_cents } else { 0 }
      $orderBody2 = @{
        items = $itemsId2
        payment_method = $PaymentMethod
        payment = $payment
        total = [decimal]($totalCents / 100.0)
      }
      # Add optional fields if present
      $orderBody2.tax_rate_bps = $HeaderTaxBps
  Write-Host ("Fallback order with product_ids: {0}" -f (($itemsId2 | ForEach-Object { $_.'product_id' }) -join ',')) -ForegroundColor Gray
      try {
        $order = Invoke-RestMethod -Method Post -Uri ("{0}/orders" -f $OrderServiceUrl.TrimEnd('/')) -Headers $headersOrder -Body ($orderBody2 | ConvertTo-Json -Depth 6)
      } catch {
        $body3 = $_.ErrorDetails.Message
        if ($body3 -and $body3 -like '*product_not_found*') {
          Write-Error "Order-service still does not see products by id. Please align service DATABASE_URL settings."
          Write-Host "\nTroubleshooting tips:" -ForegroundColor Yellow
          Write-Host "  - Verify both services target the same database and schema (products table)."
          Write-Host "  - Re-run docker-compose or service runners after updating envs."
          throw
        }
        throw
      }
    } else { throw }
  }
  Write-Host "Order created: id=$($order.id) status=$($order.status) total=$($order.total)"
  $receiptUrl = ("{0}/orders/{1}/receipt?format=txt" -f $OrderServiceUrl.TrimEnd('/'), $order.id)
  $receipt = Invoke-WebRequest -Method Get -Uri $receiptUrl -Headers $headersOrder -UseBasicParsing
  Write-Host "`n--- Receipt ---`n$($receipt.Content)" -ForegroundColor DarkGreen
}

Step "Done"
