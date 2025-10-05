param(
  [string]$TenantId,
  [string]$ProductServiceUrl = "http://localhost:8081",
  [string]$OrderServiceUrl = "http://localhost:8084",
  [string]$DatabaseUrl = $env:DATABASE_URL,
  [string]$TestDatabaseUrl = $env:TEST_DATABASE_URL,
  [string]$JwtIssuer = "https://auth.novapos.local",
  [string]$JwtAudience = "novapos-admin",
  [string]$JwtPemPath = (Join-Path $PSScriptRoot "..\jwt-dev.pem"),
  [string]$Kid = "local-dev",
  [string]$Token
)

$ErrorActionPreference = 'Stop'
function Step($m){ Write-Host "`n==> $m" -ForegroundColor Cyan }
function Info($m){ Write-Host $m -ForegroundColor Gray }
function Warn($m){ Write-Warning $m }
function Fail($m){ Write-Error $m; exit 1 }

# Ensure a tenant id for the session
if (-not $TenantId -or [string]::IsNullOrWhiteSpace($TenantId)) {
  $TenantId = ([guid]::NewGuid()).Guid
  Info "Generated TenantId: $TenantId"
}

# 1) Smoke check
Step "Running smoke-check"
try {
  & (Join-Path $PSScriptRoot 'smoke-check.ps1') -DatabaseUrl $DatabaseUrl -TestDatabaseUrl $TestDatabaseUrl -ProductServiceUrl $ProductServiceUrl -OrderServiceUrl $OrderServiceUrl
} catch {
  Fail "Smoke-check failed: $($_.Exception.Message)"
}

Step "Demo selection"
Write-Host "TenantId: $TenantId"
Write-Host "ProductService: $ProductServiceUrl  |  OrderService: $OrderServiceUrl"
Write-Host ""
Write-Host "Choose a demo to run:" -ForegroundColor Cyan
Write-Host "  [1] Compute with header + DB override (try-compute.ps1)"
Write-Host "  [2] Tax precedence (tenant/location/POS) (try-precedence.ps1)"
Write-Host "  [3] Seed SKUs and (optionally) create order (seed-skus-and-order.ps1)"
Write-Host "  [4] Exit"

$choice = Read-Host "Enter selection [1-4]"
switch ($choice) {
  '1' {
    Step "Running try-compute.ps1"
    & (Join-Path $PSScriptRoot 'try-compute.ps1') -TenantId $TenantId -ProductServiceUrl $ProductServiceUrl -OrderServiceUrl $OrderServiceUrl -JwtIssuer $JwtIssuer -JwtAudience $JwtAudience -JwtPemPath $JwtPemPath -Kid $Kid -Token $Token
  }
  '2' {
    $tenantRate = Read-Host "Tenant rate bps (default 700)"; if (-not $tenantRate) { $tenantRate = 700 }
    $locRate = Read-Host "Location rate bps (default 800)"; if (-not $locRate) { $locRate = 800 }
    $posRate = Read-Host "POS rate bps (default 900)"; if (-not $posRate) { $posRate = 900 }
    Step "Running try-precedence.ps1"
    & (Join-Path $PSScriptRoot 'try-precedence.ps1') -TenantId $TenantId -ProductServiceUrl $ProductServiceUrl -OrderServiceUrl $OrderServiceUrl -TenantRateBps ([int]$tenantRate) -LocationRateBps ([int]$locRate) -PosRateBps ([int]$posRate) -Token $Token
  }
  '3' {
    $create = Read-Host "Create order after compute? (y/N)"; $doCreate = $false; if ($create -match '^(y|yes)$') { $doCreate = $true }
    $payMethod = Read-Host "Payment method (cash/card) [default cash]"; if (-not $payMethod) { $payMethod = 'cash' }
    $disc = Read-Host "Discount percent (bps) [default 1000 = 10%]"; if (-not $disc) { $disc = 1000 }
    $headerTax = Read-Host "Header tax rate (bps) [default 800]"; if (-not $headerTax) { $headerTax = 800 }
    Step "Running seed-skus-and-order.ps1"
    $splat = @{
      TenantId = $TenantId
      ProductServiceUrl = $ProductServiceUrl
      OrderServiceUrl = $OrderServiceUrl
      DiscountPercentBp = [int]$disc
      HeaderTaxBps = [int]$headerTax
      PaymentMethod = $payMethod
    }
    if ($doCreate) { $splat.CreateOrder = $true }
    if ($Token) { $splat.Token = $Token }
    & (Join-Path $PSScriptRoot 'seed-skus-and-order.ps1') @splat
  }
  default { Info "Exit requested." }
}

Write-Host "`nDone." -ForegroundColor Green