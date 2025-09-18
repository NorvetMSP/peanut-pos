# Set the environment variable for this session
$env:DATABASE_URL = "postgres://novapos:novapos@localhost:5432/novapos"

# List of service directories
$services = @(
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

# Change to each service directory and run migrations
foreach ($service in $services) {
    $servicePath = "services\$service"
    if (Test-Path "$servicePath\migrations") {
        Write-Host "Migrating $service..."
        Push-Location $servicePath
        sqlx database create  # Safe to re-run, creates DB if missing
        sqlx migrate run --ignore-missing
        Pop-Location
    } else {
        Write-Host "Skipping $service (no migrations folder)"
    }
}
