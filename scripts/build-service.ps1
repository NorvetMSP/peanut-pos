Param(
    [Parameter(Mandatory=$true)][string]$Package,
    # Renamed from $Profile (which conflicts with automatic variable $PROFILE) to $BuildProfile.
    # Alias retained so callers can still use -Profile without clobbering $PROFILE variable.
    [Alias('Profile')][string]$BuildProfile = 'dev',
    [switch]$Verbose,
    [switch]$Release,
    [string]$ManifestPath = 'services/Cargo.toml'
)

$ErrorActionPreference = 'Stop'

if (-not (Test-Path $ManifestPath)) {
    Write-Error "Manifest path '$ManifestPath' not found."; exit 2
}

$features = $null
if ($env:CARGO_FEATURES) { $features = $env:CARGO_FEATURES }

$cargoArgs = @('build','--manifest-path', $ManifestPath,'-p', $Package)
if ($Release) { $cargoArgs += '--release' }
if ($BuildProfile -and -not $Release) { $cargoArgs += @('--profile', $BuildProfile) }
if ($features) { $cargoArgs += @('--features', $features) }
if ($Verbose) { $cargoArgs += '-vv' }

$logRoot = Join-Path (Get-Location) "build-logs"
if (-not (Test-Path $logRoot)) { New-Item -ItemType Directory -Path $logRoot | Out-Null }
$ts = Get-Date -Format 'yyyyMMdd_HHmmss'
$baseName = "$($Package)_$ts"
$outFile = Join-Path $logRoot "$baseName.out.log"
$errFile = Join-Path $logRoot "$baseName.err.log"

Write-Host "[build-service] Running: cargo $($cargoArgs -join ' ')" -ForegroundColor Cyan

# Use Start-Process to capture exit code without treating stderr as terminating error
$psi = New-Object System.Diagnostics.ProcessStartInfo
$psi.FileName = 'cargo'
$psi.ArgumentList.AddRange($cargoArgs)
$psi.RedirectStandardOutput = $true
$psi.RedirectStandardError = $true
$psi.UseShellExecute = $false
$psi.CreateNoWindow = $true
$process = New-Object System.Diagnostics.Process
$process.StartInfo = $psi
[void]$process.Start()
$stdOut = $process.StandardOutput.ReadToEndAsync()
$stdErr = $process.StandardError.ReadToEndAsync()
$process.WaitForExit()
$outText = $stdOut.Result
$errText = $stdErr.Result
$outText | Out-File -FilePath $outFile -Encoding UTF8
$errText | Out-File -FilePath $errFile -Encoding UTF8

$exitCode = $process.ExitCode

if ($exitCode -ne 0) {
    Write-Host "[build-service] Build FAILED (exit $exitCode)" -ForegroundColor Red
    # Surface the last 40 lines of stderr for quick diagnosis
    $tail = ($errText -split "`r?`n") | Select-Object -Last 40
    $tail -join [Environment]::NewLine | Write-Host -ForegroundColor Yellow
    Write-Host "Full logs: `n OUT: $outFile `n ERR: $errFile" -ForegroundColor DarkYellow
    exit $exitCode
} else {
    Write-Host "[build-service] Build SUCCEEDED" -ForegroundColor Green
    # Show summary of warnings count
    $warnCount = ([regex]::Matches($errText, 'warning:')).Count
    Write-Host "Warnings: $warnCount (see $errFile)" -ForegroundColor DarkGreen
    Write-Host "Binary artifacts in target directory." -ForegroundColor DarkGreen
}
