<#
.SYNOPSIS
    Regenerate (or verify) the C# protobuf/gRPC stubs for the Windows shell.

.DESCRIPTION
    The Windows shell compiles the same proto/ contract as the Rust core, so the
    two can never disagree about the wire format. The generated C# lives in
    platforms/windows/GgufAiEngine.Shell/Generated and is committed, because a
    WinUI project runs a C# pre-pass (XamlPreCompile) before any build target
    could produce those files.

    protoc is taken from the Grpc.Tools NuGet package the project already
    restores, so no separate protobuf install is required.

.PARAMETER Check
    Verify the committed stubs match proto/ without writing anything. Exits 1 on
    drift. Used by CI.
#>
[CmdletBinding()]
param([switch]$Check)

$ErrorActionPreference = 'Stop'

$repoRoot = Split-Path -Parent $PSScriptRoot
$projectDir = Join-Path $repoRoot 'platforms/windows/GgufAiEngine.Shell'
$protoDir = Join-Path $repoRoot 'proto'
$outDir = Join-Path $projectDir 'Generated'

function Get-ProtocTool {
    $packages = Join-Path $env:USERPROFILE '.nuget/packages/grpc.tools'
    if (-not (Test-Path $packages)) {
        throw "Grpc.Tools is not restored. Run dotnet restore in $projectDir first."
    }

    $version = Get-ChildItem $packages -Directory |
        Sort-Object { [version]$_.Name } -Descending | Select-Object -First 1
    $tools = Join-Path $version.FullName 'tools/windows_x64'
    $protoc = Join-Path $tools 'protoc.exe'
    $plugin = Join-Path $tools 'grpc_csharp_plugin.exe'

    if (-not (Test-Path $protoc)) { throw "protoc.exe not found in $tools" }
    if (-not (Test-Path $plugin)) { throw "grpc_csharp_plugin.exe not found in $tools" }

    return @{ Protoc = $protoc; Plugin = $plugin }
}

$tool = Get-ProtocTool

$temp = Join-Path ([System.IO.Path]::GetTempPath()) ('gguf-csharp-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Force -Path $temp | Out-Null

try {
    & $tool.Protoc "--proto_path=$protoDir" "--csharp_out=$temp" (Join-Path $protoDir 'common.proto')
    if ($LASTEXITCODE -ne 0) { throw 'protoc failed for common.proto' }

    foreach ($name in @('health', 'ui', 'inference', 'media')) {
        & $tool.Protoc "--proto_path=$protoDir" "--csharp_out=$temp" "--grpc_out=$temp" `
            "--plugin=protoc-gen-grpc=$($tool.Plugin)" '--grpc_opt=no_server' (Join-Path $protoDir "$name.proto")
        if ($LASTEXITCODE -ne 0) { throw "protoc failed for $name.proto" }
    }

    if ($Check) {
        # Compare normalised text, not raw bytes. .gitattributes checks .cs files
        # out with CRLF while protoc writes LF, so a byte hash reported "out of
        # date" on every Windows CI run while the content was identical - drift
        # and a line-ending difference must not look the same.
        $normalise = {
            param($path)
            [System.IO.File]::ReadAllText($path).Replace("`r`n", "`n")
        }

        $drift = @()
        foreach ($file in Get-ChildItem $temp -Filter '*.cs') {
            $committed = Join-Path $outDir $file.Name
            if (-not (Test-Path $committed)) {
                $drift += "$($file.Name): missing from Generated/"
                continue
            }

            if ((& $normalise $file.FullName) -ne (& $normalise $committed)) {
                $drift += "$($file.Name): differs from proto/"
            }
        }

        if ($drift.Count -gt 0) {
            Write-Error ("C# stubs are out of date:`n  " + ($drift -join "`n  ") +
                "`nRun scripts/generate-csharp-protos.ps1 and commit the result.")
            exit 1
        }

        Write-Output "C# stubs match proto/"
        exit 0
    }

    New-Item -ItemType Directory -Force -Path $outDir | Out-Null
    Get-ChildItem $outDir -Filter '*.cs' | Remove-Item -Force
    Copy-Item (Join-Path $temp '*.cs') $outDir -Force
    Write-Output "Generated $((Get-ChildItem $outDir -Filter '*.cs').Count) file(s) in $outDir"
}
finally {
    Remove-Item -Recurse -Force $temp -ErrorAction SilentlyContinue
}