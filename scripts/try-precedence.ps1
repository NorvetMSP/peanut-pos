param(
  [string]$TenantId = ([guid]::NewGuid()).Guid,
  [string]$ProductServiceUrl = "http://localhost:8081",
  [string]$OrderServiceUrl = "http://localhost:8084",
  [string]$JwtIssuer = "https://auth.novapos.local",
  [string]$JwtAudience = "novapos-admin",
  [string]$JwtPemPath = (Join-Path $PSScriptRoot "..\jwt-dev.pem"),
  [string]$Kid = "local-dev",
  [int]$TenantRateBps = 700,
  [int]$LocationRateBps = 800,
  [int]$PosRateBps = 900,
  [string]$Token
)

function Step($msg) { Write-Host "`n==> $msg" -ForegroundColor Cyan }
$script:usingFallback = $false

# Mint a token if not provided
if (-not $Token) {
  Step "Minting dev JWT via scripts/mint-dev-jwt.js"
  if (-not (Test-Path $JwtPemPath)) { throw "PEM not found at $JwtPemPath" }
  $node = (Get-Command node -ErrorAction SilentlyContinue)
  if (-not $node) { throw "Node.js not found in PATH. Install Node or pass -Token manually." }
  Push-Location (Join-Path $PSScriptRoot "..")
  try {
    $Token = node .\scripts\mint-dev-jwt.js --tenant $TenantId --roles Admin,Cashier --iss $JwtIssuer --aud $JwtAudience --audMode single --kid $Kid
  } finally { Pop-Location }
}

Step "Using tenant $TenantId"

# Create a sample taxable product ($10.00, STD)
$prodHeaders = @{
  'Content-Type' = 'application/json'
  'X-Tenant-ID'  = $TenantId
  'X-Roles'      = 'Admin'
  'Authorization'= "Bearer $Token"
}
$prodBody = @{ name = 'Precedence Demo Item'; price = 10.00; description = 'demo item'; image = $null; sku = $null; tax_code = 'STD' } | ConvertTo-Json -Depth 5
$prodUrl = ("{0}/products" -f $ProductServiceUrl.TrimEnd('/'))
Step "Creating product at $prodUrl"
try {
  $prodResp = Invoke-RestMethod -Method Post -Uri $prodUrl -Headers $prodHeaders -Body $prodBody
} catch {
  Write-Error "Failed to create product: $($_.Exception.Message)"; throw
}
$productId = $prodResp.id
Write-Host "Product created: id=$productId price=$($prodResp.price)"

# Choose a location and POS instance for overrides
$LocationId = [guid]::NewGuid().Guid
$PosInstanceId = [guid]::NewGuid().Guid

$adminHeaders = @{
  'Content-Type' = 'application/json'
  'X-Tenant-ID'  = $TenantId
  'X-Roles'      = 'Admin'
  'Authorization'= "Bearer $Token"
}
$adminUrl = ("{0}/admin/tax_rate_overrides" -f $OrderServiceUrl.TrimEnd('/'))
# Fallback: if the endpoint moved under /admin/orders/tax_rate_overrides in some envs, try that on 404
function Invoke-TaxOverrideUpsert {
  param([hashtable]$Headers,[string]$BaseUrl,[hashtable]$Body)
  $primary = ("{0}/admin/tax_rate_overrides" -f $BaseUrl.TrimEnd('/'))
  $alt = ("{0}/admin/orders/tax_rate_overrides" -f $BaseUrl.TrimEnd('/'))
  try {
    Write-Host ("POST {0}" -f $primary) -ForegroundColor Gray
    return Invoke-RestMethod -Method Post -Uri $primary -Headers $Headers -Body ($Body | ConvertTo-Json)
  } catch {
    $resp = $_.Exception.Response
    if ($resp) { Write-Host ("Primary failed HTTP {0}" -f $resp.StatusCode.value__) -ForegroundColor DarkYellow }
    if ($resp -and $resp.StatusCode.value__ -eq 404) {
      Write-Host ("POST {0}" -f $alt) -ForegroundColor Gray
      try { return Invoke-RestMethod -Method Post -Uri $alt -Headers $Headers -Body ($Body | ConvertTo-Json) }
      catch {
        $resp2 = $_.Exception.Response
        if ($resp2 -and $resp2.StatusCode.value__ -eq 404) {
          Write-Warning "Admin tax override endpoint not found (404) at both primary and alt paths. Continuing demo without DB upserts."
          $script:usingFallback = $true
          return $null
        }
        throw
      }
    }
    throw
  }
}

Step "Upserting tenant override: rate_bps=$TenantRateBps"
$null = Invoke-TaxOverrideUpsert -Headers $adminHeaders -BaseUrl $OrderServiceUrl -Body @{ rate_bps = $TenantRateBps }

Step "Upserting location override: location_id=$LocationId rate_bps=$LocationRateBps"
$null = Invoke-TaxOverrideUpsert -Headers $adminHeaders -BaseUrl $OrderServiceUrl -Body @{ location_id = $LocationId; rate_bps = $LocationRateBps }

Step "Upserting POS override: pos_instance_id=$PosInstanceId rate_bps=$PosRateBps"
$null = Invoke-TaxOverrideUpsert -Headers $adminHeaders -BaseUrl $OrderServiceUrl -Body @{ pos_instance_id = $PosInstanceId; rate_bps = $PosRateBps }

# Compute cases to demonstrate precedence
$computeHeaders = @{
  'Content-Type'  = 'application/json'
  'X-Tenant-ID'   = $TenantId
  'X-Roles'       = 'admin,cashier'
  'Authorization' = "Bearer $Token"
}
$computeUrl = ("{0}/orders/compute" -f $OrderServiceUrl.TrimEnd('/'))
$computeBodyBase = @{ items = @(@{ product_id = $productId; quantity = 1 }); discount_percent_bp = 0 }

$rate1 = [math]::Round($TenantRateBps / 100.0, 2)
Step "Case 1: Tenant-only (expect $rate1% rate)"
$body1 = $computeBodyBase
if ($script:usingFallback) { $body1 = @{ items = $computeBodyBase.items; discount_percent_bp = 0; tax_rate_bps = $TenantRateBps } }
$comp1 = Invoke-RestMethod -Method Post -Uri $computeUrl -Headers $computeHeaders -Body ($body1 | ConvertTo-Json -Depth 5)
Write-Host ("Tenant-only -> Subtotal={0}  Tax={1}  Total={2}" -f $comp1.subtotal_cents, $comp1.tax_cents, $comp1.total_cents)

$rate2 = [math]::Round($LocationRateBps / 100.0, 2)
Step "Case 2: Location override applies (expect $rate2% rate)"
$body2Obj = @{ items = $computeBodyBase.items; discount_percent_bp = $computeBodyBase.discount_percent_bp; location_id = $LocationId }
if ($script:usingFallback) { $body2Obj.tax_rate_bps = $LocationRateBps }
$comp2 = Invoke-RestMethod -Method Post -Uri $computeUrl -Headers $computeHeaders -Body ($body2Obj | ConvertTo-Json -Depth 5)
Write-Host ("Location -> Subtotal={0}  Tax={1}  Total={2}" -f $comp2.subtotal_cents, $comp2.tax_cents, $comp2.total_cents)

$rate3 = [math]::Round($PosRateBps / 100.0, 2)
Step "Case 3: POS override wins (expect $rate3% rate)"
$body3Obj = @{ items = $computeBodyBase.items; discount_percent_bp = $computeBodyBase.discount_percent_bp; location_id = $LocationId; pos_instance_id = $PosInstanceId }
if ($script:usingFallback) { $body3Obj.tax_rate_bps = $PosRateBps }
$comp3 = Invoke-RestMethod -Method Post -Uri $computeUrl -Headers $computeHeaders -Body ($body3Obj | ConvertTo-Json -Depth 5)
Write-Host ("POS -> Subtotal={0}  Tax={1}  Total={2}" -f $comp3.subtotal_cents, $comp3.tax_cents, $comp3.total_cents)

Step "Done"
