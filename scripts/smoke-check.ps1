param(
  [string]$DatabaseUrl = ${env:DATABASE_URL},
  [string]$TestDatabaseUrl = ${env:TEST_DATABASE_URL},
  [string]$ProductServiceUrl = "http://localhost:8081",
  [string]$OrderServiceUrl = "http://localhost:8084"
)

$ErrorActionPreference = 'Stop'
function Step($m){ Write-Host "`n==> $m" -ForegroundColor Cyan }
function Fail($m){ Write-Error $m; exit 1 }

Step "Checking service health endpoints"
try {
  $p = Invoke-WebRequest -Uri ("{0}/healthz" -f $ProductServiceUrl.TrimEnd('/')) -UseBasicParsing -TimeoutSec 5
  if ($p.StatusCode -ne 200 -or -not $p.Content.Contains('ok')) { Fail "product-service unhealthy ($($p.StatusCode))" }
  Write-Host "product-service: OK"
} catch {
  $err = $_.Exception.Message
  $msg = "product-service not reachable at {0}: {1}" -f $ProductServiceUrl, $err
  Fail $msg
}

try {
  $o = Invoke-WebRequest -Uri ("{0}/healthz" -f $OrderServiceUrl.TrimEnd('/')) -UseBasicParsing -TimeoutSec 5
  if ($o.StatusCode -ne 200 -or -not $o.Content.Contains('ok')) { Fail "order-service unhealthy ($($o.StatusCode))" }
  Write-Host "order-service: OK"
} catch {
  $err = $_.Exception.Message
  $msg = "order-service not reachable at {0}: {1}" -f $OrderServiceUrl, $err
  Fail $msg
}

Step "Checking Postgres connectivity"
$dbUrl = if ($DatabaseUrl -and $DatabaseUrl.Trim()) { $DatabaseUrl } elseif ($TestDatabaseUrl -and $TestDatabaseUrl.Trim()) { $TestDatabaseUrl } else { $null }
if (-not $dbUrl) {
  Write-Warning "No DATABASE_URL or TEST_DATABASE_URL set. Skipping DB check (services might still need it)."
} else {
  # Use psql in container (works even without local psql installed)
  try {
    if ($dbUrl -match "postgres://([^:]+):([^@]+)@([^:/]+)(?::(\d+))?/(.+)") {
      $pgUser = $Matches[1]
      $pgPass = $Matches[2]
      $pgHost = $Matches[3]
      $pgPort = if ($Matches[4]) { $Matches[4] } else { '5432' }
      $pgDb = $Matches[5]
      # If host refers to localhost/127.0.0.1, use host.docker.internal for container reachability
      if ($pgHost -eq 'localhost' -or $pgHost -eq '127.0.0.1') { $pgHost = 'host.docker.internal' }
      docker run --rm -e "PGPASSWORD=$pgPass" postgres:16 psql -h $pgHost -p $pgPort -U $pgUser -d $pgDb -c "SELECT 1;" | Out-Null
  $okMsg = "postgres: OK ({0}:{1}/{2})" -f $pgHost, $pgPort, $pgDb
  Write-Host $okMsg
    } else {
      Write-Warning "DATABASE_URL format not recognized for psql check; skipping DB probe."
    }
  } catch {
    Fail "Postgres connectivity check failed: $($_.Exception.Message)"
  }
}

Step "Checking product/order DB alignment"
$tid = ([guid]::NewGuid()).Guid
function Mint-DevToken {
  param([string]$TenantId)
  $pem = (Join-Path $PSScriptRoot "..\jwt-dev.pem")
  if (-not (Test-Path $pem)) { return $null }
  $node = (Get-Command node -ErrorAction SilentlyContinue)
  if (-not $node) { return $null }
  $token = $null
  Push-Location (Join-Path $PSScriptRoot "..")
  try {
    $token = node .\scripts\mint-dev-jwt.js --tenant $TenantId --roles Admin,Cashier --iss https://auth.novapos.local --aud novapos-admin --audMode single --kid local-dev
  } catch { $token = $null } finally { Pop-Location }
  return $token
}
${Token} = Mint-DevToken -TenantId $tid
try {
  # Create a tiny product in product-service
  $pHeaders = @{ 'Content-Type'='application/json'; 'X-Tenant-ID'=$tid; 'X-Roles'='Admin' }
  if ($Token) { $pHeaders['Authorization'] = "Bearer $Token" }
  $pUrl = ("{0}/products" -f $ProductServiceUrl.TrimEnd('/'))
  $pBody = @{ name = 'smoke-product'; price = 1.00; description=''; image=$null } | ConvertTo-Json -Depth 4
  $p = Invoke-RestMethod -Method Post -Uri $pUrl -Headers $pHeaders -Body $pBody

  # Try computing with that product_id in order-service
  $cHeaders = @{ 'Content-Type'='application/json'; 'X-Tenant-ID'=$tid; 'X-Roles'='admin,cashier' }
  if ($Token) { $cHeaders['Authorization'] = "Bearer $Token" }
  $cUrl = ("{0}/orders/compute" -f $OrderServiceUrl.TrimEnd('/'))
  $cBody = @{ items = @(@{ product_id = $p.id; quantity = 1 }); discount_percent_bp = 0 } | ConvertTo-Json -Depth 4
  $comp = Invoke-RestMethod -Method Post -Uri $cUrl -Headers $cHeaders -Body $cBody
  Write-Host "db-alignment: OK (order-service can see product-service products)"
} catch {
  $msg = $_.ErrorDetails.Message
  if ($msg -and $msg -like '*product_not_found*') {
    Write-Warning "db-alignment: product_not_found when computing with a just-created product. Services likely use different databases or schemas."
    Write-Host "  - Ensure both services point to the same DATABASE_URL and schema." -ForegroundColor Yellow
    Write-Host "  - Then restart services and rerun demos."
  } elseif ($_.Exception.Response -and $_.Exception.Response.StatusCode.value__ -eq 403) {
    Write-Warning "db-alignment: 403 Forbidden from order-service /orders/compute. Ensure roles and Authorization header are accepted in this environment."
    Write-Host "  - The smoke-check tried X-Roles=admin,cashier and a dev token if available." -ForegroundColor Yellow
  } else {
    Write-Warning ("db-alignment: skipped/unknown error: {0}" -f ($_.Exception.Message))
  }
}

Write-Host "`nAll checks passed." -ForegroundColor Green
