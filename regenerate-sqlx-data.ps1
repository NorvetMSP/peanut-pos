<# 
Regenerates SQLx offline metadata (sqlx-data.json) per service.
#>

[CmdletBinding()]
param(
  [string[]] $Services,
  [switch]   $ResetDatabase,
  [switch]   $SkipMigrations,
  [switch]   $SkipPrepare,
  [switch]   $AutoResetOnChecksum
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

if (-not $env:DATABASE_URL -or [string]::IsNullOrWhiteSpace($env:DATABASE_URL)) {
  $env:DATABASE_URL = "postgres://novapos:novapos@localhost:5432/novapos"
}
Write-Host "DATABASE_URL = $($env:DATABASE_URL)"

# Normalize service list (comma support)
if ($Services -and $Services.Count -eq 1 -and $Services[0] -match ',') {
  $Services = $Services[0].Split(',') | ForEach-Object { $_.Trim() } | Where-Object { $_ }
}
if (-not $Services -or $Services.Count -eq 0) {
  $Services = @(
    'auth-service','order-service','product-service','payment-service',
    'inventory-service','loyalty-service','customer-service',
    'analytics-service','integration-gateway'
  )
}
$Services = @($Services | ForEach-Object { $_.Trim() } | Where-Object { $_ }) | Sort-Object -Unique
Write-Host "Target services: $($Services -join ', ')"

Write-Host "Ensuring database exists..."
try { sqlx database create 2>$null | Out-Null } catch { Write-Warning "sqlx database create: $($_.Exception.Message)" }

function Test-MigrationHealth {
  param([string] $Service)
  $dir = Join-Path services $Service
  $migrations = Join-Path $dir migrations
  if (-not (Test-Path $migrations)) { return $true }
  pushd $dir | Out-Null
  try {
    $info = sqlx migrate info --format json 2>$null | ConvertFrom-Json
    if (-not $info) { return $true }
    $bad = $info | Where-Object { $_.applied -eq $true -and $_.checksum_ok -eq $false }
    return -not ($bad)
  } catch { return $true } finally { popd | Out-Null }
}

# Pre-migration checksum scan (only if we intend to run migrations and not already forcing a reset)
if (-not $SkipMigrations -and -not $ResetDatabase) {
  $needsReset = $false
  foreach ($svc in $Services) {
    if (-not (Test-MigrationHealth -Service $svc)) {
      Write-Warning "Checksum mismatch detected in $svc migrations."
      $needsReset = $true
    }
  }
  if ($needsReset) {
    if ($AutoResetOnChecksum) {
      Write-Host "AutoResetOnChecksum: enabling ResetDatabase."
      $ResetDatabase = $true
    } else {
      Write-Error "One or more checksum mismatches. Re-run with -ResetDatabase (or add -AutoResetOnChecksum)."
      exit 2
    }
  }
}

if ($ResetDatabase) {
  Write-Host "ResetDatabase: dropping active connections then recreating DB..."
  try {
    # psql may not exist; ignore failure
    psql "$($env:DATABASE_URL)" -c "SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE datname = current_database() AND pid <> pg_backend_pid();" 2>$null | Out-Null
  } catch { }
  sqlx database drop -y
  sqlx database create
  Write-Host "Database recreated."
}

# Run migrations
if (-not $SkipMigrations) {
  foreach ($svc in $Services) {
    $svcPath = Join-Path services $svc
    $migPath = Join-Path $svcPath migrations
    if (Test-Path $migPath) {
      Write-Host "[migrate] $svc"
      pushd $svcPath | Out-Null
      sqlx migrate run --ignore-missing
      popd | Out-Null
    } else {
      Write-Host "[migrate] $svc (no migrations directory)"
    }
  }
} else {
  Write-Host "Skipping migrations."
}

if ($SkipPrepare) {
  Write-Host "SkipPrepare set: skipping offline metadata generation."
  exit 0
}

$failures = @()
foreach ($svc in $Services) {
  $svcDir = Join-Path services $svc
  if (-not (Test-Path $svcDir)) {
    Write-Warning "[prepare] $svc directory missing"
    $failures += $svc
    continue
  }
  Write-Host "[prepare] $svc"
  pushd $svcDir | Out-Null
  if (Test-Path sqlx-data.json) { Remove-Item sqlx-data.json -Force }
  try {
    # Assume bin name == service folder; adjust if any differs.
    cargo sqlx prepare -- --bin $svc
    if (-not (Test-Path sqlx-data.json)) { throw "sqlx-data.json not produced" }
  } catch {
    Write-Warning "[prepare] $svc failed: $($_.Exception.Message)"
    $failures += $svc
  } finally {
    popd | Out-Null
  }
}

if ($failures.Count -gt 0) {
  Write-Error "Offline prepare failed for: $($failures -join ', ')"
  exit 1
}

Write-Host "All sqlx-data.json files generated successfully."