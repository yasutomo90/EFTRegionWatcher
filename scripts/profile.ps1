param([int]$IdleSeconds = 30)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$qa = Join-Path $root 'target/profile'
$logs = Join-Path $qa 'Logs'
$appData = Join-Path $qa 'appdata'
$configDir = Join-Path $appData 'EFTRegionWatcher'
New-Item -ItemType Directory -Force -Path $logs, $configDir | Out-Null
$logPath = $logs.Replace('\', '/')
$config = "eft_log_path = '$logPath'" + [Environment]::NewLine + "[update]" + [Environment]::NewLine + "enabled = false"
[IO.File]::WriteAllText((Join-Path $configDir 'config.toml'), $config, [Text.UTF8Encoding]::new($false))
$process = Start-Process -FilePath (Join-Path $root 'target/release/EFTRegionWatcher.exe') -Environment @{ LOCALAPPDATA = $appData } -WindowStyle Hidden -PassThru
try {
    Start-Sleep -Seconds 3
    $process.Refresh()
    if ($process.HasExited) { throw "App exited: $($process.ExitCode)" }
    $beforeCpu = $process.TotalProcessorTime.TotalSeconds
    $watch = [Diagnostics.Stopwatch]::StartNew()
    Start-Sleep -Seconds $IdleSeconds
    $process.Refresh()
    $idleCpu = $process.TotalProcessorTime.TotalSeconds - $beforeCpu
    $idle = [ordered]@{
        seconds = $watch.Elapsed.TotalSeconds
        cpu_one_core_percent = 100 * $idleCpu / $watch.Elapsed.TotalSeconds
        working_set_mb = $process.WorkingSet64 / 1MB
        private_mb = $process.PrivateMemorySize64 / 1MB
        handles = $process.HandleCount
        threads = $process.Threads.Count
    }
    $beforeCpu = $process.TotalProcessorTime.TotalSeconds
    $watch.Restart()
    for ($raid = 0; $raid -lt 100; $raid++) {
        $session = Join-Path $logs "session-$raid"
        New-Item -ItemType Directory -Force -Path $session | Out-Null
        $payload = ('Unrelated EFT fixture line' + [Environment]::NewLine) * 1000
        $payload += "Connect (address: 203.0.113.10:$(17000 + $raid))" + [Environment]::NewLine
        [IO.File]::WriteAllText((Join-Path $session 'application.log'), $payload)
    }
    Start-Sleep -Seconds 2
    $process.Refresh()
    $active = [ordered]@{
        synthetic_raids = 100
        seconds = $watch.Elapsed.TotalSeconds
        cpu_seconds = $process.TotalProcessorTime.TotalSeconds - $beforeCpu
        working_set_mb = $process.WorkingSet64 / 1MB
        private_mb = $process.PrivateMemorySize64 / 1MB
        handles = $process.HandleCount
        threads = $process.Threads.Count
    }
    $beforeCpu = $process.TotalProcessorTime.TotalSeconds
    Start-Sleep -Seconds 10
    $process.Refresh()
    $after = [ordered]@{
        cpu_seconds_in_10s = $process.TotalProcessorTime.TotalSeconds - $beforeCpu
        working_set_mb = $process.WorkingSet64 / 1MB
        private_mb = $process.PrivateMemorySize64 / 1MB
        handles = $process.HandleCount
        threads = $process.Threads.Count
    }
    $report = [ordered]@{ measured_at = (Get-Date -Format o); os = [Environment]::OSVersion.VersionString; idle = $idle; synthetic_activity = $active; after = $after; network = 'Disabled update checks; documentation IP rejected locally. No packet capture or one-hour request measurement.' }
    $json = $report | ConvertTo-Json -Depth 5
    [IO.File]::WriteAllText((Join-Path $qa 'metrics.json'), $json)
    $json
} finally {
    if (-not $process.HasExited) { Stop-Process -Id $process.Id }
    $process.Dispose()
}
