<#
Regenerates SQLx offline metadata (.sqlx/query-*.json) per service.

Features:
  - Optional database reset & migration checksum auto-reset
  - Per-service feature filtering
  - Pruning (-Prune) to drop stale query metadata by deleting .sqlx before regeneration
  - Query counting & warnings for zero captured queries
  - FailOnZero to enforce macro coverage

Note: Legacy merged sqlx-data.json generation removed; per-query files are the canonical format.
#>

[CmdletBinding()]
param(
  [string[]] $Services,
  [switch]   $ResetDatabase,
  [switch]   $SkipMigrations,
  [switch]   $SkipPrepare,
  [switch]   $AutoResetOnChecksum,
  [string[]] $Features,
  [switch]   $Prune,
  [switch]   $FailOnZero,
  [switch]   $Diff,
  [switch]   $FailOnDrift,
  [string]   $ReportPath = "sqlx-query-report.json"
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

function Get-DbNameFromUrl {
  param([string] $Url)
  # Expect format postgres://user:pass@host:port/dbname[?params]
  if (-not $Url) { return $null }
  try {
    $uri = [System.Uri]::new($Url)
    $db = $uri.AbsolutePath.Trim('/')
    if ($db -match '\?') { $db = $db.Split('?')[0] }
    return $db
  } catch { return $null }
}

function Invoke-TerminateConnections {
  param([string] $DbName, [string] $Url)
  if (-not (Get-Command psql -ErrorAction SilentlyContinue)) {
    Write-Warning "psql not found; skipping connection termination."
    return
  }
  try {
    $uri = [System.Uri]::new($Url)
  $pgHost = $uri.Host
  $port = if ($uri.Port -gt 0) { $uri.Port } else { 5432 }
    $userInfo = $uri.UserInfo
    $user = $null; $pass = $null
    if ($userInfo) {
      $parts = $userInfo.Split(':',2)
      $user = $parts[0]
      if ($parts.Count -gt 1) { $pass = $parts[1] }
    }
    if (-not $user) { $user = $env:PGUSER }
    if (-not $user) { $user = 'postgres' }
    if ($pass) { $env:PGPASSWORD = $pass }
  Write-Host "Terminating active connections to $DbName (host=$pgHost port=$port user=$user)..."
  & psql -h $pgHost -p $port -U $user -d postgres -c "SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE datname = '$DbName' AND pid <> pg_backend_pid();" 2>$null | Out-Null
  } catch {
    Write-Warning "Failed to terminate connections via psql: $($_.Exception.Message)"
  } finally {
    if ($env:PGPASSWORD) { Remove-Item Env:PGPASSWORD -ErrorAction SilentlyContinue }
  }
}

function Invoke-DropAndCreateDatabase {
  param([string] $DbName)
  $maxAttempts = 5
  for ($i=1; $i -le $maxAttempts; $i++) {
    Invoke-TerminateConnections -DbName $DbName -Url $env:DATABASE_URL
    try {
      Write-Host ("Attempt {0}: dropping database {1} ..." -f $i, $DbName)
      sqlx database drop -y 2>&1 | ForEach-Object { $_ | Write-Host }
      if ($LASTEXITCODE -ne 0) {
        $dropOk = $false
        Write-Warning "sqlx database drop exited with code $LASTEXITCODE"
      } else {
        $dropOk = $true
      }
    } catch {
      $dropOk = $false
      Write-Warning "drop attempt $i failed: $($_.Exception.Message)"
    }
    if ($dropOk) { break }
    Start-Sleep -Seconds 1
  }
  if (-not $dropOk) {
    Write-Error "Unable to drop database $DbName after $maxAttempts attempts. Abort."
    exit 10
  }
  Write-Host "Creating database $DbName ..."
  sqlx database create | Out-Null
  Write-Host "Database recreated."
}

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
  Push-Location $dir | Out-Null
  try {
    $info = sqlx migrate info --format json 2>$null | ConvertFrom-Json
    if (-not $info) { return $true }
    $bad = $info | Where-Object { $_.applied -eq $true -and $_.checksum_ok -eq $false }
    return -not ($bad)
  } catch { return $true } finally { Pop-Location | Out-Null }
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
  Write-Host "ResetDatabase: force drop & recreate..."
  $dbName = Get-DbNameFromUrl -Url $env:DATABASE_URL
  if (-not $dbName) { Write-Error "Could not parse database name from DATABASE_URL"; exit 9 }
  Invoke-DropAndCreateDatabase -DbName $dbName
}

# Run migrations
if (-not $SkipMigrations) {
  $migrationFailures = @()
  foreach ($svc in $Services) {
    $svcPath = Join-Path services $svc
    $migPath = Join-Path $svcPath migrations
    if (Test-Path $migPath) {
      Write-Host "[migrate] $svc"
      Push-Location $svcPath | Out-Null
      try {
        sqlx migrate run --ignore-missing
        if ($LASTEXITCODE -ne 0) {
          Write-Error "[migrate] $svc failed with exit code $LASTEXITCODE"
          $migrationFailures += $svc
        }
      } catch {
        Write-Error "[migrate] $svc exception: $($_.Exception.Message)"
        $migrationFailures += $svc
      } finally {
        Pop-Location | Out-Null
      }
    } else {
      Write-Host "[migrate] $svc (no migrations directory)"
    }
  }
  if ($migrationFailures.Count -gt 0) {
    Write-Error "One or more migrations failed (possible modified checksum): $($migrationFailures -join ', ')";
    Write-Host "Aborting before prepare. Resolve by:"
    Write-Host "  1. Reverting unintended changes to applied migration files OR"
    Write-Host "  2. Creating a new follow-on migration instead of editing an applied one OR"
    Write-Host "  3. (Dev only) Resetting the database with -ResetDatabase or -AutoResetOnChecksum"
    exit 3
  }
} else {
  Write-Host "Skipping migrations."
}

if ($SkipPrepare) {
  Write-Host "SkipPrepare set: skipping offline metadata generation."
  exit 0
}

$failures = @()
$_queryCountReport = @{}
$_serviceClassification = @{}
$_diffAdded = @{}
$_diffRemoved = @{}

# Capture baseline if diff requested
$baseline = @{}
if ($Diff) {
  foreach ($svc in $Services) {
    $svcDir = Join-Path services $svc
    $qDir = Join-Path $svcDir '.sqlx'
    if (Test-Path $qDir) {
      $existing = Get-ChildItem $qDir -Filter 'query-*.json' -ErrorAction SilentlyContinue | ForEach-Object { $_.Name }
      $baseline[$svc] = $existing
    } else {
      $baseline[$svc] = @()
    }
  }
}
foreach ($svc in $Services) {
  $svcDir = Join-Path services $svc
  if (-not (Test-Path $svcDir)) {
    Write-Warning "[prepare] $svc directory missing"
    $failures += $svc
    continue
  }
  Write-Host "[prepare] $svc"
  Push-Location $svcDir | Out-Null
  # Remove legacy merged file if present (we no longer maintain it)
  if (Test-Path sqlx-data.json) { Remove-Item sqlx-data.json -Force }
  # If pruning, remove entire .sqlx directory so only current queries are regenerated
  if ($Prune -and (Test-Path .sqlx)) {
    Write-Host "[prune] removing existing .sqlx directory to drop stale query metadata"
    Remove-Item .sqlx -Recurse -Force -ErrorAction SilentlyContinue
  }
  try {
    # Classify service by scanning source files for macro or runtime sqlx usage
    $hasMacro = $false; $hasRuntime = $false; $hasSqlxDep = $false
    $cargoTomlPath = Join-Path $svcDir 'Cargo.toml'
    if (Test-Path $cargoTomlPath) {
      $cargoContent = Get-Content $cargoTomlPath -Raw
      if ($cargoContent -match "(?m)^sqlx\s*=") { $hasSqlxDep = $true }
    }
    $srcDir = Join-Path $svcDir 'src'
    if (Test-Path $srcDir) {
      $rsFiles = Get-ChildItem $srcDir -Recurse -Include *.rs -ErrorAction SilentlyContinue
      foreach ($f in $rsFiles) {
        try {
          $text = Get-Content $f.FullName -Raw
          if (-not $hasMacro -and $text -match 'query!?[_a-z]*!\(') { $hasMacro = $true }
          # runtime calls: sqlx::query(  or sqlx::query_as( etc but NOT macro ! forms
          if (-not $hasRuntime -and $text -match 'sqlx::query(?![_a-zA-Z0-9]*!)\(') { $hasRuntime = $true }
          if ($hasMacro -and $hasRuntime) { break }
        } catch { }
      }
    }
    $classification = if ($hasMacro) { 'macro' } elseif ($hasRuntime -and $hasSqlxDep) { 'runtime-only' } elseif ($hasSqlxDep) { 'no-macro' } else { 'no-sqlx' }
    $_serviceClassification[$svc] = $classification
    # Assume bin name == service folder; adjust if any differs.
    $postArgs = @('--all-targets')
    if ($Features -and $Features.Count -gt 0) {
      $cargoToml = Join-Path $svcDir 'Cargo.toml'
      $declared = @()
      if (Test-Path $cargoToml) {
        try {
          $content = Get-Content $cargoToml -Raw
          if ($content -match '(?s)\[features\](.+?)(\n\[|$)') {
            $block = $Matches[1]
            $declared = ($block -split "`n") | ForEach-Object { ($_ -split '=')[0].Trim() } | Where-Object { $_ -match '^[A-Za-z0-9_-]+$' }
          }
        } catch { Write-Warning "[prepare] $svc unable to parse features: $($_.Exception.Message)" }
      }
      $svcFeatures = @()
      foreach ($f in $Features) {
        $fname = $f.Trim()
        if ($fname -and ($declared -contains $fname)) { $svcFeatures += $fname }
      }
      if ($svcFeatures.Count -gt 0) {
        $joined = $svcFeatures -join ','
        Write-Host "[prepare] service features applied: $joined"
        $postArgs = @('--features', $joined) + $postArgs
      } else {
        Write-Host "[prepare] no matching requested features for $svc; skipping feature flags"
      }
    }
    # Run prepare allowing warning-only stderr without tripping Stop semantics; honor non-zero exit codes.
    $previousErrorActionPreference = $ErrorActionPreference
    try {
      $ErrorActionPreference = 'Continue'
      & cargo sqlx prepare -- @postArgs
    } finally {
      $ErrorActionPreference = $previousErrorActionPreference
    }
    $prepareExitCode = $LASTEXITCODE
    if ($prepareExitCode -ne 0) {
      throw "cargo sqlx prepare exited with code $prepareExitCode"
    }
    if (-not (Test-Path .sqlx)) {
      if ($classification -in @('runtime-only','no-sqlx','no-macro')) {
        # Accept absence when no macro queries are expected
        $_queryCountReport[$svc] = 0
        Pop-Location | Out-Null
        continue
      } else {
        throw "sqlx offline artifact not produced (missing .sqlx)"
      }
    }
    try {
      $count = 0
      $files = Get-ChildItem .sqlx -Filter 'query-*.json' -ErrorAction SilentlyContinue
      if ($files) { $count = $files.Count }
      $_queryCountReport[$svc] = $count
      if ($count -eq 0 -and $_serviceClassification[$svc] -eq 'macro') {
        Write-Warning "[prepare] $svc classified as macro but produced zero captured queries. Investigate feature flags or macro usage."
      } elseif ($count -eq 0 -and $_serviceClassification[$svc] -eq 'runtime-only') {
        Write-Host "[prepare] $svc runtime-only (no macro queries to capture)."
      }
    } catch {
      Write-Warning "[prepare] $svc unable to evaluate query count: $($_.Exception.Message)"
    }
  } catch {
    Write-Warning "[prepare] $svc failed: $($_.Exception.Message)"
    $failures += $svc
  } finally {
  Pop-Location | Out-Null
  }
}

if ($failures.Count -gt 0) {
  Write-Error "Offline prepare failed for: $($failures -join ', ')"
  exit 1
}

Write-Host "Offline query metadata generation complete."
Write-Host "Query entry counts:" 
$_queryCountReport.GetEnumerator() | Sort-Object Name | ForEach-Object { Write-Host ("  {0,-20} {1,4}" -f $_.Name, $_.Value) }

# Diff detection
if ($Diff) {
  Write-Host "Diff results:" 
  foreach ($svc in $Services) {
    $svcDir = Join-Path services $svc
    $qDir = Join-Path $svcDir '.sqlx'
    $after = @()
    if (Test-Path $qDir) { $after = Get-ChildItem $qDir -Filter 'query-*.json' -ErrorAction SilentlyContinue | ForEach-Object { $_.Name } }
    $before = if ($baseline.ContainsKey($svc)) { $baseline[$svc] } else { @() }
    $added = @($after | Where-Object { $before -notcontains $_ })
    $removed = @($before | Where-Object { $after -notcontains $_ })
    $_diffAdded[$svc] = $added
    $_diffRemoved[$svc] = $removed
    if ($added.Count -gt 0 -or $removed.Count -gt 0) {
      Write-Host ("  {0}: +{1} -{2}" -f $svc, $added.Count, $removed.Count)
    } else {
      Write-Host ("  {0}: no changes" -f $svc)
    }
  }
}

# Fail conditions
if ($FailOnZero) {
  $macroZero = ($_queryCountReport.GetEnumerator() | Where-Object { $_serviceClassification[$_.Key] -eq 'macro' -and $_.Value -eq 0 })
  if ($macroZero.Count -gt 0) {
    Write-Error "FailOnZero: macro-classified services with zero queries: $($macroZero.Key -join ', ')"; exit 11
  }
}
if ($FailOnDrift -and $Diff) {
  $hasDrift = $false
  foreach ($svc in $Services) { if (($_diffAdded[$svc].Count + $_diffRemoved[$svc].Count) -gt 0) { $hasDrift = $true; break } }
  if ($hasDrift) { Write-Error "FailOnDrift: offline metadata drift detected."; exit 12 }
}

# Build report JSON
$report = [ordered]@{
  generated_at = (Get-Date).ToString('o')
  database_url = $env:DATABASE_URL
  services = @{}
  diff = @{}
}
foreach ($svc in $Services) {
  $countVal = 0
  if ($_queryCountReport.ContainsKey($svc)) { $countVal = $_queryCountReport[$svc] }
  $addedList = @()
  $removedList = @()
  if ($Diff) { $addedList = $_diffAdded[$svc]; $removedList = $_diffRemoved[$svc] }
  $report.services[$svc] = [ordered]@{
    classification = $_serviceClassification[$svc]
    count = $countVal
    added = $addedList
    removed = $removedList
  }
}
if ($Diff) {
  $totalAdded = ($_diffAdded.Values | ForEach-Object { $_.Count } | Measure-Object -Sum).Sum
  $totalRemoved = ($_diffRemoved.Values | ForEach-Object { $_.Count } | Measure-Object -Sum).Sum
  $report.diff = [ordered]@{ total_added = $totalAdded; total_removed = $totalRemoved }
}
try {
  $report | ConvertTo-Json -Depth 6 | Out-File -FilePath $ReportPath -Encoding UTF8
  Write-Host "Report written to $ReportPath"
} catch {
  Write-Warning "Unable to write report JSON: $($_.Exception.Message)"
}
