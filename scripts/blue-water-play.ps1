[CmdletBinding()]
param(
    [string]$SourceBase = 'D:\Code\DinurdoJK\target\release\base',
    [string]$Map = 'costadelsol',
    [ValidateRange(640, 7680)]
    [int]$Width = 1920,
    [ValidateRange(480, 4320)]
    [int]$Height = 1080
)
$ErrorActionPreference = 'Stop'
$workspaceRoot = Split-Path -Parent $PSScriptRoot
$executable = Join-Path $workspaceRoot 'target\release\DinurdoJK.exe'
if (-not (Test-Path -LiteralPath $executable)) { throw 'Build the release client first.' }
$playRoot = Join-Path $workspaceRoot 'target\blue-water-play'
$playBase = Join-Path $playRoot 'base'
New-Item -ItemType Directory -Force -Path $playBase | Out-Null
foreach ($package in Get-ChildItem -LiteralPath $SourceBase -File -Filter '*.pk3') {
    $destination = Join-Path $playBase $package.Name
    if (-not (Test-Path -LiteralPath $destination)) {
        New-Item -ItemType HardLink -Path $destination -Value $package.FullName | Out-Null
    }
}
foreach ($dll in Get-ChildItem -LiteralPath (Split-Path -Parent $SourceBase) -File -Filter '*.dll') {
    $destination = Join-Path (Split-Path -Parent $executable) $dll.Name
    if (-not (Test-Path -LiteralPath $destination)) {
        Copy-Item -LiteralPath $dll.FullName -Destination $destination
    }
}
# Dedicated profile with normal input and no benchmark capture or automatic quit.
Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'blue-water-optimized.cfg') -Destination (Join-Path $playBase 'blue_water_optimized.cfg') -Force
Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'blue-water-compare.cfg') -Destination (Join-Path $playBase 'blue_water_compare.cfg') -Force
$commands = @(
    'set r_backend "vulkan"',
    'set r_mode "-1"',
    "set r_customwidth `"$Width`"",
    "set r_customheight `"$Height`"",
    'set r_fullscreen "0"',
    'set r_swapInterval "0"',
    'set r_worldPath "unified"',
    'set r_fastsky "0"',
    'set r_ocean "1"',
    'set r_oceanSkyExtend "2"',
    'exec blue_water_optimized.cfg',
    'set sensitivity "5"',
    'set cl_mouseAccel "0"',
    'set con_notifytime "3"',
    'set cg_thirdPerson "0"',
    'team free',
    'noclip',
    'setviewpos -2397 11562 817 292 -353'
)
[System.IO.File]::WriteAllLines((Join-Path $playBase 'blue_water_play.cfg'), $commands)
$process = Start-Process -FilePath $executable -ArgumentList @(
    '--base', ('"' + $playBase + '"'), '--map', $Map, '+exec', 'blue_water_play.cfg'
) -WorkingDirectory $playRoot -WindowStyle Normal -PassThru
Write-Output "Blue Water free-flight session launched (PID $($process.Id)); isolated profile: $playBase"
