param(
    [switch]$NoFailFast
)

Write-Host "[ci-kafka-tests] Running integration-gateway tests with kafka-producer feature"
$failFastFlag = if ($NoFailFast) { "" } else { "--no-fail-fast" }

cargo test -p integration-gateway --tests --features kafka-producer $failFastFlag
if ($LASTEXITCODE -ne 0) {
  Write-Host "Kafka feature tests failed" -ForegroundColor Red
  exit $LASTEXITCODE
}
Write-Host "Kafka feature tests passed" -ForegroundColor Green
