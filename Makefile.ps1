<#
.SYNOPSIS
  PowerShell task runner for NovaPOS monorepo.
  Usage examples:
    ./Makefile.ps1 Build-Services
    ./Makefile.ps1 Run-Service -Name order-service
    ./Makefile.ps1 Dev-Frontend -App pos-app
#>

param(
    [Parameter(Position=0, Mandatory=$true)]
    [ValidateSet("Start-Infra","Stop-Infra","Build-Services","Test-Services","Run-Service","Dev-Frontend","Lint-Frontend","Audit-Coverage")]
    [string]$Task,
    [string]$Name,
    [string]$App
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Start-Infra {
    Write-Host "Starting local infra (Postgres, Redis, Kafka) with Docker Compose..."
    docker compose -f local/docker-compose/docker-compose.yml up -d
}

function Stop-Infra {
    Write-Host "Stopping local infra..."
    docker compose -f local/docker-compose/docker-compose.yml down -v
}

function Build-Services {
    Write-Host "Building all Rust microservices..."
    Push-Location services
    try {
        cargo build --workspace
    } finally {
        Pop-Location
    }
}

function Test-Services {
    Write-Host "Running all Rust tests..."
    Push-Location services
    try {
        cargo test --workspace
    } finally {
        Pop-Location
    }
}

function Run-Service {
    param([string]$Name)

    if (-not $Name) {
        Write-Error "Please provide a service name. Example: ./Makefile.ps1 Run-Service -Name order-service"
        exit 1
    }

    $svcPath = "services/$Name"
    if (-not (Test-Path $svcPath)) {
        Write-Error "Service '$Name' not found in services/"
        exit 1
    }

    Write-Host "Running $Name..."
    Push-Location $svcPath
    try {
        cargo run
    } finally {
        Pop-Location
    }
}

function Dev-Frontend {
    param([string]$App)

    if (-not $App) {
        Write-Error "Please provide frontend app (e.g., pos-app or admin-portal)."
        exit 1
    }

    $appPath = "frontends/$App"
    if (-not (Test-Path $appPath)) {
        Write-Error "Frontend '$App' not found in frontends/"
        exit 1
    }

    Write-Host "Starting frontend: $App..."
    Push-Location $appPath
    try {
        npm install
        npm run dev
    } finally {
        Pop-Location
    }
}

function Lint-Frontend {
    Write-Host "Linting frontends..."
    if (-not (Test-Path "frontends")) {
        Write-Host "No frontends directory found."
        return
    }
    $frontendDirs = Get-ChildItem -Directory frontends | Select-Object -ExpandProperty Name
    foreach ($dir in $frontendDirs) {
        $appPath = "frontends/$dir"
        if (Test-Path "$appPath/eslint.config.js") {
            Write-Host "Linting $dir..."
            Push-Location $appPath
            try {
                npx eslint "." --quiet
            } finally {
                Pop-Location
            }
        } else {
            Write-Host "Skipping $dir (no eslint.config.js)"
        }
    }
}

function Audit-Coverage {
    Write-Host "Running audit coverage scanner..."
    Push-Location services
    try {
        cargo run -p audit-coverage -- --root . --json ../audit_coverage_report.json --metrics-out ../audit_coverage_metrics.prom --min-ratio 90
    } finally {
        Pop-Location
    }
    if (Test-Path audit_coverage_report.json) {
        Write-Host "Coverage report written to services/../audit_coverage_report.json"
    }
    if (Test-Path audit_coverage_metrics.prom) {
        Write-Host "Prometheus metrics written to services/../audit_coverage_metrics.prom"
    }
}

switch ($Task) {
    "Start-Infra"    { Start-Infra }
    "Stop-Infra"     { Stop-Infra }
    "Build-Services" { Build-Services }
    "Test-Services"  { Test-Services }
    "Run-Service"    { Run-Service -Name $Name }
    "Dev-Frontend"   { Dev-Frontend -App $App }
    "Lint-Frontend"  { Lint-Frontend }
    "Audit-Coverage" { Audit-Coverage }
    default { Write-Error "Unknown task '$Task'." }
}

