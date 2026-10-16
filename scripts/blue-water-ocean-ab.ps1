[CmdletBinding()]
param(
    [string]$Executable,
    [string]$SourceBase = 'D:\Code\DinurdoJK\target\release\base',
    [ValidateRange(640, 7680)]
    [int]$Width = 1280,
    [ValidateRange(480, 4320)]
    [int]$Height = 720,
    [int[]]$Backends = @(0, 1),
    [ValidateSet('ocean', 'sky', 'cloud', 'environment')]
    [string]$Compare = 'ocean',
    [ValidateRange(0, 1)]
    [int]$OceanBackend = 1,
    [ValidateRange(0, 1)]
    [int]$SkyBackend = 0,
    [ValidateRange(0, 1)]
    [int]$CloudBackend = 0,
    [ValidatePattern('^[a-z0-9-]+$')]
    [string]$RunName = 'original-surface',
    [ValidatePattern('^-?[0-9]+(?:\.[0-9]+)?(?: +-?[0-9]+(?:\.[0-9]+)?){4}$')]
    [string]$View = '-2397 11562 817 292 -353',
    [switch]$Noclip,
    [switch]$NoClouds = $true,
    [string[]]$ExtraSettings = @(),
    [string]$SettingsFile,
    [string]$ValidationScript,
    [ValidateSet('unspecified', 'full-release', 'thin-lto-iteration')]
    [string]$BuildProfile = 'unspecified'
)
$ErrorActionPreference = 'Stop'
$workspaceRoot = Split-Path -Parent $PSScriptRoot
if (-not $Executable) { $Executable = Join-Path $workspaceRoot 'target\release\DinurdoJK.exe' }
# The client's native runtime DLLs live beside the existing release executable.
# Share them as read-only hardlinks beside this worktree's release executable.
$referenceRelease = Split-Path -Parent $SourceBase
foreach ($dll in Get-ChildItem -LiteralPath $referenceRelease -File -Filter '*.dll') {
    $destination = Join-Path (Split-Path -Parent $Executable) $dll.Name
    if (-not (Test-Path -LiteralPath $destination)) {
        try { New-Item -ItemType HardLink -Path $destination -Value $dll.FullName -ErrorAction Stop | Out-Null }
        catch { Copy-Item -LiteralPath $dll.FullName -Destination $destination }
    }
}
$runRoot = Join-Path $workspaceRoot "target\blue-water-ab\$RunName"
$runBase = Join-Path $runRoot 'base'
New-Item -ItemType Directory -Force -Path $runBase | Out-Null
foreach ($package in Get-ChildItem -LiteralPath $SourceBase -File -Filter '*.pk3') {
    $destination = Join-Path $runBase $package.Name
    if (-not (Test-Path -LiteralPath $destination)) {
        New-Item -ItemType HardLink -Path $destination -Value $package.FullName | Out-Null
    }
}
if (-not (Test-Path -LiteralPath (Join-Path $runBase 'costadelsol.pk3'))) {
    throw 'costadelsol.pk3 is missing from the isolated base directory'
}
function Get-TotalGpuMemoryMiB {
    if (-not (Get-Command nvidia-smi -ErrorAction SilentlyContinue)) { return $null }
    $lines = @(nvidia-smi --query-gpu=memory.used --format=csv,noheader,nounits 2>$null)
    $sum = 0
    foreach ($line in $lines) {
        $value = 0
        if ([int]::TryParse($line.Trim(), [ref]$value)) { $sum += $value }
    }
    return $sum
}
$results = @()
$comparisonCamera = $null
$executableHash = (Get-FileHash -LiteralPath $Executable -Algorithm SHA256).Hash.ToLowerInvariant()
$viewValues = @($View.Split(' ', [System.StringSplitOptions]::RemoveEmptyEntries) | ForEach-Object {
    [double]::Parse($_, [System.Globalization.CultureInfo]::InvariantCulture)
})
$expectedTeleport = 'SETVIEWPOS: origin=({0:F1} {1:F1} {2:F1}) yaw={3:F1} pitch={4:F1}' -f $viewValues
foreach ($backend in $Backends) {
    if ($backend -notin @(0, 1)) { throw "Invalid ocean backend: $backend" }
    $label = "costadelsol_${Compare}_$backend"
    $oceanMode = if ($Compare -in @('ocean', 'environment')) { $backend } else { $OceanBackend }
    $skyMode = if ($Compare -in @('sky', 'environment')) { $backend } else { $SkyBackend }
    $cloudMode = if ($Compare -in @('cloud', 'environment')) { $backend } else { $CloudBackend }
    $settings = if ($SettingsFile) { @(Get-Content -LiteralPath $SettingsFile) } else { @() }
    $settings += @(
        'set r_backend "vulkan"',
        'set r_mode "-1"',
        "set r_customwidth `"$Width`"",
        "set r_customheight `"$Height`"",
        'set r_fullscreen "0"',
        'set r_swapInterval "0"',
        'set r_worldPath "unified"',
        'set r_ocean "1"',
        'set r_oceanSkyExtend "2"',
        "set r_oceanBackend `"$oceanMode`"",
        "set r_skyBackend `"$skyMode`"",
        "set r_cloudBackend `"$cloudMode`"",
        'set r_oceanUpdates "60"',
        'set r_gpuTimings "1"',
        'set sensitivity "0"',
        'set cl_mouseAccel "0"',
        'set con_notifytime "-1"'
    )
    if ($NoClouds) { $settings += 'set r_clouds "0"' }
    $settings += $ExtraSettings
    $setup = @()
    if ($Noclip) { $setup = @('set cg_thirdPerson "0"', 'team free', 'noclip') }
    $commands = $settings + $setup + @(
        "setviewpos $View",
        'perfsample 15 ocean_warmup',
        'screenshot_png',
        "render_perf_capture 180 $label",
        "perfsample 20 $label",
        'wait 10'
    )
    if ($ValidationScript) {
        Copy-Item -LiteralPath $ValidationScript -Destination (Join-Path $runBase 'blue_water_validation.cfg') -Force
        $commands += 'exec blue_water_validation.cfg'
    }
    $commands += @(
        'quit'
    )
    [System.IO.File]::WriteAllLines((Join-Path $runBase 'DinurdoJK.cfg'), $settings)
    [System.IO.File]::WriteAllLines((Join-Path $runBase 'blue_water_ab.cfg'), $commands)
    $stdoutPath = Join-Path $runRoot "$label.log"
    $stderrPath = Join-Path $runRoot "$label.stderr.log"
    $memoryBeforeMiB = Get-TotalGpuMemoryMiB
    $memoryPeakMiB = $memoryBeforeMiB
    $lastMemoryProbe = [DateTime]::UtcNow
    Write-Output "Starting $label"
    $runTimer = [System.Diagnostics.Stopwatch]::StartNew()
    $process = Start-Process -FilePath $Executable -ArgumentList @('--base', ('"' + $runBase + '"'), '--map', 'costadelsol', '+exec', 'blue_water_ab.cfg') -WorkingDirectory $runRoot -WindowStyle Hidden -PassThru -RedirectStandardOutput $stdoutPath -RedirectStandardError $stderrPath
    $null = $process.Handle
    $deadline = [DateTime]::UtcNow.AddMinutes(5)
    while (-not $process.WaitForExit(500)) {
        if ([DateTime]::UtcNow -ge $lastMemoryProbe.AddSeconds(5)) {
            $sample = Get-TotalGpuMemoryMiB
            if ($null -ne $sample) { $memoryPeakMiB = [Math]::Max($memoryPeakMiB, $sample) }
            $lastMemoryProbe = [DateTime]::UtcNow
        }
        if ([DateTime]::UtcNow -gt $deadline) {
            Stop-Process -Id $process.Id
            throw "A/B run timed out: $label ($stdoutPath)"
        }
    }
    $process.WaitForExit()
    $runTimer.Stop()
    $stdout = [System.IO.File]::ReadAllText($stdoutPath)
    $stderr = [System.IO.File]::ReadAllText($stderrPath)
    $cameraMatch = [regex]::Match($stdout, ('label=' + [regex]::Escape($label) + ' .*?view=\(([^)]+)\) yaw=([^ ]+) pitch=([^ ]+)'))
    $sampleCamera = if ($cameraMatch.Success) {
        $cameraMatch.Groups[1].Value + ';yaw=' + $cameraMatch.Groups[2].Value + ';pitch=' + $cameraMatch.Groups[3].Value
    } else { $null }
    $result = [pscustomobject]@{
        label = $label
        backend = $backend
        comparison = $Compare
        ocean_backend = $oceanMode
        sky_backend = $skyMode
        cloud_backend = $cloudMode
        build_profile = $BuildProfile
        executable_sha256 = $executableHash
        exit_code = $process.ExitCode
        run_seconds = [Math]::Round($runTimer.Elapsed.TotalSeconds, 3)
        camera_reached = $stdout.Contains($expectedTeleport)
        requested_view = $View
        noclip = [bool]$Noclip
        clouds_enabled = -not [bool]$NoClouds
        validation_script = $ValidationScript
        extra_settings = $ExtraSettings
        settings_file = $SettingsFile
        camera_at_sample = $sampleCamera
        gpu_error = [bool](($stderr + $stdout) -match 'Validation Error|panicked|DeviceLost|uncaptured error|RENDERER ERROR')
        total_gpu_memory_before_mib = $memoryBeforeMiB
        total_gpu_memory_peak_mib = $memoryPeakMiB
        stdout = $stdoutPath
        stderr = $stderrPath
    }
    $results += $result
    $results | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $runRoot 'results.json')
    Write-Output $result
    if ($result.exit_code -ne 0 -or $result.gpu_error -or -not $result.camera_reached -or -not $sampleCamera `
        -or ($Noclip -and -not $stdout.Contains('NOCLIP ON'))) {
        throw "A/B run failed: $label ($stdoutPath and $stderrPath)"
    }
    if ($null -ne $comparisonCamera -and $comparisonCamera -ne $sampleCamera) {
        throw "A/B cameras differ: $comparisonCamera versus $sampleCamera"
    }
    $comparisonCamera = $sampleCamera
}
