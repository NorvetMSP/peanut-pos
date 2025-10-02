<#
Runs matrix of audit feature builds:
1. common-audit without kafka features (unit tests)
2. Full workspace build (normal features)
#>
param(
    [switch]$VerboseOutput
)

Write-Host "[audit-matrix] Step 1: common-audit --no-default-features tests" -ForegroundColor Cyan
$c1 = Start-Process powershell -ArgumentList '-NoProfile','-Command','cargo test -p common-audit --no-default-features' -Wait -PassThru
if ($c1.ExitCode -ne 0) { Write-Error "common-audit tests (no features) failed"; exit $c1.ExitCode }

Write-Host "[audit-matrix] Step 2: full workspace build & tests" -ForegroundColor Cyan
$c2 = Start-Process powershell -ArgumentList '-NoProfile','-Command','cargo test --quiet' -Wait -PassThru
if ($c2.ExitCode -ne 0) { Write-Error "workspace tests failed"; exit $c2.ExitCode }

Write-Host "[audit-matrix] SUCCESS" -ForegroundColor Green
