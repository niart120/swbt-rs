param(
    [Parameter(Mandatory = $true)]
    [string[]] $Path
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

foreach ($taskPath in $Path) {
    $taskResolved = Resolve-Path -LiteralPath $taskPath
    $taskBom = Get-Content -Raw -LiteralPath $taskResolved.Path | ConvertFrom-Json

    if ($taskBom.bomFormat -ne 'CycloneDX' -or $taskBom.specVersion -ne '1.5') {
        throw "Expected a CycloneDX 1.5 document: $taskPath"
    }

    $taskRoot = $taskBom.metadata.component
    $taskOldRootRef = [string] $taskRoot.'bom-ref'
    $taskNewRootRef = "pkg:cargo/$($taskRoot.name)@$($taskRoot.version)"
    $taskRoot.'bom-ref' = $taskNewRootRef
    $taskRoot.purl = $taskNewRootRef

    foreach ($taskTarget in @($taskRoot.components)) {
        $taskTargetPurl = [string] $taskTarget.purl
        $taskSubpathMarker = $taskTargetPurl.IndexOf('#')
        if ($taskSubpathMarker -lt 0) {
            throw "Root target lacks a package URL subpath: $taskPath"
        }

        $taskTargetRef = $taskNewRootRef + $taskTargetPurl.Substring($taskSubpathMarker)
        $taskTarget.'bom-ref' = $taskTargetRef
        $taskTarget.purl = $taskTargetRef
    }

    foreach ($taskDependency in @($taskBom.dependencies)) {
        if ($taskDependency.ref -eq $taskOldRootRef) {
            $taskDependency.ref = $taskNewRootRef
        }
    }

    $taskJson = $taskBom | ConvertTo-Json -Depth 100
    if ($taskJson -match 'path\+file|download_url=file|file:///') {
        throw "Local source information remains in $taskPath"
    }

    $taskKnownRefs = @($taskNewRootRef) + @($taskBom.components | ForEach-Object { $_.'bom-ref' })
    $taskUnknownRefs = @($taskBom.dependencies | Where-Object { $_.ref -notin $taskKnownRefs })
    if ($taskUnknownRefs.Count -ne 0) {
        throw "Unknown dependency references remain in $taskPath"
    }

    [System.IO.File]::WriteAllText(
        $taskResolved.Path,
        $taskJson + [Environment]::NewLine,
        [System.Text.UTF8Encoding]::new($false)
    )
}
