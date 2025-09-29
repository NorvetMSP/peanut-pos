[CmdletBinding()]
param(
    [string]$DatabaseUrl = $env:DATABASE_URL,
    [string[]]$Services,
    [switch]$ResetDatabase,
    [switch]$SkipMigrations,
    [switch]$SkipPrepare
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Invoke-Tool {
    param(
        [Parameter(Mandatory=$true)][string]$Command,
        [string[]]$Arguments = @(),
        [string]$ErrorContext
    )

    & $Command @Arguments
    if ($LASTEXITCODE -ne 0) {
        $context = if ($ErrorContext) { $ErrorContext } else { "$Command $($Arguments -join ' ')" }
        throw "'$context' exited with code $LASTEXITCODE"
    }
}

if (-not (Get-Command sqlx -ErrorAction SilentlyContinue)) {
    throw "sqlx CLI not found on PATH. Install with 'cargo install sqlx-cli'."
}

if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    throw "cargo is required but was not found on PATH."
}

if (-not $DatabaseUrl) {
    $DatabaseUrl = "postgres://novapos:novapos@localhost:5432/novapos"
}

$env:DATABASE_URL = $DatabaseUrl
if ($env:SQLX_OFFLINE) {
    Remove-Item Env:SQLX_OFFLINE
}

$repoRoot = Split-Path -Parent $MyInvocation.MyCommand.Definition
$servicesRoot = Join-Path $repoRoot "services"
if (-not (Test-Path $servicesRoot)) {
    throw "Unable to locate services directory at '$servicesRoot'."
}

$defaultOrder = @(
    "product-service",
    "order-service",
    "auth-service",
    "inventory-service",
    "customer-service",
    "loyalty-service",
    "analytics-service",
    "integration-gateway",
    "payment-service"
)

$metadataCommand = @(
    "metadata",
    "--manifest-path", (Join-Path $servicesRoot "Cargo.toml"),
    "--no-deps",
    "--format-version", "1"
)
$metadataJson = & cargo @metadataCommand
if ($LASTEXITCODE -ne 0) {
    throw "Failed to read cargo metadata"
}
$metadata = $metadataJson | ConvertFrom-Json

$packageMap = @{}
foreach ($pkg in $metadata.packages) {
    $pkgDir = Split-Path $pkg.manifest_path -Parent
    if (-not $pkgDir.StartsWith($servicesRoot, [System.StringComparison]::OrdinalIgnoreCase)) {
        continue
    }
    $binTargets = @($pkg.targets | Where-Object { $_.kind -contains 'bin' } | ForEach-Object { $_.name })
    if ($binTargets.Count -eq 0) {
        continue
    }
    $packageMap[$pkg.name] = [ordered]@{
        Path = $pkgDir
        Bins = $binTargets
    }
}

if ($Services -and $Services.Count -gt 0) {
    $servicesToProcess = $Services
} else {
    $servicesToProcess = $defaultOrder + ($packageMap.Keys | Where-Object { $defaultOrder -notcontains $_ })
}
$servicesToProcess = $servicesToProcess | Where-Object { $packageMap.ContainsKey($_) } | Select-Object -Unique
if ($servicesToProcess.Count -eq 0) {
    Write-Warning "No matching services to process."
    exit 0
}

Write-Host "Using DATABASE_URL = $DatabaseUrl"

if ($ResetDatabase) {
    Write-Host "Resetting database..."
    try {
        $uri = [System.Uri]$DatabaseUrl
        $dbName = $uri.AbsolutePath.TrimStart('/')
        if ([string]::IsNullOrWhiteSpace($dbName)) { throw 'Database name missing from DATABASE_URL' }
        $userInfo = $uri.UserInfo
        $dbHost = $uri.Host
        $port = if ($uri.Port -gt 0) { $uri.Port } else { 5432 }
        $adminUrl = "postgres://{0}@{1}:{2}/postgres" -f $userInfo, $dbHost, $port
        $dropSql = "DROP DATABASE IF EXISTS ""$dbName"" WITH (FORCE);"
        Invoke-Tool -Command "psql" -Arguments @('-d', $adminUrl, '-c', $dropSql) -ErrorContext 'psql drop database'
    } catch {
        Write-Warning $_.Exception.Message
    }
    Invoke-Tool -Command "sqlx" -Arguments @("database", "create") -ErrorContext "sqlx database create"
} else {
    Write-Host "Ensuring database exists..."
    Invoke-Tool -Command "sqlx" -Arguments @("database", "create") -ErrorContext "sqlx database create"
}

$failures = @()
foreach ($serviceName in $servicesToProcess) {
    $info = $packageMap[$serviceName]
    Write-Host "`n=== $serviceName ==="
    Push-Location $info.Path
    try {
        if (-not $SkipMigrations -and (Test-Path "migrations")) {
            Write-Host "  Running migrations..."
            Invoke-Tool -Command "sqlx" -Arguments @("migrate", "run", "--ignore-missing") -ErrorContext "sqlx migrate run ($serviceName)"
        } elseif ($SkipMigrations) {
            Write-Host "  Skipping migrations (per flag)."
        } else {
            Write-Host "  No migrations directory; skipping."
        }

        if (-not $SkipPrepare) {
            foreach ($bin in $info.Bins) {
                Write-Host "  Preparing queries for '$bin'..."
                Invoke-Tool -Command "cargo" -Arguments @("sqlx", "prepare", "--", "--bin", $bin) -ErrorContext ("cargo sqlx prepare ({0}::{1})" -f $serviceName, $bin)
            }
        } else {
            Write-Host "  Skipping cargo sqlx prepare (per flag)."
        }
    } catch {
        $failures += ("{0}: {1}" -f $serviceName, $_.Exception.Message)
        Write-Warning "  Failed: $($_.Exception.Message)"
    } finally {
        Pop-Location
    }
}

if ($failures.Count -gt 0) {
    Write-Error ("SQLx refresh completed with errors:`n - " + ($failures -join "`n - "))
    exit 1
}

Write-Host "`nAll services processed successfully."
