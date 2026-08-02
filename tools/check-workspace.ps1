$ErrorActionPreference = "Stop"

$metadataJson = cargo metadata --format-version 1 --no-deps
if ($LASTEXITCODE -ne 0) {
    throw "cargo metadata failed with exit code $LASTEXITCODE"
}
$metadata = $metadataJson | ConvertFrom-Json

$expectedPackages = @("swbt-hardware-runner", "swbt-probe", "swbt-rs")
$actualPackages = @($metadata.packages.name | Sort-Object)
if (Compare-Object $expectedPackages $actualPackages) {
    throw "workspace packages differ: $($actualPackages -join ', ')"
}

$rootManifest = (Resolve-Path "Cargo.toml").Path
$rootPackage = $metadata.packages | Where-Object {
    [IO.Path]::GetFullPath($_.manifest_path) -eq $rootManifest
}
if ($null -eq $rootPackage) {
    throw "root package is missing"
}
if (@($metadata.workspace_default_members).Count -ne 1 -or
    $metadata.workspace_default_members[0] -ne $rootPackage.id) {
    throw "workspace default member must be the root swbt-rs package"
}

foreach ($toolName in @("swbt-hardware-runner", "swbt-probe")) {
    $tool = $metadata.packages | Where-Object name -eq $toolName
    if ($null -eq $tool -or @($tool.publish).Count -ne 0) {
        throw "$toolName must be a publish=false workspace package"
    }
}

Write-Output "workspace contract passed"
