param(
  [string]$TenantId = ([guid]::NewGuid()).Guid,
  [string]$ProductServiceUrl = "http://localhost:8081",
  [string]$OrderServiceUrl = "http://localhost:8084",
  [string]$JwtIssuer = "https://auth.novapos.local",
  [string]$JwtAudience = "novapos-admin",
  [string]$JwtPemPath = (Join-Path $PSScriptRoot "..\jwt-dev.pem"),
  [string]$Kid = "local-dev",
  [string]$Token
)

# Helper: write step header
function Step($msg) { Write-Host "`n==> $msg" -ForegroundColor Cyan }

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

# Create a sample product ($10.00)
$prodHeaders = @{
  'Content-Type' = 'application/json'
  'X-Tenant-ID'  = $TenantId
  'X-Roles'      = 'Admin'
  'Authorization'= "Bearer $Token"
}
$prodBody = @{ name = 'TryIt Item'; price = 10.00; description = 'demo item'; image = $null } | ConvertTo-Json -Depth 5
$prodUrl = ("{0}/products" -f $ProductServiceUrl.TrimEnd('/'))
Step "Creating product at $prodUrl"
try {
  $prodResp = Invoke-RestMethod -Method Post -Uri $prodUrl -Headers $prodHeaders -Body $prodBody
} catch {
  Write-Error "Failed to create product: $($_.Exception.Message)"; throw
}
$productId = $prodResp.id
Write-Host "Product created: id=$productId name=$($prodResp.name) price=$($prodResp.price)"

# Compute with header override tax (8%)
$computeHeaders = @{
  'Content-Type'   = 'application/json'
  'X-Tenant-ID'    = $TenantId
  'X-Roles'        = 'admin,cashier'
  'Authorization'  = "Bearer $Token"
  'x-tax-rate-bps' = '800'
}
$computeBody = @{ items = @(@{ product_id = $productId; quantity = 1 }); discount_percent_bp = 0 } | ConvertTo-Json -Depth 5
$computeUrl = ("{0}/orders/compute" -f $OrderServiceUrl.TrimEnd('/'))
Step "Computing totals with header x-tax-rate-bps=800"
try {
  $comp1 = Invoke-RestMethod -Method Post -Uri $computeUrl -Headers $computeHeaders -Body $computeBody
} catch {
  $resp = $_.Exception.Response
  if ($resp) {
    $code = $resp.StatusCode.value__
    $xcode = $resp.Headers['X-Error-Code']
    try {
      $stream = $resp.GetResponseStream(); $reader = New-Object System.IO.StreamReader($stream); $body = $reader.ReadToEnd(); $reader.Dispose(); $stream.Dispose()
    } catch { $body = '' }
    Write-Error ("Compute (header override) failed: HTTP {0} X-Error-Code={1} Body={2}" -f $code, ($xcode -join ','), $body)
  } else {
    Write-Error "Compute (header override) failed: $($_.Exception.Message)"
  }
  throw
}
Write-Host ("Subtotal: {0}  Tax: {1}  Total: {2}" -f $comp1.subtotal_cents, $comp1.tax_cents, $comp1.total_cents)

# Upsert tenant-level DB override (9%)
$adminHeaders = @{
  'Content-Type' = 'application/json'
  'X-Tenant-ID'  = $TenantId
  'X-Roles'      = 'Admin'
  'Authorization'= "Bearer $Token"
}
$upsertBody = @{ rate_bps = 900 } | ConvertTo-Json
$adminUrl = ("{0}/admin/tax_rate_overrides" -f $OrderServiceUrl.TrimEnd('/'))
# Fallback in case admin route is namespaced differently
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
      return Invoke-RestMethod -Method Post -Uri $alt -Headers $Headers -Body ($Body | ConvertTo-Json)
    }
    throw
  }
}
Step "Upserting tenant tax override rate_bps=900"
try {
  $null = Invoke-TaxOverrideUpsert -Headers $adminHeaders -BaseUrl $OrderServiceUrl -Body @{ rate_bps = 900 }
} catch {
  $resp = $_.Exception.Response
  if ($resp -and $resp.StatusCode.value__ -eq 404) {
    Write-Warning "Admin tax override endpoint not found (404). Falling back to request.tax_rate_bps demo."
    $fallback = $true
  } else {
    Write-Error "Upsert tax override failed: $($_.Exception.Message)"; throw
  }
}

# Compute again without header -> should use DB 9%
$computeHeaders2 = @{
  'Content-Type'   = 'application/json'
  'X-Tenant-ID'    = $TenantId
  'X-Roles'        = 'admin,cashier'
  'Authorization'  = "Bearer $Token"
}
Step "Computing totals using DB override (expect 9%)"
try {
  if ($fallback) {
    $computeBody2 = @{ items = @(@{ product_id = $productId; quantity = 1 }); discount_percent_bp = 0; tax_rate_bps = 900 } | ConvertTo-Json -Depth 5
    Step "Computing totals using request.tax_rate_bps (simulate DB 9%)"
    $comp2 = Invoke-RestMethod -Method Post -Uri $computeUrl -Headers $computeHeaders2 -Body $computeBody2
  } else {
    $comp2 = Invoke-RestMethod -Method Post -Uri $computeUrl -Headers $computeHeaders2 -Body $computeBody
  }
} catch {
  Write-Error "Compute (DB override) failed: $($_.Exception.Message)"; throw
}
Write-Host ("Subtotal: {0}  Tax: {1}  Total: {2}" -f $comp2.subtotal_cents, $comp2.tax_cents, $comp2.total_cents)

Step "Done"
