# No Pester or network required. Exercise manifest repair and offline bootstrap.
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'ensure-smaa-vendor.ps1')
$workspace = Split-Path -Parent $PSScriptRoot
$fixtureRoot = Join-Path ([System.IO.Path]::GetTempPath()) ('DinurdoJK-smaa-test-' + [guid]::NewGuid().ToString('N'))
$originalCargoHome = $env:CARGO_HOME
$utf8 = New-Object System.Text.UTF8Encoding($false)

function Assert-True($Condition, [string]$Message) {
    if (-not $Condition) { throw $Message }
}

function Assert-Throws([scriptblock]$Action, [string]$MessagePattern) {
    $caught = $false
    try { & $Action } catch {
        if ($_.Exception.Message -notmatch $MessagePattern) { throw }
        $caught = $true
    }
    Assert-True $caught "Expected failure: $MessagePattern"
}

New-Item -ItemType Directory -Path $fixtureRoot | Out-Null
try {
    $manifestPath = Join-Path $fixtureRoot 'Cargo.toml'
    foreach ($newline in @("`n", "`r`n")) {
        foreach ($defaults in @('', 'default-features = true', 'default-features = false')) {
            $manifest = @(
                '[package]', 'name = "smaa"', 'version = "0.21.0"', '',
                '[dependencies.wgpu]', 'version = "30"', 'features = ["glsl"]',
                $defaults, '', '[dev-dependencies.winit]', 'version = "0.29"'
            ) -join $newline
            [System.IO.File]::WriteAllText($manifestPath, $manifest, $utf8)
            Assert-Smaa21Manifest $manifestPath
            Set-SmaaWgpuDefaultsOff $manifestPath
            $patched = [System.IO.File]::ReadAllText($manifestPath)
            Assert-True ($patched -match 'default-features = false') 'WGPU defaults were not disabled'
            Assert-True ($patched.Contains('features = ["glsl"]')) 'GLSL input was removed'
            Assert-True ($patched.Contains('[dev-dependencies.winit]')) 'Following section was damaged'
            $timestamp = (Get-Item -LiteralPath $manifestPath).LastWriteTimeUtc
            Set-SmaaWgpuDefaultsOff $manifestPath
            Assert-True ($patched -ceq [System.IO.File]::ReadAllText($manifestPath)) 'Second run changed bytes'
            Assert-True ($timestamp -eq (Get-Item -LiteralPath $manifestPath).LastWriteTimeUtc) 'Second run rewrote manifest'
        }
    }
    [System.IO.File]::WriteAllText($manifestPath, '[dependencies]' + "`n" + 'wgpu = { version = "30", features = ["glsl"], default-features = true }', $utf8)
    Set-SmaaWgpuDefaultsOff $manifestPath
    Assert-True ([System.IO.File]::ReadAllText($manifestPath).Contains('default-features = false')) 'Inline manifest was not repaired'

    [System.IO.File]::WriteAllText($manifestPath, '[package]' + "`n" + 'name = "smaa"' + "`n" + 'version = "0.20.0"' + "`n" + '[lib]' + "`n" + 'version = "0.21.0"', $utf8)
    Assert-Throws { Assert-Smaa21Manifest $manifestPath } 'Expected the actual SMAA'
    [System.IO.File]::WriteAllText($manifestPath, '[dependencies.wgpu]' + "`n" + 'version = "29"', $utf8)
    Assert-Throws { Set-SmaaWgpuDefaultsOff $manifestPath } 'does not depend on WGPU 30'

    $env:CARGO_HOME = Join-Path $fixtureRoot 'cargo'
    $registry = Join-Path $env:CARGO_HOME 'registry\src\test-registry'
    New-Item -ItemType Directory -Path $registry -Force | Out-Null
    Copy-Item -LiteralPath (Join-Path $workspace 'vendor\smaa-0.21.0') -Destination $registry -Recurse
    $checkout = Join-Path $fixtureRoot 'checkout'
    New-Item -ItemType Directory -Path $checkout | Out-Null
    Ensure-PatchedSmaaVendor -WorkspaceRoot $checkout
    $vendorManifest = Join-Path $checkout 'vendor\smaa-0.21.0\Cargo.toml'
    $vendorHash = (Get-FileHash -LiteralPath $vendorManifest).Hash
    Ensure-PatchedSmaaVendor -WorkspaceRoot $checkout
    Assert-True ($vendorHash -eq (Get-FileHash -LiteralPath $vendorManifest).Hash) 'Offline bootstrap was not idempotent'
    Assert-True (@(Get-ChildItem -LiteralPath (Join-Path $checkout 'vendor') -Directory -Filter '.smaa-*-staging-*').Count -eq 0) 'Staging directory was not cleaned'
    Remove-Item -LiteralPath (Join-Path $checkout 'vendor\smaa-0.21.0\src\lib.rs')
    Assert-Throws { Ensure-PatchedSmaaVendor -WorkspaceRoot $checkout } 'Incomplete SMAA'
    Write-Host 'SMAA vendor checks passed: LF/CRLF manifests, inline repair, version rejection, offline bootstrap, idempotency, incomplete source detection.'
} finally {
    $env:CARGO_HOME = $originalCargoHome
    $resolvedFixture = (Resolve-Path -LiteralPath $fixtureRoot).Path
    $temporaryRoot = [System.IO.Path]::GetFullPath([System.IO.Path]::GetTempPath()).TrimEnd('\') + '\'
    if (-not $resolvedFixture.StartsWith($temporaryRoot, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw 'Fixture cleanup must remain inside the temporary directory.'
    }
    Remove-Item -LiteralPath $resolvedFixture -Recurse -Force
}
