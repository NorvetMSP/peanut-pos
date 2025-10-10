param(
  [string]$TenantId,
  [string]$StoreId = 'store-001',
  [string]$OrderServiceUrl = 'http://localhost:8084'
)

$ErrorActionPreference = 'Stop'

if (-not $TenantId -or [string]::IsNullOrWhiteSpace($TenantId)) {
  if ($env:SEED_TENANT_ID -and -not [string]::IsNullOrWhiteSpace($env:SEED_TENANT_ID)) {
    $TenantId = $env:SEED_TENANT_ID
  } else {
    $TenantId = [guid]::NewGuid().ToString()
    $env:SEED_TENANT_ID = $TenantId
  }
}

# Mint a dev JWT
$token = node "$PSScriptRoot/mint-dev-jwt.js" --tenant $TenantId --roles Admin,Cashier --iss https://auth.novapos.local --aud novapos-frontend --audMode single --sub pos-agent --expMins 30
if (-not $token) { throw 'Failed to mint JWT (is Node installed?)' }

$headers = @{
  'Content-Type' = 'application/json'
  'X-Tenant-ID'  = $TenantId
  'Authorization' = "Bearer $token"
}

$payload = @{
  ts = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
  labels = @{ tenant_id = $TenantId; store_id = $StoreId }
  counters = @(
    @{ name = 'pos.print.retry.queued';  value = 2 }
    @{ name = 'pos.print.retry.failed';  value = 1 }
    @{ name = 'pos.print.retry.success'; value = 0 }
  )
  gauges = @(
    @{ name = 'pos.print.queue_depth';            value = 3 }
    @{ name = 'pos.print.retry.last_attempt';     value = 1200 }
  )
} | ConvertTo-Json -Depth 10

Write-Host "Posting telemetry for tenant $TenantId store $StoreId ..."
$resp = Invoke-RestMethod -Method Post -Uri "$OrderServiceUrl/pos/telemetry" -Headers $headers -Body $payload
$resp | Format-List | Out-String | Write-Host

Write-Host "Grepping /metrics for pos_print_* ..."
$metrics = (Invoke-WebRequest -Uri "$OrderServiceUrl/metrics").Content
($metrics -split "`n") | Where-Object { $_ -match 'pos_print_retry_total|pos_print_gauge' } | ForEach-Object { Write-Host $_ }
