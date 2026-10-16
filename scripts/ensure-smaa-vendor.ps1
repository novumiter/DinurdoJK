# Restore the published SMAA 0.21.0 crate, while keeping DinurdoJK's WGPU
# backend policy. Dot-sourced by build.ps1.
# The upstream crate is downloaded only if it is not already vendored.
function Set-SmaaWgpuDefaultsOff([string]$ManifestPath) {
    if (-not (Test-Path -LiteralPath $ManifestPath)) { return }

    $utf8NoBom = New-Object System.Text.UTF8Encoding($false)
    $text = [System.IO.File]::ReadAllText($ManifestPath)

    # Cargo's normalized manifest uses a dependency table.
    $sectionPattern = '(?ms)^\[dependencies\.wgpu\][ \t]*\r?\n.*?(?=^\[|\z)'
    $match = [regex]::Match($text, $sectionPattern)
    if ($match.Success) {
        $section = $match.Value
        if ($section -notmatch '(?m)^version\s*=\s*"30(?:\.[^"]*)?"\s*$') {
            throw "SMAA 0.21.0 unexpectedly does not depend on WGPU 30: $ManifestPath"
        }
        if ($section -match '(?m)^default-features\s*=\s*true\s*$') {
            $section = $section -replace '(?m)^(default-features\s*=\s*)true\s*$', '${1}false'
        } elseif ($section -notmatch '(?m)^default-features\s*=') {
            $newline = if ($section.Contains("`r`n")) { "`r`n" } else { "`n" }
            $section = $section.TrimEnd("`r", "`n") + $newline + 'default-features = false' + $newline
        }
        $text = $text.Substring(0, $match.Index) + $section + $text.Substring($match.Index + $match.Length)
    } else {
        # Cargo.toml.orig is maintained as documentation; only Cargo.toml is
        # consumed by Cargo. Support the upstream inline dependency declaration.
        $inlinePattern = '(?m)^(\s*wgpu\s*=\s*\{)([^}\r\n]*)(\}[ \t]*\r?)$'
        $match = [regex]::Match($text, $inlinePattern)
        if (-not $match.Success) {
            throw "Cannot find upstream SMAA's WGPU dependency in $ManifestPath"
        }
        $inside = $match.Groups[2].Value
        if ($inside -notmatch 'version\s*=\s*"30(?:\.[^"]*)?"') {
            throw "SMAA 0.21.0 unexpectedly does not depend on WGPU 30: $ManifestPath"
        }
        if ($inside -match 'default-features\s*=\s*true') {
            $inside = $inside -replace 'default-features\s*=\s*true', 'default-features = false'
        } elseif ($inside -notmatch 'default-features\s*=') {
            $inside = ' default-features = false,' + $inside
        }
        $text = $text.Substring(0, $match.Index) + $match.Groups[1].Value + $inside + $match.Groups[3].Value + $text.Substring($match.Index + $match.Length)
    }

    if ($text -cne [System.IO.File]::ReadAllText($ManifestPath)) {
        [System.IO.File]::WriteAllText($ManifestPath, $text, $utf8NoBom)
    }
}

function Assert-Smaa21Manifest([string]$ManifestPath) {
    if (-not (Test-Path -LiteralPath $ManifestPath)) { throw "Missing SMAA manifest: $ManifestPath" }
    $manifest = [System.IO.File]::ReadAllText($ManifestPath)
    $package = [regex]::Match($manifest, '(?ms)^\[package\][ \t]*\r?\n.*?(?=^\[|\z)').Value
    if ($package -notmatch '(?m)^name\s*=\s*"smaa"\s*$' -or
        $package -notmatch '(?m)^version\s*=\s*"0\.21\.0"\s*$') {
        throw "Expected the actual SMAA 0.21.0 crate, not a renamed older source: $ManifestPath"
    }
}

function Ensure-PatchedSmaaVendor([string]$WorkspaceRoot) {
    $workspaceRoot = (Resolve-Path -LiteralPath $WorkspaceRoot).Path
    $vendorRoot = Join-Path $workspaceRoot 'vendor'
    $vendorSmaa = Join-Path $vendorRoot 'smaa-0.21.0'
    $vendorManifest = Join-Path $vendorSmaa 'Cargo.toml'

    if (-not (Test-Path -LiteralPath $vendorManifest)) {
        if (Test-Path -LiteralPath $vendorSmaa) {
            throw "Incomplete SMAA vendor directory has no Cargo.toml: $vendorSmaa"
        }
        New-Item -ItemType Directory -Force -Path $vendorRoot | Out-Null
        $stagingRoot = Join-Path $vendorRoot ('.smaa-0.21.0-staging-' + [guid]::NewGuid().ToString('N'))
        New-Item -ItemType Directory -Force -Path $stagingRoot | Out-Null
        try {
            $stagedSmaa = Join-Path $stagingRoot 'smaa-0.21.0'
            # A previously downloaded registry source works without internet.
            $cargoHome = if ($env:CARGO_HOME) { $env:CARGO_HOME } else { Join-Path $env:USERPROFILE '.cargo' }
            $registrySrc = Join-Path $cargoHome 'registry\src'
            $cachedSmaa = $null
            if (Test-Path -LiteralPath $registrySrc) {
                $cachedSmaa = Get-ChildItem -LiteralPath $registrySrc -Directory -ErrorAction SilentlyContinue |
                    ForEach-Object { Join-Path $_.FullName 'smaa-0.21.0' } |
                    Where-Object { Test-Path -LiteralPath (Join-Path $_ 'Cargo.toml') } |
                    Select-Object -First 1
            }
            if ($cachedSmaa) {
                Copy-Item -LiteralPath $cachedSmaa -Destination $stagedSmaa -Recurse -Force
            } else {
                # Pin the published package, not the floating GitHub main branch.
                # Extraction takes place in a temporary directory so a failed
                # download cannot leave an incomplete vendor tree.
                $crateUrl = 'https://static.crates.io/crates/smaa/smaa-0.21.0.crate'
                $crateFile = Join-Path $stagingRoot 'smaa-0.21.0.crate'
                Write-Host 'Fetching upstream SMAA 0.21.0 (one-time vendor bootstrap)...'
                Invoke-WebRequest -UseBasicParsing -Uri $crateUrl -OutFile $crateFile
                $expectedHash = '70f1733b8fc96f93ad2ed99e81f6a3f1fc58a23ad1d8d764ed70016911005c27'
                if ((Get-FileHash -LiteralPath $crateFile -Algorithm SHA256).Hash -ne $expectedHash) {
                    throw 'SMAA 0.21.0 crate checksum does not match its published release.'
                }
                & tar.exe -xzf $crateFile -C $stagingRoot
                if ($LASTEXITCODE -ne 0) { throw 'Failed to extract SMAA 0.21.0 crate.' }
            }

            Assert-Smaa21Manifest (Join-Path $stagedSmaa 'Cargo.toml')
            Set-SmaaWgpuDefaultsOff (Join-Path $stagedSmaa 'Cargo.toml')
            if (Test-Path -LiteralPath (Join-Path $stagedSmaa 'Cargo.toml.orig')) {
                Set-SmaaWgpuDefaultsOff (Join-Path $stagedSmaa 'Cargo.toml.orig')
            }
            if (-not (Test-Path -LiteralPath $vendorManifest)) {
                Move-Item -LiteralPath $stagedSmaa -Destination $vendorSmaa
            }
        } finally {
            if (Test-Path -LiteralPath $stagingRoot) {
                $resolvedStaging = (Resolve-Path -LiteralPath $stagingRoot).Path
                $resolvedVendor = (Resolve-Path -LiteralPath $vendorRoot).Path.TrimEnd('\') + '\'
                if (-not $resolvedStaging.StartsWith($resolvedVendor, [System.StringComparison]::OrdinalIgnoreCase)) {
                    throw 'SMAA staging cleanup must remain inside the vendor directory.'
                }
                Remove-Item -LiteralPath $stagingRoot -Recurse -Force
            }
        }
    }

    Assert-Smaa21Manifest $vendorManifest
    foreach ($relativePath in @('src\lib.rs', 'src\shader.rs', 'third_party\smaa\SMAA.hlsl')) {
        if (-not (Test-Path -LiteralPath (Join-Path $vendorSmaa $relativePath))) {
            throw "Incomplete SMAA 0.21.0 vendor tree: missing $relativePath"
        }
    }
    Set-SmaaWgpuDefaultsOff $vendorManifest
    Set-SmaaWgpuDefaultsOff (Join-Path $vendorSmaa 'Cargo.toml.orig')
    $manifest = [System.IO.File]::ReadAllText($vendorManifest)
    $wgpu = [regex]::Match($manifest, '(?ms)^\[dependencies\.wgpu\][ \t]*\r?\n.*?(?=^\[|\z)').Value
    if ($wgpu -notmatch '(?m)^default-features\s*=\s*false\s*$' -or
        $wgpu -notmatch '(?ms)^features\s*=\s*\[[^\]]*"glsl"') {
        throw 'Vendored SMAA 0.21.0 must disable WGPU default features and retain GLSL input.'
    }
}
