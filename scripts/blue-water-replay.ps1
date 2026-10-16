[CmdletBinding()]
param(
    [Parameter(Mandatory)]
    [ValidatePattern('^[a-z0-9-]+$')]
    [string]$RunName,
    [ValidateRange(0, 1)]
    [int]$Backend = 1
)
$ErrorActionPreference = 'Stop'
$workspaceRoot = Split-Path -Parent $PSScriptRoot
$runRoot = Join-Path $workspaceRoot "target\blue-water-ab\$RunName"
$sourceBase = Join-Path $runRoot 'base'
$sourceConfig = Join-Path $sourceBase 'DinurdoJK.cfg'
$metadataPath = Join-Path $runRoot 'results.json'
if (-not (Test-Path -LiteralPath $metadataPath)) { throw "Benchmark metadata is missing: $metadataPath" }
if (-not (Test-Path -LiteralPath $sourceConfig)) { throw "Benchmark config is missing: $sourceConfig" }
$runs = Get-Content -LiteralPath $metadataPath -Raw | ConvertFrom-Json
$run = $runs | Where-Object backend -EQ $Backend | Select-Object -Last 1
if (-not $run) { $run = $runs | Select-Object -Last 1 }
$executable = Join-Path $workspaceRoot 'target\release\DinurdoJK.exe'
$actualHash = (Get-FileHash -LiteralPath $executable -Algorithm SHA256).Hash.ToLowerInvariant()
if ($actualHash -ne $run.executable_sha256) {
    throw "The current executable differs from this capture. Choose a capture made with the current build. Expected $($run.executable_sha256); found $actualHash."
}
# Use a separate writable profile so a hands-on session cannot alter the capture.
$playRoot = Join-Path $workspaceRoot "target\blue-water-replay\$RunName"
$playBase = Join-Path $playRoot 'base'
New-Item -ItemType Directory -Force -Path $playBase | Out-Null
foreach ($package in Get-ChildItem -LiteralPath $sourceBase -File -Filter '*.pk3') {
    $destination = Join-Path $playBase $package.Name
    if (-not (Test-Path -LiteralPath $destination)) {
        try { New-Item -ItemType HardLink -Path $destination -Value $package.FullName -ErrorAction Stop | Out-Null }
        catch { Copy-Item -LiteralPath $package.FullName -Destination $destination }
    }
}
Copy-Item -LiteralPath $sourceConfig -Destination (Join-Path $playBase 'DinurdoJK.cfg') -Force
Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'blue-water-optimized.cfg') -Destination (Join-Path $playBase 'blue_water_optimized.cfg') -Force
$commands = @(
    'set sensitivity 5',
    'set cl_mouseAccel 0',
    'set con_notifytime 3',
    "set r_oceanBackend $Backend"
)
if ($run.noclip) { $commands += @('set cg_thirdPerson 0', 'team free', 'noclip') }
$commands += "setviewpos $($run.requested_view)"
[System.IO.File]::WriteAllLines((Join-Path $playBase 'blue_water_replay.cfg'), $commands)
$process = Start-Process -FilePath $executable -ArgumentList @(
    '--base', ('"' + $playBase + '"'), '--map', 'costadelsol', '+exec', 'blue_water_replay.cfg'
) -WorkingDirectory $playRoot -WindowStyle Normal -PassThru
Write-Output "Interactive replay PID: $($process.Id)"
Write-Output "Executable: $executable"
Write-Output "Base (config and linked Costa assets): $playBase"
Write-Output 'Console A/B: r_oceanBackend 0 = Godot; r_oceanBackend 1 = BlueWater.'
Write-Output 'Benchmark graphics settings retained. Mouse restored; no automatic captures or quit.'
