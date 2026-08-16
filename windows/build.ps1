# Copyright 2026 Curtis Galloway
#
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
#     http://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.

# Build the "Oh Brother" Windows app from the Rust workspace (the
# label-app shell plus the label-web.exe and label.exe it bundles),
# mirroring macos/build.sh. Usage:
#   windows\build.ps1            build into windows\build\Oh Brother\
#   windows\build.ps1 install    build, copy to %LOCALAPPDATA%\Programs,
#                                and create a Start Menu shortcut
#                                (dev-loop shortcut; end users get the MSI)
#   windows\build.ps1 msi        build, then produce the per-user MSI
#                                (installer\Package.wxs) into windows\build\
#   windows\build.ps1 bundle     msi, then wrap it in the Burn setup
#                                bundle (installer\Bundle.wxs) that chains
#                                the WebView2 Evergreen Bootstrapper
#   -NoCargo                     skip the cargo build (reuse existing
#                                target\release binaries; used by CI where
#                                cargo has already run)
#
# Requires a Rust toolchain (MSVC) on PATH. The msi/bundle actions
# additionally need the WiX v7 CLI:  dotnet tool install --global wix
#
# STATUS: not yet exercised on a real Windows machine — TESTING.md is
# the verification checklist for whoever runs this first.

param([string]$Action = "", [switch]$NoCargo)
$ErrorActionPreference = "Stop"

$Windows = Split-Path -Parent $MyInvocation.MyCommand.Path
$Repo = (Resolve-Path (Join-Path $Windows "..")).Path

# The single source of truth for the version is the Rust workspace.
$CargoToml = Get-Content (Join-Path $Repo "rust\Cargo.toml") -Raw
if ($CargoToml -notmatch '(?ms)\[workspace\.package\].*?version\s*=\s*"([^"]+)"') {
    throw "couldn't find workspace.package version in rust\Cargo.toml"
}
$Version = $Matches[1]

if (-not $NoCargo) {
    & cargo build --release --manifest-path (Join-Path $Repo "rust\Cargo.toml")
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }
}

Set-Location $Windows
if (Test-Path build) { Remove-Item -Recurse -Force build }
$Dist = "build\Oh Brother"
New-Item -ItemType Directory -Force $Dist | Out-Null

$ReleaseDir = Join-Path $Repo "rust\target\release"
Copy-Item (Join-Path $ReleaseDir "label-app.exe") (Join-Path $Dist "Oh Brother.exe")
Copy-Item (Join-Path $ReleaseDir "label-web.exe") $Dist
Copy-Item (Join-Path $ReleaseDir "label.exe") $Dist

Write-Host "built $Dist (version $Version)"

function Assert-Wix {
    if (-not (Get-Command wix -ErrorAction SilentlyContinue)) {
        throw "WiX CLI not found. Install it with: dotnet tool install --global wix"
    }
    # Idempotent; makes -ext resolution work on a fresh machine/runner.
    & wix extension add --global WixToolset.Util.wixext/7.0.0 | Out-Null
    & wix extension add --global WixToolset.BootstrapperApplications.wixext/7.0.0 | Out-Null
}

function Build-Msi {
    Assert-Wix
    $DistAbs = (Resolve-Path $Dist).Path
    $Msi = "build\OhBrother-$Version-x64.msi"
    & wix build -arch x64 `
        -d "Version=$Version" -d "DistDir=$DistAbs" -d "WindowsDir=$Windows" `
        (Join-Path $Windows "installer\Package.wxs") -o $Msi
    if ($LASTEXITCODE -ne 0) { throw "wix build (msi) failed" }
    Write-Host "built $Msi"
    return $Msi
}

function Build-Bundle([string]$Msi) {
    Assert-Wix
    # The Evergreen Bootstrapper is a small stub whose contents change
    # server-side; fetch fresh at build time and compress it into the
    # bundle so we never pin a stale hash.
    $WV2 = "build\MicrosoftEdgeWebView2Setup.exe"
    if (-not (Test-Path $WV2)) {
        Invoke-WebRequest -Uri "https://go.microsoft.com/fwlink/p/?LinkId=2124703" -OutFile $WV2
    }
    $Bundle = "build\OhBrother-$Version-x64-setup.exe"
    & wix build -arch x64 `
        -ext WixToolset.Util.wixext/7.0.0 -ext WixToolset.BootstrapperApplications.wixext/7.0.0 `
        -d "Version=$Version" -d "Msi=$((Resolve-Path $Msi).Path)" `
        -d "WindowsDir=$Windows" -d "WebView2Bootstrapper=$((Resolve-Path $WV2).Path)" `
        (Join-Path $Windows "installer\Bundle.wxs") -o $Bundle
    if ($LASTEXITCODE -ne 0) { throw "wix build (bundle) failed" }
    Write-Host "built $Bundle"
    return $Bundle
}

switch ($Action) {
    "" { }
    "msi" { Build-Msi | Out-Null }
    "bundle" { Build-Bundle (Build-Msi) | Out-Null }
    "install" {
        $Dest = Join-Path $env:LOCALAPPDATA "Programs\Oh Brother"
        if (Test-Path $Dest) { Remove-Item -Recurse -Force $Dest }
        New-Item -ItemType Directory -Force (Split-Path $Dest) | Out-Null
        Copy-Item -Recurse $Dist $Dest
        $StartMenu = Join-Path ([Environment]::GetFolderPath("Programs")) "Oh Brother.lnk"
        $Shell = New-Object -ComObject WScript.Shell
        $Shortcut = $Shell.CreateShortcut($StartMenu)
        $Shortcut.TargetPath = Join-Path $Dest "Oh Brother.exe"
        $Shortcut.WorkingDirectory = $Dest
        $Shortcut.Save()
        Write-Host "installed to $Dest and the Start Menu"
    }
    default { throw "unknown action '$Action' (expected: install, msi, bundle)" }
}
