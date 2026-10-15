[CmdletBinding()]
param(
    [string]$Executable,
    [string]$SourceBase,
    [string[]]$Maps = @('mp/ffa3', 'costadelsol'),
    [string[]]$Presets = @('minimal', 'low', 'medium')
)
$ErrorActionPreference = 'Stop'
$workspaceRoot = Split-Path -Parent $PSScriptRoot
if (-not $Executable) { $Executable = Join-Path $workspaceRoot 'target\release\DinurdoJK.exe' }
if (-not $SourceBase) { $SourceBase = Join-Path $workspaceRoot 'target\release\base' }
$validationRoot = Join-Path $workspaceRoot 'target\shader-mode-validation'
$validationBase = Join-Path $validationRoot 'base'
New-Item -ItemType Directory -Force -Path $validationBase | Out-Null
# Only asset packages are shared. Configs, screenshots, caches and logs are isolated.
foreach ($package in Get-ChildItem -LiteralPath $SourceBase -File -Filter '*.pk3') {
    $destination = Join-Path $validationBase $package.Name
    if (-not (Test-Path -LiteralPath $destination)) {
        New-Item -ItemType HardLink -Path $destination -Value $package.FullName | Out-Null
    }
}
$quality = [System.IO.File]::ReadAllText((Join-Path $workspaceRoot 'crates\jka-client\src\app\quality.rs'))
$minimal = [regex]::Match($quality, '(?s)Self::Minimal \| Self::MinimalUnified => &\[(.*?)\],\s*Self::Low').Groups[1].Value
$low = [regex]::Match($quality, '(?s)Self::Low => &\[(.*?)\],\s*Self::High').Groups[1].Value
if (-not $minimal -or -not $low) { throw 'Quality preset source anchors changed.' }
$mediumOverrides = @{}
foreach ($match in [regex]::Matches($quality, '"([^"]+)" => \*value = "([^"]+)"')) {
    $mediumOverrides[$match.Groups[1].Value] = $match.Groups[2].Value
}
$results = @()
foreach ($map in $Maps) {
    foreach ($preset in $Presets) {
        if ($preset -notin @('minimal', 'low', 'medium')) { throw 'Only Minimal Unified, Low and Medium are scoped.' }
        $pairs = [regex]::Matches($(if ($preset -eq 'minimal') { $minimal } else { $low }), '\("([^"]+)", "([^"]+)"\)')
        $settings = @('seta r_backend "vulkan"', 'seta r_mode "-1"', 'seta r_customwidth "640"',
            'seta r_customheight "480"', 'seta r_fullscreen "0"', 'seta r_hdrOutput "0"',
            'seta r_swapInterval "0"', 'seta r_gamma "1"', 'seta r_worldPath "unified"',
            'seta r_fragmentShaderMode "1"', 'seta r_perfTrace "1"', 'seta r_verbose "1"')
        foreach ($pair in $pairs) {
            $name = $pair.Groups[1].Value
            $value = $pair.Groups[2].Value
            if ($preset -eq 'medium' -and $mediumOverrides.ContainsKey($name)) { $value = $mediumOverrides[$name] }
            $settings += 'seta {0} "{1}"' -f $name, $value
        }
        $label = $map.Replace('/', '_') + '_' + $preset
        [System.IO.File]::WriteAllLines((Join-Path $validationBase 'DinurdoJK.cfg'), $settings)
        $commands = @()
        foreach ($setting in $settings) { $commands += $setting.Replace('seta ', 'set ') }
        $commands += @('perfsample 2 baseline_warmup', 'set r_fragmentShaderMode 3',
            'perfsample 8 mode3_compile_wait', "perfsample 2 ${label}_shared", 'screenshot_png', 'wait 10',
            'set r_worldSortMode 1', 'set r_fragmentShaderMode 4', 'perfsample 12 mode4_compile_wait',
            "perfsample 2 ${label}_generated", "render_perf_capture 60 $label", 'wait 500', 'screenshot_png', 'wait 10',
            'set r_fragmentShaderMode 1', 'perfsample 2 return_to_mode1',
            'set r_fragmentShaderMode 4', "perfsample 2 ${label}_cached", 'quit')
        [System.IO.File]::WriteAllLines((Join-Path $validationBase 'shader_smoke.cfg'), $commands)
        $stdoutPath = Join-Path $validationRoot ($label + '.log')
        $stderrPath = Join-Path $validationRoot ($label + '.stderr.log')
        Write-Output "Starting shader-mode smoke test: $label"
        $process = Start-Process -FilePath $Executable -ArgumentList @('--base', ('"' + $validationBase + '"'),
            '--map', $map, '+exec', 'shader_smoke.cfg') -WorkingDirectory $validationRoot -WindowStyle Hidden -PassThru `
            -RedirectStandardOutput $stdoutPath -RedirectStandardError $stderrPath
        # Windows PowerShell may lose ExitCode if the handle is first opened
        # after termination. Retain it while the child is alive.
        $null = $process.Handle
        $deadline = [DateTime]::UtcNow.AddSeconds(120)
        while (-not $process.WaitForExit(500)) {
            if ([DateTime]::UtcNow -gt $deadline) {
                Stop-Process -Id $process.Id
                throw "Smoke test timed out: $label (see $stdoutPath)"
            }
        }
        $process.WaitForExit()
        $output = [System.IO.File]::ReadAllText($stdoutPath)
        $errors = [System.IO.File]::ReadAllText($stderrPath)
        $shared = [regex]::Match($output, 'label=' + [regex]::Escape($label) + '_shared[^\r\n]*shader_mode=3/3[^\r\n]*shared=([1-9][0-9]*)')
        $generated = [regex]::Match($output, 'label=' + [regex]::Escape($label) + '_generated[^\r\n]*shader_mode=4/4[^\r\n]*generated=([1-9][0-9]*)')
        $cached = [regex]::Match($output, 'label=' + [regex]::Escape($label) + '_cached[^\r\n]*shader_mode=4/4')
        $passed = $process.ExitCode -eq 0 -and $shared.Success -and $generated.Success -and $cached.Success `
            -and $errors -notmatch 'Validation Error|panicked at|DeviceLost|uncaptured error'
        $results += [pscustomobject]@{ Map=$map; Preset=$preset; Passed=$passed; ExitCode=$process.ExitCode;
            SharedBinds=$(if ($shared.Success) { [int]$shared.Groups[1].Value } else { 0 });
            GeneratedBinds=$(if ($generated.Success) { [int]$generated.Groups[1].Value } else { 0 }); Log=$stdoutPath }
        $results | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $validationRoot 'results.json')
        Write-Output "Completed ${label}: passed=$passed shared=$($shared.Groups[1].Value) generated=$($generated.Groups[1].Value)"
        if (-not $passed) { throw "Shader-mode smoke test failed: $label (see $stdoutPath and $stderrPath)" }
    }
}
