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

Write-Host "`nAll checks passed." -ForegroundColor Green
