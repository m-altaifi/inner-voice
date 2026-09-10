param([ValidateRange(1,3600)][int]$Seconds=120)
$ErrorActionPreference='Stop'
$root=Split-Path -Parent $PSScriptRoot
$out=Join-Path $root 'target/evaluation'
New-Item -ItemType Directory -Force -Path $out | Out-Null
$binary=Get-ChildItem -LiteralPath (Join-Path $root 'target/release/deps') -Filter 'inner_voice-*.exe' |
    Sort-Object LastWriteTime -Descending | Select-Object -First 1
if (-not $binary) {throw 'Build the release tests first.'}
$previous=$env:IV_STRESS_SECONDS
$env:IV_STRESS_SECONDS=[string]$Seconds
try {
    $process=Start-Process -FilePath $binary.FullName -ArgumentList 'stress_synthetic_audio','--ignored','--nocapture' `
        -WorkingDirectory $root -WindowStyle Hidden -PassThru `
        -RedirectStandardOutput (Join-Path $out 'stress.stdout.txt') -RedirectStandardError (Join-Path $out 'stress.stderr.txt')
    $watch=[Diagnostics.Stopwatch]::StartNew()
    $samples=[Collections.Generic.List[object]]::new()
    try {
        while (-not $process.WaitForExit(2000)) {
            $process.Refresh()
            $samples.Add([PSCustomObject]@{seconds=$watch.Elapsed.TotalSeconds;working_set_bytes=$process.WorkingSet64;private_bytes=$process.PrivateMemorySize64;cpu_seconds=$process.TotalProcessorTime.TotalSeconds})
            if ($watch.Elapsed.TotalSeconds -gt $Seconds+180) {throw 'Audio stress exceeded its deadline.'}
        }
        $process.WaitForExit()
        $samples | ConvertTo-Json -Depth 3 | Set-Content -LiteralPath (Join-Path $out 'process-memory.json') -Encoding UTF8
        if ($process.ExitCode -ne 0) {throw "Stress test failed; see target/evaluation/stress.stderr.txt"}
    } finally {
        if (-not $process.HasExited) {$process.Kill();$process.WaitForExit()}
    }
} finally {$env:IV_STRESS_SECONDS=$previous}
Write-Output 'Completed GPU stress and process-memory sampling; reports in target/evaluation.'
