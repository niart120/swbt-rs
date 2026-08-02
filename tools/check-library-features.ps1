$ErrorActionPreference = "Stop"

$coreTree = cargo tree -p swbt-core --edges normal --prefix none --locked
if ($LASTEXITCODE -ne 0) {
    throw "core dependency tree failed with exit code $LASTEXITCODE"
}
$coreTreeText = $coreTree -join "`n"
if ($coreTreeText -match "(?m)^(swbt-bumble-backend|rusb|tracing|atomic-write-file) v") {
    throw "swbt-core must remain independent of runtime, USB, tracing, and profile writer dependencies"
}

$runtimeTree = cargo tree -p swbt-rs --no-default-features --edges normal --prefix none --locked
if ($LASTEXITCODE -ne 0) {
    throw "runtime dependency tree failed with exit code $LASTEXITCODE"
}
$runtimeTreeText = $runtimeTree -join "`n"
foreach ($package in @("swbt-bumble-backend", "rusb", "tracing")) {
    if ($runtimeTreeText -notmatch "(?m)^$package v") {
        throw "swbt-rs --no-default-features must compile $package"
    }
}

Write-Output "core/runtime package boundary passed"
