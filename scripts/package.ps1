param([string]$OutputDirectory = "target/dist")
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$out = [IO.Path]::GetFullPath((Join-Path $root $OutputDirectory))
if (-not $out.StartsWith($root + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) { throw 'Package output must be inside the repository.' }
New-Item -ItemType Directory -Force -Path $out | Out-Null
Copy-Item -LiteralPath (Join-Path $root 'target/release/EFTRegionWatcher.exe') -Destination (Join-Path $out 'EFTRegionWatcher.exe')
Copy-Item -LiteralPath (Join-Path $root 'target/release/EFTRegionWatcherUpdater.exe') -Destination (Join-Path $out 'EFTRegionWatcher.Updater.exe')
Copy-Item -LiteralPath (Join-Path $root 'README.md') -Destination (Join-Path $out 'README.md')
Copy-Item -LiteralPath (Join-Path $root 'LICENSE') -Destination (Join-Path $out 'LICENSE')
# The embedded Noto Sans JP is redistributed under the SIL Open Font License.
Copy-Item -LiteralPath (Join-Path $root 'assets/fonts/OFL.txt') -Destination (Join-Path $out 'LICENSE-NotoSansJP.txt')
Copy-Item -LiteralPath (Join-Path $root 'docs') -Destination $out -Recurse -Force
$sums = foreach ($name in @('EFTRegionWatcher.exe', 'EFTRegionWatcher.Updater.exe')) {
    $digest = (Get-FileHash -LiteralPath (Join-Path $out $name) -Algorithm SHA256).Hash.ToLowerInvariant()
    "$digest  $name"
}
[IO.File]::WriteAllLines((Join-Path $out 'SHA256SUMS.txt'), $sums, [Text.UTF8Encoding]::new($false))
Compress-Archive -LiteralPath @((Join-Path $out 'EFTRegionWatcher.exe'), (Join-Path $out 'EFTRegionWatcher.Updater.exe'), (Join-Path $out 'README.md'), (Join-Path $out 'LICENSE'), (Join-Path $out 'LICENSE-NotoSansJP.txt'), (Join-Path $out 'SHA256SUMS.txt'), (Join-Path $out 'docs')) -DestinationPath (Join-Path $out 'EFTRegionWatcher-win-x64.zip') -Force
$zipHash = (Get-FileHash -LiteralPath (Join-Path $out 'EFTRegionWatcher-win-x64.zip') -Algorithm SHA256).Hash.ToLowerInvariant()
[IO.File]::WriteAllText((Join-Path $out 'EFTRegionWatcher-win-x64.zip.sha256'), "$zipHash  EFTRegionWatcher-win-x64.zip" + [Environment]::NewLine, [Text.UTF8Encoding]::new($false))
