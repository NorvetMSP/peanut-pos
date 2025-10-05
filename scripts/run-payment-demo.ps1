param(
  [string]$Roles = 'Admin,Cashier',
  [string]$Issuer = 'https://auth.novapos.local',
  [string]$Audience = 'novapos-admin',
  [int]$ExpMins = 30,
  [string]$KeyId = 'local-dev'
)

$ErrorActionPreference = 'Stop'

function Test-ToolAvailable {
  param([string]$Tool)
  $cmd = Get-Command $Tool -ErrorAction SilentlyContinue
  if (-not $cmd) {
    throw "Required tool not found: $Tool"
  }
}

Test-ToolAvailable -Tool 'node'

$TenantId = if ($env:SEED_TENANT_ID -and -not [string]::IsNullOrWhiteSpace($env:SEED_TENANT_ID)) { $env:SEED_TENANT_ID } else { [guid]::NewGuid().ToString() }
$env:SEED_TENANT_ID = $TenantId

function Try-RunDemoWithAudience {
  param([string]$Aud)
  Write-Host "Minting dev JWT for tenant $TenantId with roles $Roles and aud '$Aud' ..."
  $tok = node "$PSScriptRoot/mint-dev-jwt.js" --tenant $TenantId --roles $Roles --iss $Issuer --aud $Aud --audMode single --sub dev-admin --expMins $ExpMins --kid $KeyId
  if (-not $tok -or [string]::IsNullOrWhiteSpace($tok)) { throw 'Failed to mint JWT token.' }
  $env:AUTH_BEARER = $tok
  try {
    & "$PSScriptRoot/create-order-with-payment.ps1"
    return $true
  } catch {
    $msg = $_.Exception.Message
    if ($msg -match 'InvalidAudience') { return $false }
    throw
  }
}

$candidates = @(
  $Audience,
  'novapos-frontend,novapos-admin,novapos-postgres',
  'novapos-frontend',
  'novapos-postgres'
) | Select-Object -Unique
foreach ($aud in $candidates) {
  if (Try-RunDemoWithAudience -Aud $aud) { exit 0 }
}

throw "All audience attempts failed. Tried: $($candidates -join ', ')"
