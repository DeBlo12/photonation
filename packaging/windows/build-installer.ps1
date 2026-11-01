# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 The Photonation contributors
#
# Builds the release binaries and the Windows installer
# (target\installer\photonation-<version>-windows-x64-setup.exe).
# Needs Rust (rustup) with the MSVC build tools, and Inno Setup 6:
#   winget install JRSoftware.InnoSetup
#
# Usage: powershell -ExecutionPolicy Bypass -File packaging\windows\build-installer.ps1 [-SkipBuild]

param(
    # Package the binaries already in target\release.
    [switch]$SkipBuild
)

$ErrorActionPreference = 'Stop'
$root = Resolve-Path (Join-Path $PSScriptRoot '..\..')

# The workspace version, from [workspace.package] in Cargo.toml.
$version = (Select-String -Path (Join-Path $root 'Cargo.toml') -Pattern '^version = "(.+)"' |
    Select-Object -First 1).Matches[0].Groups[1].Value

if (-not $SkipBuild) {
    Push-Location $root
    try {
        cargo build --release --locked -p photonation-desktop -p photonation-cli
        if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    } finally {
        Pop-Location
    }
}

# The licence texts of every compiled-in crate (NOTICE, "Obligations when
# distributing binaries"). Needs cargo-about:
#   cargo install cargo-about --locked --features cli
if (-not (Get-Command cargo-about -ErrorAction SilentlyContinue)) {
    throw 'cargo-about not found. Install it with: cargo install cargo-about --locked --features cli'
}
Push-Location $root
try {
    cargo about generate --locked --workspace --fail `
        -o target\THIRD-PARTY-LICENSES.txt packaging\third-party-licenses.hbs
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
} finally {
    Pop-Location
}

# ISCC from PATH, or from Inno Setup's per-user or per-machine install.
$iscc = (Get-Command ISCC.exe -ErrorAction SilentlyContinue).Source
if (-not $iscc) {
    $iscc = @(
        "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe",
        "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe",
        "$env:ProgramFiles\Inno Setup 6\ISCC.exe"
    ) | Where-Object { Test-Path $_ } | Select-Object -First 1
}
if (-not $iscc) {
    throw 'Inno Setup 6 not found. Install it with: winget install JRSoftware.InnoSetup'
}

& $iscc "/DAppVersion=$version" (Join-Path $PSScriptRoot 'photonation.iss')
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
