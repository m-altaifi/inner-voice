# Build inner-voice.  Anything you pass is forwarded to cargo:
#   .\build.ps1 build --release
#   .\build.ps1 test
#
# Three things on this machine need saying out loud, hence this script:
#   1. CUDA never integrated with VS 18's MSBuild ("No CUDA toolset found"),
#      so we generate with Ninja, which drives nvcc directly.
#   2. CUDA 12.8 only sanctions MSVC 2017-2022 and this box has VS 2026.
#   3. nvcc finds its host compiler via -ccbin and then runs vcvars64.bat to
#      set up the environment. Do NOT wrap this in a VS developer shell: that
#      PATH is ~7.5k characters and vcvars64.bat dies on cmd's 8191-char limit
#      ("The input line is too long"). cc-rs locates MSVC by itself anyway.

$ErrorActionPreference = 'Stop'
$vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
$vs = if ($env:IV_VS_PATH) { $env:IV_VS_PATH } elseif (Test-Path -LiteralPath $vswhere) {
    & $vswhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
} else { $null }
if (-not $vs) { throw 'Install Visual Studio C++ build tools, or set IV_VS_PATH.' }
$cm = "$vs\Common7\IDE\CommonExtensions\Microsoft\CMake"
$cuda = if ($env:CUDA_PATH) { $env:CUDA_PATH } else { "C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\v12.8" }
# A CPU build asks for none of the CUDA apparatus, so do not demand it. cmake
# and Ninja are still needed either way: whisper.cpp is built with cmake
# whichever backend it targets.
$wantsCuda = -not ($args -contains '--no-default-features')
$needed = @("$cm\CMake\bin\cmake.exe", "$cm\Ninja\ninja.exe")
if ($wantsCuda) { $needed += "$cuda\bin\nvcc.exe" }
foreach ($required in $needed) {
    if (-not (Test-Path -LiteralPath $required)) { throw "Missing build prerequisite: $required" }
}
$cargo = (Get-Command cargo.exe -ErrorAction Stop).Source
$previousPath = $env:PATH

# Minimal PATH, on purpose. The inherited one here is ~5.7k characters over 115
# entries; vcvars64.bat prepends to it and overruns cmd's 8191-char limit.
$env:PATH = @(
    "$cm\CMake\bin"
    "$cm\Ninja"
    "$cuda\bin"
    (Split-Path -Parent $cargo)
    "$env:SystemRoot\system32"
    $env:SystemRoot
    "$env:SystemRoot\System32\Wbem"
) -join ';'

$env:CMAKE_GENERATOR = "Ninja"
if (-not $env:CMAKE_CUDA_ARCHITECTURES) { $env:CMAKE_CUDA_ARCHITECTURES = "native" }

# 8.3 short path: no spaces, so it survives cmake's flag splitting unquoted,
# and it keeps the command line that vcvars64.bat inherits short.
$clDir = "$vs\VC\Tools\MSVC"
$clDir = Join-Path (Get-ChildItem $clDir | Sort-Object Name -Descending | Select-Object -First 1).FullName "bin\HostX64\x64"
$ccbin = (New-Object -ComObject Scripting.FileSystemObject).GetFolder($clDir).ShortPath
if (-not $env:CMAKE_CUDA_FLAGS) {
    $env:CMAKE_CUDA_FLAGS = "-allow-unsupported-compiler -ccbin $ccbin"
}

try {
    Push-Location $PSScriptRoot
    try {
        & $cargo @args
        $cargoExit = $LASTEXITCODE
    } finally { Pop-Location }
} finally { $env:PATH = $previousPath }
exit $cargoExit
