# Build with .\build.ps1 build --release, then run in a fresh PowerShell process.
# Preview uses synthetic content and never opens audio, memory storage or a provider.
$ErrorActionPreference = 'Stop'
$ivRoot = Split-Path -Parent $PSScriptRoot
$ivPreviousDump = $env:IV_UI_DUMP
$ivPreview = $null
Push-Location $ivRoot
try {
    New-Item -ItemType Directory -Path 'target/implementation' -Force | Out-Null
    $env:IV_UI_DUMP = Join-Path $ivRoot 'target/implementation/ui-state.json'
    $ivExe = Join-Path $ivRoot 'target/release/inner-voice.exe'
    $ivPreview = Start-Process -FilePath $ivExe -ArgumentList '--preview' -WindowStyle Hidden -PassThru
    Start-Sleep -Seconds 2
    . .\tools\ui_smoke.ps1 -PreviewProcessId $ivPreview.Id -CloseAfterCheck
    if (-not $ivPreview.WaitForExit(5000)) { throw 'Main preview did not close' }
    foreach ($ivPane in @('memory', 'awareness', 'commitments', 'learning')) {
        $ivPreview = Start-Process -FilePath $ivExe -ArgumentList '--preview', '--preview-command', "/$ivPane" -WindowStyle Hidden -PassThru
        Start-Sleep -Seconds 2
        $preview = Get-Process -Id $ivPreview.Id
        $window = $preview.MainWindowHandle
        $state = Read-State
        if ($state.view -ne 118 -or $state.memory_title -notmatch $ivPane -or $state.memory_text -notmatch 'Preview only') {
            throw "$ivPane did not open its synthetic memory pane"
        }
        Capture-Window "preview-$ivPane.png"
        Press 112
        if (-not $ivPreview.WaitForExit(5000)) { throw "$ivPane preview did not close" }
    }
    'PASS: memory, awareness, commitments and learning preview panes opened and were captured.'
} finally {
    if ($ivPreview -and -not $ivPreview.HasExited) { $ivPreview.CloseMainWindow() | Out-Null }
    $env:IV_UI_DUMP = $ivPreviousDump
    Pop-Location
}
