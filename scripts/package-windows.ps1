<#
.SYNOPSIS
    Build the Windows MSIX package for GGUF AI Engine.

.DESCRIPTION
    Assembles a staging layout and runs makeappx to produce a .msix.

    Layout (mirrors spec §15's run directory):

        shell/          WinUI 3 shell, self-contained
        core/           gguf-core, gguf, gguf-worker
        runtimes/       empty; runtimes are installed per-user at runtime
        Package.appxmanifest, Assets/

    The package is produced UNSIGNED. Installing it requires either developer
    mode or a signing certificate whose subject matches the manifest's
    Publisher. Signing is deliberately not automated here: a build script that
    invents a certificate would produce packages nobody should trust.

.PARAMETER SkipShell
    Skip the WinUI publish step (the Rust side can be packaged alone).

.PARAMETER OutDir
    Where to write the .msix. Defaults to target/package/windows.
#>
[CmdletBinding()]
param(
    [switch]$SkipShell,
    [string]$OutDir
)

$ErrorActionPreference = 'Stop'

$repo = Split-Path -Parent $PSScriptRoot
if (-not $OutDir) { $OutDir = Join-Path $repo 'target/package/windows' }
$stage = Join-Path $repo 'target/package/stage'

# --- locate makeappx from the Windows SDK -------------------------------
function Find-MakeAppx {
    $command = Get-Command makeappx.exe -ErrorAction SilentlyContinue
    if ($command) { return $command.Source }

    $kits = 'C:\Program Files (x86)\Windows Kits\10\bin'
    if (Test-Path $kits) {
        $candidate = Get-ChildItem $kits -Directory |
            Sort-Object Name -Descending |
            ForEach-Object { Join-Path $_.FullName 'x64/makeappx.exe' } |
            Where-Object { Test-Path $_ } |
            Select-Object -First 1
        if ($candidate) { return $candidate }
    }

    throw 'makeappx.exe not found. Install the Windows SDK (App packaging tools).'
}

$makeappx = Find-MakeAppx
Write-Output "makeappx  : $makeappx"

# --- version from Cargo -------------------------------------------------
$cargo = Get-Content (Join-Path $repo 'Cargo.toml') -Raw
if ($cargo -notmatch 'version\s*=\s*"([0-9]+\.[0-9]+\.[0-9]+)"') {
    throw 'could not read the workspace version from Cargo.toml'
}
$version = $Matches[1]
$msixVersion = "$version.0"
Write-Output "version   : $msixVersion"

# --- staging layout -----------------------------------------------------
if (Test-Path $stage) { Remove-Item -Recurse -Force $stage }
New-Item -ItemType Directory -Force -Path $stage | Out-Null
New-Item -ItemType Directory -Force -Path (Join-Path $stage 'core') | Out-Null
New-Item -ItemType Directory -Force -Path (Join-Path $stage 'Assets') | Out-Null
New-Item -ItemType Directory -Force -Path (Join-Path $stage 'runtimes') | Out-Null

# Rust binaries: the daemon, the CLI and the worker.
$release = Join-Path $repo 'target/release'
foreach ($binary in @('gguf-core.exe', 'gguf.exe', 'gguf-worker.exe')) {
    $source = Join-Path $release $binary
    if (-not (Test-Path $source)) {
        throw "missing $binary; run: cargo build --release -p gguf-engine-daemon -p gguf-engine-cli -p gguf-engine-runtime-worker"
    }
    Copy-Item $source (Join-Path $stage 'core') -Force
}

# WinUI shell, self-contained so no .NET install is required.
if (-not $SkipShell) {
    $shellProject = Join-Path $repo 'platforms/windows/GgufAiEngine.Shell'
    $publish = Join-Path $stage 'shell'
    & dotnet publish $shellProject -c Release -p:Platform=x64 -p:SelfContained=true -o $publish | Out-Null
    if ($LASTEXITCODE -ne 0) { throw 'dotnet publish failed for the shell' }
    Write-Output "shell     : published to $publish"
}

# Manifest: template with the real version substituted.
$manifestTemplate = Get-Content (Join-Path $repo 'packaging/windows/Package.appxmanifest') -Raw
$manifest = $manifestTemplate -replace 'Version="0\.1\.0\.0"', "Version=`"$msixVersion`""
# makeappx requires exactly this file name; "Package.appxmanifest" is the
# Visual Studio convention and is not accepted by the packer.
Set-Content -Path (Join-Path $stage 'AppxManifest.xml') -Value $manifest -Encoding UTF8

# Placeholder art. Real assets are a design task; a package without them will
# not build, so a 1x1 PNG is generated rather than shipping nothing.
Add-Type -AssemblyName System.Drawing
foreach ($asset in @('StoreLogo.png', 'Square150x150Logo.png', 'Square44x44Logo.png')) {
    $size = if ($asset -eq 'StoreLogo.png') { 50 } elseif ($asset -eq 'Square150x150Logo.png') { 150 } else { 44 }
    $bitmap = New-Object System.Drawing.Bitmap $size, $size
    $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
    $graphics.Clear([System.Drawing.Color]::FromArgb(255, 24, 24, 32))
    $graphics.Dispose()
    $bitmap.Save((Join-Path $stage "Assets/$asset"), [System.Drawing.Imaging.ImageFormat]::Png)
    $bitmap.Dispose()
}

# --- pack ---------------------------------------------------------------
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
$msix = Join-Path $OutDir "GGUFAIEngine-$version-x64.msix"
if (Test-Path $msix) { Remove-Item -Force $msix }

& $makeappx pack /d $stage /p $msix /o | Out-Null
if ($LASTEXITCODE -ne 0) { throw "makeappx failed with exit code $LASTEXITCODE" }

$info = Get-Item $msix
Write-Output "package   : $($info.FullName)"
Write-Output "size      : $([math]::Round($info.Length / 1MB, 1)) MB"
Write-Output ''
Write-Output 'The package is UNSIGNED. To install it:'
Write-Output '  * enable Developer Mode, or'
Write-Output '  * sign it with a certificate whose subject matches the manifest Publisher:'
Write-Output ("      signtool sign /fd SHA256 /a /f <cert.pfx> `"$msix`"")