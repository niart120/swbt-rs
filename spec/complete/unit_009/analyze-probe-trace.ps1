param(
    [Parameter(Mandatory = $true)]
    [string]$TracePath,

    [Parameter(Mandatory = $true)]
    [string]$CompletionPath,

    [long]$CpuTimeNs = -1,
    [long]$CpuWindowNs = -1
)

$ErrorActionPreference = 'Stop'
$periodNs = [long]8000000

function Get-NearestRank {
    param(
        [Parameter(Mandatory = $true)]
        [long[]]$Values,

        [Parameter(Mandatory = $true)]
        [ValidateSet(50, 95, 99, 100)]
        [int]$Percentile
    )

    if ($Values.Count -eq 0) {
        return $null
    }
    $sorted = @($Values | Sort-Object)
    $rank = [Math]::Ceiling($sorted.Count * $Percentile / 100.0)
    return $sorted[[Math]::Max(0, $rank - 1)]
}

function Get-Distribution {
    param([Parameter(Mandatory = $true)][long[]]$Values)

    return [ordered]@{
        samples = $Values.Count
        p50_ns = Get-NearestRank -Values $Values -Percentile 50
        p95_ns = Get-NearestRank -Values $Values -Percentile 95
        p99_ns = Get-NearestRank -Values $Values -Percentile 99
        max_ns = Get-NearestRank -Values $Values -Percentile 100
    }
}

function Assert-ExactFields {
    param(
        [Parameter(Mandatory = $true)]
        [hashtable]$Record,

        [Parameter(Mandatory = $true)]
        [string[]]$Expected
    )

    $actualNames = @($Record.Keys | Sort-Object)
    $expectedNames = @($Expected | Sort-Object)
    if ($actualNames.Count -ne $expectedNames.Count -or
        (Compare-Object -ReferenceObject $expectedNames -DifferenceObject $actualNames).Count -ne 0) {
        throw "unexpected fields for event: $($Record.event)"
    }
}

$records = @(
    Get-Content -LiteralPath $TracePath | ForEach-Object {
        $_ | ConvertFrom-Json -AsHashtable
    }
)
if ($records.Count -eq 0) {
    throw 'trace has no complete records'
}

$previousElapsed = [long]-1
foreach ($record in $records) {
    if ($record.schema -ne 'swbt.diagnostics' -or $record.schema_version -ne 1) {
        throw 'unexpected diagnostics schema'
    }
    if (-not $record.ContainsKey('trace_elapsed_ns')) {
        throw 'trace record has no writer-observed timestamp'
    }
    $runtimeFields = @(
        'schema',
        'schema_version',
        'event',
        'controller_kind',
        'reporting_kind',
        'session_id',
        'trace_elapsed_ns'
    )
    $expectedFields = switch ($record.event) {
        'environment' {
            @(
                'schema',
                'schema_version',
                'event',
                'controller_kind',
                'reporting_kind',
                'package_version',
                'target_os',
                'target_arch',
                'trace_elapsed_ns'
            )
        }
        'session_started' { $runtimeFields }
        'lifecycle_changed' { $runtimeFields + 'lifecycle' }
        'subcommand_observed' { $runtimeFields + 'subcommand_id' }
        'report_tx_accepted' {
            $runtimeFields + @('report_mode', 'imu_mode', 'input_reports_accepted')
        }
        'reply_tx_accepted' {
            $runtimeFields + @('report_mode', 'imu_mode', 'replies_accepted')
        }
        'session_ended' { $runtimeFields + @('lifecycle', 'disconnect_reason') }
        'worker_failed' { $runtimeFields + 'failure_category' }
        'unsupported_button' { $runtimeFields + 'button_kind' }
        default { throw "unknown diagnostics event: $($record.event)" }
    }
    Assert-ExactFields -Record $record -Expected $expectedFields
    $elapsed = [long]$record.trace_elapsed_ns
    if ($elapsed -lt $previousElapsed) {
        throw 'trace timestamps are not monotonic'
    }
    $previousElapsed = $elapsed
}

$forbiddenFields = @(
    'adapter_selector',
    'usb_bus',
    'usb_address',
    'usb_port',
    'usb_serial',
    'profile_path',
    'profile_json',
    'peer_address',
    'local_address',
    'link_key',
    'raw_packet',
    'error',
    'error_source',
    'message'
)
foreach ($record in $records) {
    foreach ($field in $forbiddenFields) {
        if ($record.ContainsKey($field)) {
            throw "trace contains forbidden field: $field"
        }
    }
}

$ready = $records |
    Where-Object { $_.event -eq 'lifecycle_changed' -and $_.lifecycle -eq 'ready' } |
    Select-Object -First 1
if ($null -eq $ready) {
    throw 'trace has no Ready lifecycle event'
}
$sessionId = [long]$ready.session_id
$sessionEnd = $records |
    Where-Object { $_.event -eq 'session_ended' -and [long]$_.session_id -eq $sessionId } |
    Select-Object -Last 1
if ($null -eq $sessionEnd) {
    throw 'trace has no session end event'
}

$reports = @(
    $records | Where-Object {
        $_.event -eq 'report_tx_accepted' -and
        [long]$_.session_id -eq $sessionId -and
        [long]$_.trace_elapsed_ns -ge [long]$ready.trace_elapsed_ns -and
        [long]$_.trace_elapsed_ns -le [long]$sessionEnd.trace_elapsed_ns
    }
)
if ($reports.Count -lt 2) {
    throw 'trace has fewer than two Ready-session reports'
}

$intervals = [System.Collections.Generic.List[long]]::new()
for ($index = 1; $index -lt $reports.Count; $index++) {
    $intervals.Add(
        [long]$reports[$index].trace_elapsed_ns - [long]$reports[$index - 1].trace_elapsed_ns
    )
}
$intervalValues = [long[]]$intervals.ToArray()
$intervalErrors = [long[]]@(
    $intervalValues | ForEach-Object { [Math]::Abs($_ - $periodNs) }
)
$overrunIntervals = @($intervalValues | Where-Object { $_ -ge 2 * $periodNs })
$missedPeriods = [long]0
foreach ($interval in $overrunIntervals) {
    $missedPeriods += [Math]::Max(0, [Math]::Floor($interval / $periodNs) - 1)
}
$catchUpIntervals = @($intervalValues | Where-Object { $_ -lt $periodNs / 2 })

$replyLatencies = [System.Collections.Generic.List[long]]::new()
$subcommands = @(
    $records | Where-Object {
        $_.event -eq 'subcommand_observed' -and [long]$_.session_id -eq $sessionId
    }
)
foreach ($subcommand in $subcommands) {
    $reply = $records |
        Where-Object {
            $_.event -eq 'reply_tx_accepted' -and
            [long]$_.session_id -eq $sessionId -and
            [long]$_.trace_elapsed_ns -ge [long]$subcommand.trace_elapsed_ns
        } |
        Select-Object -First 1
    if ($null -ne $reply) {
        $replyLatencies.Add(
            [long]$reply.trace_elapsed_ns - [long]$subcommand.trace_elapsed_ns
        )
    }
}

$completion = Get-Content -LiteralPath $CompletionPath -Raw |
    ConvertFrom-Json -AsHashtable
if ($completion.schema -ne 'swbt.probe' -or
    $completion.schema_version -ne 1 -or
    $completion.event -ne 'connection_completed') {
    throw 'unexpected probe completion schema'
}
foreach ($field in @('neutral_close', 'profile_unchanged', 'adapter_reopened')) {
    if ($completion[$field] -ne $true) {
        throw "probe completion did not prove $field"
    }
}

$imuModes = @(
    $reports |
        ForEach-Object { [long]$_.imu_mode } |
        Sort-Object -Unique
)
$cpuPercentOfOneCore = $null
if ($CpuTimeNs -ge 0 -and $CpuWindowNs -gt 0) {
    $cpuPercentOfOneCore = 100.0 * $CpuTimeNs / $CpuWindowNs
}

[ordered]@{
    schema = 'swbt.m8.probe-trace-summary'
    schema_version = 1
    measurement_boundary = 'trace_subscriber_observation_after_status_projection'
    target_period_ns = $periodNs
    trace_records = $records.Count
    session_id = $sessionId
    ready_elapsed_ns = [long]$ready.trace_elapsed_ns
    session_end_lifecycle = $sessionEnd.lifecycle
    disconnect_reason = $sessionEnd.disconnect_reason
    report_mode = $reports[-1].report_mode
    committed_imu_modes = $imuModes
    input_reports_accepted_final = [long]$reports[-1].input_reports_accepted
    reply_reports_accepted_final = if ($records.event -contains 'reply_tx_accepted') {
        [long]($records | Where-Object { $_.event -eq 'reply_tx_accepted' } | Select-Object -Last 1).replies_accepted
    } else {
        0
    }
    observed_subcommands = [long[]]@($subcommands.subcommand_id | Sort-Object -Unique)
    interval = Get-Distribution -Values $intervalValues
    interval_error = Get-Distribution -Values $intervalErrors
    overrun_intervals = $overrunIntervals.Count
    estimated_missed_periods = $missedPeriods
    catch_up_intervals = $catchUpIntervals.Count
    subcommand_reply_latency = Get-Distribution -Values ([long[]]$replyLatencies.ToArray())
    imu_run_seconds = [long]$completion.imu_run_seconds
    imu_apply_command_latency_ns = [long]$completion.imu_apply_command_latency_ns
    imu_non_neutral_reports_accepted = [long]$completion.imu_non_neutral_reports_accepted
    neutral_reports_accepted = [long]$completion.neutral_reports_accepted
    shutdown_latency_ns = [long]$completion.shutdown_latency_ns
    neutral_close = [bool]$completion.neutral_close
    profile_unchanged = [bool]$completion.profile_unchanged
    adapter_reopened = [bool]$completion.adapter_reopened
    process_cpu_time_ns = if ($CpuTimeNs -ge 0) { $CpuTimeNs } else { $null }
    process_cpu_window_ns = if ($CpuWindowNs -ge 0) { $CpuWindowNs } else { $null }
    process_cpu_percent_of_one_core = $cpuPercentOfOneCore
    trace_parse = $true
    forbidden_fields_absent = $true
} | ConvertTo-Json -Depth 5 -Compress
