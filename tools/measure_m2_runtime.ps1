[CmdletBinding()]
param(
    [string]$OutputDirectory = "",
    [switch]$Quick,
    [switch]$AllowDirty
)

$ErrorActionPreference = "Stop"
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$utf8NoBom = [System.Text.UTF8Encoding]::new($false)
$startedAtUtc = (Get-Date).ToUniversalTime().ToString("o")

function Write-Utf8NoBom {
    param(
        [Parameter(Mandatory = $true)]
        [string]$LiteralPath,
        [Parameter(Mandatory = $true)]
        [string]$Content
    )

    [System.IO.File]::WriteAllText($LiteralPath, $Content, $utf8NoBom)
}

function Wait-ForPath {
    param(
        [Parameter(Mandatory = $true)]
        [string]$LiteralPath,
        [Parameter(Mandatory = $true)]
        [System.Diagnostics.Process]$Process,
        [Parameter(Mandatory = $true)]
        [int]$TimeoutMilliseconds,
        [Parameter(Mandatory = $true)]
        [string]$Description
    )

    $watch = [System.Diagnostics.Stopwatch]::StartNew()
    while (-not [System.IO.File]::Exists($LiteralPath)) {
        if ($Process.HasExited) {
            throw "measurement process exited before $Description"
        }
        if ($watch.ElapsedMilliseconds -ge $TimeoutMilliseconds) {
            throw "timed out waiting for $Description"
        }
        Start-Sleep -Milliseconds 10
        $Process.Refresh()
    }
}

function Get-RemainingMilliseconds {
    param(
        [Parameter(Mandatory = $true)]
        [System.Diagnostics.Stopwatch]$Watch,
        [Parameter(Mandatory = $true)]
        [int]$LimitMilliseconds,
        [Parameter(Mandatory = $true)]
        [string]$Description
    )

    $remaining = $LimitMilliseconds - [int]$Watch.ElapsedMilliseconds
    if ($remaining -le 0) {
        throw "overall watchdog expired before $Description"
    }
    $remaining
}

if ($AllowDirty -and -not $Quick) {
    throw "-AllowDirty is restricted to -Quick smoke runs"
}

$statusLines = @(& git -C $repoRoot status --porcelain=v1 --untracked-files=all)
if ($LASTEXITCODE -ne 0) {
    throw "git status failed"
}
$worktreeDirty = $statusLines.Count -gt 0
if ($worktreeDirty -and -not $AllowDirty) {
    throw "measurement requires a clean worktree; use -Quick -AllowDirty only for smoke runs"
}
$headSha = (& git -C $repoRoot rev-parse HEAD).Trim()
if ($LASTEXITCODE -ne 0) {
    throw "git rev-parse failed"
}
$branch = (& git -C $repoRoot branch --show-current).Trim()
if ($LASTEXITCODE -ne 0) {
    throw "git branch failed"
}
$sourceStatusStart = (@($statusLines | Sort-Object) -join "`n")

if (-not $OutputDirectory) {
    $timestamp = (Get-Date).ToUniversalTime().ToString("yyyyMMddTHHmmssfffZ")
    $nonce = [Guid]::NewGuid().ToString("N").Substring(0, 8)
    $OutputDirectory = Join-Path "target\measurements\m2-activity-wait" "$timestamp-$nonce"
}
if ([System.IO.Path]::IsPathRooted($OutputDirectory)) {
    throw "OutputDirectory must be relative to the repository root"
}

$resolvedOutput = [System.IO.Path]::GetFullPath(
    [System.IO.Path]::Combine($repoRoot, $OutputDirectory)
)
$repoPrefix = $repoRoot.TrimEnd(
    [System.IO.Path]::DirectorySeparatorChar,
    [System.IO.Path]::AltDirectorySeparatorChar
) + [System.IO.Path]::DirectorySeparatorChar
$allowedOutputRoots = @(
    [System.IO.Path]::GetFullPath(
        [System.IO.Path]::Combine(
            $repoRoot,
            "target\measurements\m2-activity-wait"
        )
    ),
    [System.IO.Path]::GetFullPath(
        [System.IO.Path]::Combine(
            $repoRoot,
            "spec\wip\unit_003\evidence"
        )
    ),
    [System.IO.Path]::GetFullPath(
        [System.IO.Path]::Combine(
            $repoRoot,
            "spec\complete\unit_003\evidence"
        )
    )
)
$outputAllowed = $false
foreach ($allowedRoot in $allowedOutputRoots) {
    $allowedPrefix = $allowedRoot.TrimEnd(
        [System.IO.Path]::DirectorySeparatorChar,
        [System.IO.Path]::AltDirectorySeparatorChar
    ) + [System.IO.Path]::DirectorySeparatorChar
    if ($resolvedOutput.StartsWith(
        $allowedPrefix,
        [System.StringComparison]::OrdinalIgnoreCase
    )) {
        $outputAllowed = $true
        break
    }
}
if (-not $outputAllowed) {
    throw "OutputDirectory must be a run below the M2 measurement or evidence roots"
}
if ([System.IO.Directory]::Exists($resolvedOutput) -or
    [System.IO.File]::Exists($resolvedOutput)) {
    throw "OutputDirectory already exists"
}
$outputRelative = $resolvedOutput.Substring($repoPrefix.Length).Replace("\", "/")

$buildStartInfo = [System.Diagnostics.ProcessStartInfo]::new()
$buildStartInfo.FileName = "cargo"
$buildStartInfo.Arguments =
    "test --release --lib --no-default-features --locked --no-run --message-format=json"
$buildStartInfo.WorkingDirectory = $repoRoot
$buildStartInfo.UseShellExecute = $false
$buildStartInfo.CreateNoWindow = $true
$buildStartInfo.RedirectStandardOutput = $true
$buildStartInfo.RedirectStandardError = $true
$buildProcess = [System.Diagnostics.Process]::new()
$buildProcess.StartInfo = $buildStartInfo
$buildStarted = $false
try {
    $buildStarted = $buildProcess.Start()
    if (-not $buildStarted) {
        throw "release measurement build did not start"
    }
    $buildStdoutTask = $buildProcess.StandardOutput.ReadToEndAsync()
    $buildStderrTask = $buildProcess.StandardError.ReadToEndAsync()
    if (-not $buildProcess.WaitForExit(300000)) {
        throw "release measurement build exceeded its watchdog"
    }
    $buildStdout = $buildStdoutTask.GetAwaiter().GetResult()
    $buildStderr = $buildStderrTask.GetAwaiter().GetResult()
    $buildExitCode = $buildProcess.ExitCode
} finally {
    if ($buildStarted -and -not $buildProcess.HasExited) {
        try {
            $buildProcess.Kill()
            $null = $buildProcess.WaitForExit(5000)
        } catch {
        }
    }
    $buildProcess.Dispose()
}
if ($buildExitCode -ne 0) {
    throw "release measurement build failed`n$buildStderr"
}
$buildOutput = @($buildStdout -split "\r?\n")

$testExecutables = @()
foreach ($line in $buildOutput) {
    try {
        $message = $line | ConvertFrom-Json -ErrorAction Stop
    } catch {
        continue
    }
    if ($message.reason -eq "compiler-artifact" -and
        $message.profile.test -eq $true -and
        $message.target.kind -contains "lib" -and
        $message.target.name -eq "swbt" -and
        $message.executable) {
        $testExecutables += [string]$message.executable
    }
}
$testExecutables = @($testExecutables | Sort-Object -Unique)
if ($testExecutables.Count -ne 1) {
    throw "Cargo must report exactly one release lib-test executable"
}
$testExecutable = $testExecutables[0]
if (-not [System.IO.File]::Exists($testExecutable)) {
    throw "Cargo reported a missing release lib-test executable"
}

[System.IO.Directory]::CreateDirectory($resolvedOutput) | Out-Null
$rawPath = Join-Path $resolvedOutput "activity-wait.ndjson"
$summaryPath = Join-Path $resolvedOutput "activity-wait-summary.json"
$manifestPath = Join-Path $resolvedOutput "manifest.json"
$manifestHashPath = Join-Path $resolvedOutput "manifest.sha256"
$idleReadyPath = Join-Path $resolvedOutput "idle-ready"
$idleStartPath = Join-Path $resolvedOutput "idle-start"
$idleDonePath = Join-Path $resolvedOutput "idle-done"
$idleCpuPath = Join-Path $resolvedOutput "idle-cpu.json"

if ($Quick) {
    $sampleConfig = [ordered]@{
        idle_ms = 100
        jitter = 20
        command = 100
        transport = 100
        shutdown_each = 10
        fairness_ticks = 20
    }
    $overallTimeoutMilliseconds = 120000
} else {
    $sampleConfig = [ordered]@{
        idle_ms = 10000
        jitter = 10000
        command = 10000
        transport = 10000
        shutdown_each = 1000
        fairness_ticks = 10000
    }
    $overallTimeoutMilliseconds = 900000
}

$startInfo = [System.Diagnostics.ProcessStartInfo]::new()
$startInfo.FileName = $testExecutable
$startInfo.Arguments = "controller::runtime_measurement::activity_wait_decision --exact --ignored --nocapture --test-threads=1"
$startInfo.WorkingDirectory = $repoRoot
$startInfo.UseShellExecute = $false
$startInfo.CreateNoWindow = $true
$startInfo.RedirectStandardOutput = $true
$startInfo.RedirectStandardError = $true
$startInfo.EnvironmentVariables["SWBT_MEASUREMENT_OUTPUT"] = $rawPath
$startInfo.EnvironmentVariables["SWBT_MEASUREMENT_GIT_SHA"] = $headSha
$startInfo.EnvironmentVariables["SWBT_MEASUREMENT_PROFILE"] = "release-test"
$startInfo.EnvironmentVariables["SWBT_MEASUREMENT_FEATURE_SET"] = "no-default-features"
$startInfo.EnvironmentVariables["SWBT_MEASUREMENT_MODE"] = if ($Quick) { "quick" } else { "full" }
$startInfo.EnvironmentVariables["SWBT_MEASUREMENT_WORKTREE_DIRTY"] =
    if ($worktreeDirty) { "true" } else { "false" }
$startInfo.EnvironmentVariables["SWBT_MEASUREMENT_OS"] =
    [System.Runtime.InteropServices.RuntimeInformation]::OSDescription
$startInfo.EnvironmentVariables["SWBT_MEASUREMENT_CPU"] = $env:PROCESSOR_IDENTIFIER
$startInfo.EnvironmentVariables["SWBT_MEASUREMENT_LOGICAL_CPUS"] =
    [string][Environment]::ProcessorCount
$startInfo.EnvironmentVariables["SWBT_MEASUREMENT_IDLE_MS"] =
    [string]$sampleConfig.idle_ms
$startInfo.EnvironmentVariables["SWBT_MEASUREMENT_JITTER_SAMPLES"] =
    [string]$sampleConfig.jitter
$startInfo.EnvironmentVariables["SWBT_MEASUREMENT_COMMAND_SAMPLES"] =
    [string]$sampleConfig.command
$startInfo.EnvironmentVariables["SWBT_MEASUREMENT_TRANSPORT_SAMPLES"] =
    [string]$sampleConfig.transport
$startInfo.EnvironmentVariables["SWBT_MEASUREMENT_SHUTDOWN_SAMPLES"] =
    [string]$sampleConfig.shutdown_each
$startInfo.EnvironmentVariables["SWBT_MEASUREMENT_FAIRNESS_TICKS"] =
    [string]$sampleConfig.fairness_ticks

$process = [System.Diagnostics.Process]::new()
$process.StartInfo = $startInfo
$started = $false
try {
    $started = $process.Start()
    if (-not $started) {
        throw "measurement process did not start"
    }
    $stdoutTask = $process.StandardOutput.ReadToEndAsync()
    $stderrTask = $process.StandardError.ReadToEndAsync()
    $processWatch = [System.Diagnostics.Stopwatch]::StartNew()

    $remainingMilliseconds = Get-RemainingMilliseconds `
        -Watch $processWatch `
        -LimitMilliseconds $overallTimeoutMilliseconds `
        -Description "idle-ready marker"
    Wait-ForPath `
        -LiteralPath $idleReadyPath `
        -Process $process `
        -TimeoutMilliseconds ([Math]::Min(30000, $remainingMilliseconds)) `
        -Description "idle-ready marker"

    $process.Refresh()
    $cpuBefore = $process.TotalProcessorTime.Ticks
    $idleWall = [System.Diagnostics.Stopwatch]::StartNew()
    Write-Utf8NoBom -LiteralPath $idleStartPath -Content "start`n"

    $idleTimeoutMilliseconds = [Math]::Max(
        30000,
        [int]$sampleConfig.idle_ms + 30000
    )
    $remainingMilliseconds = Get-RemainingMilliseconds `
        -Watch $processWatch `
        -LimitMilliseconds $overallTimeoutMilliseconds `
        -Description "idle-done marker"
    Wait-ForPath `
        -LiteralPath $idleDonePath `
        -Process $process `
        -TimeoutMilliseconds ([Math]::Min(
            $idleTimeoutMilliseconds,
            $remainingMilliseconds
        )) `
        -Description "idle-done marker"

    $idleWall.Stop()
    $process.Refresh()
    $cpuAfter = $process.TotalProcessorTime.Ticks
    $cpuDelta = [Math]::Max(0, $cpuAfter - $cpuBefore)
    $wallNanoseconds = [long][Math]::Round(
        $idleWall.Elapsed.TotalMilliseconds * 1000000.0
    )
    $idleCpu = [ordered]@{
        process_cpu_ticks_100ns = $cpuDelta
        wall_ns = $wallNanoseconds
    }
    $idleCpuTemporaryPath = "$idleCpuPath.tmp-$PID"
    Write-Utf8NoBom `
        -LiteralPath $idleCpuTemporaryPath `
        -Content (($idleCpu | ConvertTo-Json -Depth 4 -Compress) + "`n")
    [System.IO.File]::Move($idleCpuTemporaryPath, $idleCpuPath)

    $remainingMilliseconds = Get-RemainingMilliseconds `
        -Watch $processWatch `
        -LimitMilliseconds $overallTimeoutMilliseconds `
        -Description "measurement process completion"
    if (-not $process.WaitForExit($remainingMilliseconds)) {
        throw "measurement process exceeded its overall watchdog"
    }
    $stdout = $stdoutTask.GetAwaiter().GetResult()
    $stderr = $stderrTask.GetAwaiter().GetResult()
    if ($process.ExitCode -ne 0) {
        throw "measurement process failed with exit code $($process.ExitCode)`n$stdout`n$stderr"
    }
} finally {
    if ($started -and -not $process.HasExited) {
        try {
            $process.Kill()
            $null = $process.WaitForExit(5000)
        } catch {
        }
    }
    $process.Dispose()
}

if (-not [System.IO.File]::Exists($rawPath) -or
    -not [System.IO.File]::Exists($summaryPath)) {
    throw "measurement did not write raw and summary files"
}

$rawLineCount = 0L
$rawFirstLine = $null
$commandCapacity = 0
$fakeEventCapacity = 0
$metricCounts = @{
    idle = 0L
    jitter = 0L
    command = 0L
    transport = 0L
    "shutdown:idle" = 0L
    "shutdown:saturated" = 0L
    fairness = 0L
}
foreach ($rawLine in [System.IO.File]::ReadLines($rawPath)) {
    try {
        $rawRecord = $rawLine | ConvertFrom-Json
    } catch {
        throw "raw record $rawLineCount is not valid JSON: $($_.Exception.Message)"
    }
    if ($rawRecord.schema -ne "swbt.m2.activity-wait.raw.v2") {
        throw "raw record $rawLineCount has an unexpected schema"
    }
    if ($rawLineCount -eq 0) {
        $rawFirstLine = $rawLine
        if ($rawRecord.record -ne "meta") {
            throw "first raw record is not measurement metadata"
        }
        $commandCapacity = [int]$rawRecord.meta.tuning.command_capacity
        $fakeEventCapacity = [int]$rawRecord.meta.tuning.fake_event_capacity
        if ($commandCapacity -le 0 -or $fakeEventCapacity -le 0) {
            throw "raw metadata has invalid fairness capacities"
        }
    } else {
        $metricKey = [string]$rawRecord.metric
        if ($metricKey -eq "shutdown") {
            $metricKey = "shutdown:$([string]$rawRecord.condition)"
        }
        if (-not $metricCounts.ContainsKey($metricKey)) {
            throw "raw record $rawLineCount has an unexpected metric or condition"
        }
        $expectedIndex = [long]$metricCounts[$metricKey]
        if ([long]$rawRecord.sample_index -ne $expectedIndex) {
            throw "raw $metricKey sample index mismatch: expected $expectedIndex, got $($rawRecord.sample_index)"
        }
        if ($metricKey -eq "fairness") {
            if (@($rawRecord.command_response_ns).Count -ne
                $commandCapacity -or
                @($rawRecord.post_release_command_completion_observed_ns).Count -ne
                $commandCapacity) {
                throw "fairness sample $expectedIndex does not contain one full command batch"
            }
            if (@($rawRecord.reply_attempt_ns).Count -ne
                $fakeEventCapacity -or
                @($rawRecord.post_release_reply_attempt_ns).Count -ne
                $fakeEventCapacity) {
                throw "fairness sample $expectedIndex does not contain one full transport batch"
            }
            if ([long]$rawRecord.commands_completed -ne
                [long]$commandCapacity -or
                [long]$rawRecord.transport_events_drained -ne
                [long]$fakeEventCapacity) {
                throw "fairness sample $expectedIndex did not complete its configured workload"
            }
        }
        $metricCounts[$metricKey] = $expectedIndex + 1L
    }
    $rawLineCount += 1
}
$expectedRawLineCount = 2L +
    [long]$sampleConfig.jitter +
    [long]$sampleConfig.command +
    [long]$sampleConfig.transport +
    (2L * [long]$sampleConfig.shutdown_each) +
    [long]$sampleConfig.fairness_ticks
if ($rawLineCount -ne $expectedRawLineCount) {
    throw "raw sample count mismatch: expected $expectedRawLineCount, got $rawLineCount"
}

$expectedMetricCounts = @{
    idle = 1L
    jitter = [long]$sampleConfig.jitter
    command = [long]$sampleConfig.command
    transport = [long]$sampleConfig.transport
    "shutdown:idle" = [long]$sampleConfig.shutdown_each
    "shutdown:saturated" = [long]$sampleConfig.shutdown_each
    fairness = [long]$sampleConfig.fairness_ticks
}
foreach ($metricKey in $expectedMetricCounts.Keys) {
    if ([long]$metricCounts[$metricKey] -ne
        [long]$expectedMetricCounts[$metricKey]) {
        throw "raw $metricKey count mismatch: expected $($expectedMetricCounts[$metricKey]), got $($metricCounts[$metricKey])"
    }
}

$rawMetadata = $rawFirstLine | ConvertFrom-Json
if ($rawMetadata.schema -ne "swbt.m2.activity-wait.raw.v2" -or
    $rawMetadata.record -ne "meta") {
    throw "raw measurement metadata has an unexpected schema or record"
}
$summaryDocument = Get-Content -LiteralPath $summaryPath -Raw | ConvertFrom-Json
if ($summaryDocument.schema -ne "swbt.m2.activity-wait.summary.v2") {
    throw "measurement summary has an unexpected schema"
}

$headShaEnd = (& git -C $repoRoot rev-parse HEAD).Trim()
$branchEnd = (& git -C $repoRoot branch --show-current).Trim()
$statusLinesEnd = @(& git -C $repoRoot status --porcelain=v1 --untracked-files=all)
if ($LASTEXITCODE -ne 0) {
    throw "final source-state inspection failed"
}
$outputPattern = [regex]::Escape($outputRelative + "/")
$sourceStatusEndLines = @(
    $statusLinesEnd | Where-Object {
        $_.Replace("\", "/") -notmatch $outputPattern
    }
)
$sourceStatusEnd = (@($sourceStatusEndLines | Sort-Object) -join "`n")
$sourceChanged = (
    $headShaEnd -ne $headSha -or
    $branchEnd -ne $branch -or
    $sourceStatusEnd -ne $sourceStatusStart
)
if ($sourceChanged -and -not $Quick) {
    throw "source state changed during retained measurement"
}

foreach ($transientPath in @(
    $idleReadyPath,
    $idleStartPath,
    $idleDonePath,
    $idleCpuPath
)) {
    [System.IO.File]::Delete($transientPath)
}

$rawHash = (Get-FileHash -LiteralPath $rawPath -Algorithm SHA256).Hash.ToLowerInvariant()
$summaryHash = (
    Get-FileHash -LiteralPath $summaryPath -Algorithm SHA256
).Hash.ToLowerInvariant()
$manifest = [ordered]@{
    schema = "swbt.m2.activity-wait.manifest.v2"
    started_at_utc = $startedAtUtc
    finished_at_utc = (Get-Date).ToUniversalTime().ToString("o")
    source = [ordered]@{
        head_sha = $headSha
        branch = $branch
        worktree_dirty_at_start = $worktreeDirty
        head_sha_end = $headShaEnd
        branch_end = $branchEnd
        changed_during_run = $sourceChanged
        evidence_class =
            if (-not $Quick -and -not $worktreeDirty -and -not $sourceChanged) {
                "retained"
            } else {
                "smoke-only"
            }
    }
    runner = [ordered]@{
        profile = "release-test"
        feature_set = "no-default-features"
        mode = if ($Quick) { "quick" } else { "full" }
        output = $outputRelative
        build_command =
            "cargo test --release --lib --no-default-features --locked --no-run --message-format=json"
        test_arguments =
            "controller::runtime_measurement::activity_wait_decision --exact --ignored --nocapture --test-threads=1"
        overall_watchdog_ms = $overallTimeoutMilliseconds
        powershell = "$($PSVersionTable.PSEdition) $($PSVersionTable.PSVersion)"
        samples = $sampleConfig
        raw_records = [ordered]@{
            expected = $expectedRawLineCount
            actual = $rawLineCount
        }
    }
    files = @(
        [ordered]@{
            path = "activity-wait.ndjson"
            sha256 = $rawHash
        },
        [ordered]@{
            path = "activity-wait-summary.json"
            sha256 = $summaryHash
        }
    )
}
Write-Utf8NoBom `
    -LiteralPath $manifestPath `
    -Content (($manifest | ConvertTo-Json -Depth 8) + "`n")
$manifestHash = (
    Get-FileHash -LiteralPath $manifestPath -Algorithm SHA256
).Hash.ToLowerInvariant()
Write-Utf8NoBom `
    -LiteralPath $manifestHashPath `
    -Content "$manifestHash  manifest.json`n"

Write-Output "measurement complete"
Write-Output "output: $OutputDirectory"
