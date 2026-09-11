# Offline, synthetic speech only; never uses the microphone or speakers.
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$out = Join-Path $root 'target/evaluation/audio'
New-Item -ItemType Directory -Force -Path $out | Out-Null
$cases = Get-Content -LiteralPath (Join-Path $root 'tests/fixtures/decision-scenarios.json') -Raw | ConvertFrom-Json
$voice = New-Object -ComObject SAPI.SpVoice
foreach ($case in $cases) {
    $stream = New-Object -ComObject SAPI.SpFileStream
    try {
        $stream.Format.Type = 18 # SAFT16kHz16BitMono
        $stream.Open((Join-Path $out ($case.id + '.wav')), 3, $false)
        $voice.AudioOutputStream = $stream
        $voice.Speak($case.speech) | Out-Null
    } finally { $stream.Close() }
}
Write-Output "Generated $($cases.Count) labelled synthetic clips in $out"
