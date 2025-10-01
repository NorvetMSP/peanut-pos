<#!
Scans service source trees for runtime sqlx::query / query_as calls that use a simple string literal
and suggests equivalent macro forms (query! / query_as!). It DOES NOT modify files.

Usage:
  powershell -NoLogo -NoProfile -File .\scripts\convert-sqlx-runtime-to-macro.ps1 [-Services service-a,service-b]

Outputs a table and a companion JSON report: sqlx-runtime-macro-suggestions.json

Heuristics:
  - Matches patterns: sqlx::query("..."), sqlx::query_as::<Type>("..."), sqlx::query_scalar("...")
  - Skips if the string literal contains formatting braces { } or string interpolation signs (not raw here but safety)
  - Skips if it appears to concatenate or use a variable (multi-line with + or contains \n followed by variable markers)
  - Flags queries containing 'WHERE' with potential optional filters for manual review.
Limitations:
  - Does not parse Rust AST; relies on regex heuristics.
  - Multiline raw strings are not handled (add if needed).
#>
[CmdletBinding()]
param(
  [string[]] $Services
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

if (-not $Services -or $Services.Count -eq 0) {
  $Services = @(
    'auth-service','order-service','product-service','payment-service',
    'inventory-service','loyalty-service','customer-service',
    'analytics-service','integration-gateway'
  )
}
$Services = $Services | Sort-Object -Unique

$patternBasic = 'sqlx::query\(\s*"([^"]+)"\s*\)'
$patternAs = 'sqlx::query_as::?<[^>]+>?\(\s*"([^"]+)"\s*\)'
$patternScalar = 'sqlx::query_scalar\(\s*"([^"]+)"\s*\)'

$suggestions = @()
foreach ($svc in $Services) {
  $src = Join-Path services $svc 'src'
  if (-not (Test-Path $src)) { continue }
  $files = Get-ChildItem $src -Recurse -Include *.rs -ErrorAction SilentlyContinue
  foreach ($f in $files) {
    $lines = Get-Content $f.FullName
    for ($i=0; $i -lt $lines.Count; $i++) {
      $line = $lines[$i]
      foreach ($mode in 'basic','as','scalar') {
        $regex = switch ($mode) {
          'basic' { $patternBasic }
          'as' { $patternAs }
          'scalar' { $patternScalar }
        }
        $m = [regex]::Match($line, $regex)
        if ($m.Success) {
          $sql = $m.Groups[1].Value
          # Skip if suspicious dynamic usage
          if ($sql -match '{' -or $sql -match '\\$' -or $sql -match '\\n.*\$') { continue }
          $containsWhere = $sql -match '(?i)\bwhere\b'
          $suggestedMacro = switch ($mode) {
            'basic' { 'query!' }
            'as' { 'query_as!' }
            'scalar' { 'query_scalar!' }
          }
          $rec = [PSCustomObject]@{
            service = $svc
            file = $f.FullName.Substring((Get-Location).Path.Length + 1)
            line = $i + 1
            mode = $mode
            macro = $suggestedMacro
            sql = $sql
            where_clause = $containsWhere
          }
          $suggestions += $rec
        }
      }
    }
  }
}

if ($suggestions.Count -eq 0) {
  Write-Host 'No static runtime sqlx::query patterns found that qualify.'
} else {
  $suggestions | Sort-Object service,file,line | Format-Table -AutoSize
}

# Write JSON report
try {
  $json = $suggestions | ConvertTo-Json -Depth 4
  $json | Out-File -FilePath sqlx-runtime-macro-suggestions.json -Encoding UTF8
  Write-Host "Suggestion report written to sqlx-runtime-macro-suggestions.json"
} catch {
  Write-Warning "Failed to write suggestion report: $($_.Exception.Message)"
}
