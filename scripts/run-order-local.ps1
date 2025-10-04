$ErrorActionPreference = 'Stop'

# Local dev environment variables
$env:DATABASE_URL = "postgres://novapos:novapos@localhost:5432/novapos"
$env:JWT_ISSUER   = "https://auth.novapos.local"
$env:JWT_AUDIENCE = "novapos-frontend,novapos-admin,novapos-postgres"

# Load dev public key for local JWT verification (no JWKS fetch)
$rootPath = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$pubKeyPath = Join-Path $rootPath 'jwt-dev.pub.pem'
if (Test-Path $pubKeyPath) {
    $env:JWT_DEV_PUBLIC_KEY_PEM = Get-Content -Raw $pubKeyPath
}

$servicesPath = Join-Path $rootPath 'services'
$features = $env:ORDER_FEATURES
Push-Location $servicesPath
try {
    if ([string]::IsNullOrWhiteSpace($features)) {
        cargo run -p order-service --no-default-features
    } else {
        cargo run -p order-service --no-default-features --features $features
    }
} finally {
    Pop-Location
}
