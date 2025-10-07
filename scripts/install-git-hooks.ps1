Param(
  [string]$RepoRoot = (Resolve-Path ".").Path
)

$ErrorActionPreference = 'Stop'

$gitDir = Join-Path $RepoRoot '.git'
if (-not (Test-Path $gitDir)) {
  Write-Error "Not a git repository: $RepoRoot"
}

$hooksSrc = Join-Path $RepoRoot 'scripts/git-hooks/pre-commit'
$hooksDstDir = Join-Path $gitDir 'hooks'
$hooksDst = Join-Path $hooksDstDir 'pre-commit'

if (-not (Test-Path $hooksDstDir)) { New-Item -ItemType Directory -Force -Path $hooksDstDir | Out-Null }

Copy-Item -Force $hooksSrc $hooksDst

# Try to set executable bit for bash scripts on systems that honor it
try {
  & git update-index --chmod=+x scripts/git-hooks/pre-commit 2>$null | Out-Null
} catch {}

Write-Host "Installed pre-commit hook to $hooksDst"
