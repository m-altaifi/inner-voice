# Proves --hear isolates one app: two processes speak different sentences at
# the same time through the speakers, the panel hears only one of them, and
# only that sentence's words reach THEM. The microphone will pick up both from
# the speakers as YOU -- that is expected and is why only THEM lines are judged.
#
# Speech comes from SAPI in each process directly, so the audio session belongs
# to that process. The two shells have different image names -- `pwsh` and
# `powershell` -- which is what --hear keys on. Run this from powershell.exe.
param([string]$Exe = '.\target\release\inner-voice.exe')
$ErrorActionPreference = 'Stop'
if (-not (Get-Command pwsh.exe -ErrorAction SilentlyContinue)) { throw 'pwsh.exe (PowerShell 7) is needed as the second voice' }
$log = Join-Path $env:TEMP 'iv-hear-isolation'
Remove-Item -Recurse -Force $log -ErrorAction SilentlyContinue
Get-Process inner-voice -ErrorAction SilentlyContinue | Stop-Process -Force

$app = Start-Process -PassThru -FilePath $Exe -ArgumentList '--provider','none','--hear','pwsh','--log',$log
try {
    Start-Sleep -Seconds 30   # model load and warm-up; the panel says "hearing: waiting for pwsh..."
    $heard = 'The purple elephant is dancing in the kitchen tonight.'
    $decoy = 'An orange giraffe is reading a newspaper by the river.'
    $say = { param($shell, $text, $times) Start-Process -PassThru -FilePath $shell -ArgumentList '-NoProfile','-Command',"`$v = New-Object -ComObject SAPI.SpVoice; 1..$times | ForEach-Object { `$v.Speak('$text') | Out-Null; Start-Sleep 2 }" }
    # The decoy talks first and longest; the target repeats so the second
    # reading lands after --hear has hooked its (new) process.
    $d = & $say 'powershell.exe' $decoy 3
    $t = & $say 'pwsh.exe' $heard 3
    $t.WaitForExit(); $d.WaitForExit()
    Start-Sleep -Seconds 8   # hang + transcription of the last utterance
} finally {
    Stop-Process -Id $app.Id -Force -ErrorAction SilentlyContinue
}
$lines = Get-ChildItem $log -Filter *.jsonl | Get-Content
$them = ($lines | Where-Object { $_ -match '"who":"THEM"' }) -join ' '
"THEM heard: $them"
if ($them -notmatch 'elephant') { throw 'The named app was not transcribed as THEM' }
if ($them -match 'giraffe') { throw 'The decoy app leaked into THEM: --hear is not isolating' }
'PASS: --hear pwsh heard the elephant and not the giraffe'
