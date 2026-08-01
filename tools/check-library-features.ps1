$ErrorActionPreference = "Stop"

$defaultTree = cargo tree -p swbt-rs --no-default-features --edges normal --prefix none --locked
if ($LASTEXITCODE -ne 0) {
    throw "default dependency tree failed with exit code $LASTEXITCODE"
}
$defaultTreeText = $defaultTree -join "`n"
if ($defaultTreeText -match "(?m)^tracing v") {
    throw "featureless swbt-rs must not compile tracing"
}

$diagnosticsTree = cargo tree -p swbt-rs --no-default-features --features diagnostics-schema --edges normal --prefix none --locked
if ($LASTEXITCODE -ne 0) {
    throw "diagnostics dependency tree failed with exit code $LASTEXITCODE"
}
$diagnosticsTreeText = $diagnosticsTree -join "`n"
if ($diagnosticsTreeText -notmatch "(?m)^tracing v") {
    throw "diagnostics-schema must compile tracing"
}

Write-Output "library feature contract passed"
