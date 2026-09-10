param([switch]$IncludeNetwork)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot

function Invoke-CapturedCargo {
    param([string[]]$CargoArguments)
    $info = [Diagnostics.ProcessStartInfo]::new()
    $info.FileName = (Get-Command cargo -CommandType Application -ErrorAction Stop).Source
    $info.WorkingDirectory = $root
    $info.UseShellExecute = $false
    $info.CreateNoWindow = $true
    $info.RedirectStandardOutput = $true
    $info.RedirectStandardError = $true
    $info.StandardOutputEncoding = [Text.UTF8Encoding]::new($false)
    $info.StandardErrorEncoding = [Text.UTF8Encoding]::new($false)
    foreach ($argument in $CargoArguments) { $info.ArgumentList.Add($argument) }

    $process = [Diagnostics.Process]::new()
    $process.StartInfo = $info
    try {
        if (-not $process.Start()) { throw 'cargo を開始できませんでした。' }
        # Drain both streams concurrently to avoid blocking on compiler/test output.
        $stdoutTask = $process.StandardOutput.ReadToEndAsync()
        $stderrTask = $process.StandardError.ReadToEndAsync()
        $process.WaitForExit()
        $stdout = $stdoutTask.GetAwaiter().GetResult()
        $stderr = $stderrTask.GetAwaiter().GetResult()
        if ($process.ExitCode -ne 0) {
            [Console]::Error.WriteLine("失敗: cargo " + ($CargoArguments -join ' '))
            if ($stdout) { [Console]::Error.Write($stdout) }
            if ($stderr) { [Console]::Error.Write($stderr) }
            if (-not $stdout -and -not $stderr) {
                [Console]::Error.WriteLine("終了コード: " + $process.ExitCode)
            }
            return $false
        }
        return $true
    } finally {
        $process.Dispose()
    }
}

try {
    if (-not (Invoke-CapturedCargo @('test', '--locked', '--all-targets'))) { exit 1 }
    if ($IncludeNetwork) {
        if (-not (Invoke-CapturedCargo @('test', '--locked', '--test', 'network', '--', '--ignored', '--nocapture'))) { exit 1 }
    }
    [Console]::WriteLine('異常なし')
    exit 0
} catch {
    [Console]::Error.WriteLine($_.Exception.Message)
    exit 1
}
