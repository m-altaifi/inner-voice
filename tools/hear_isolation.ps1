# Proves --hear isolates one app: two processes speak different sentences at
# the same time through the speakers, the panel hears only one of them, and
# only that sentence's words reach the far end. The microphone will pick up both
# from the speakers as YOU -- that is expected, and is why only the far-end
# lines are judged.
#
# The far end is every log line that is *not* YOU, not the ones matching
# "who":"THEM": once the roster binds a name, a far-end turn is logged under
# that name instead ("who":"Sarah Chen" happened on a real run), so matching
# THEM would drop it silently -- failing the elephant check while --hear works,
# and missing a giraffe that leaked under a name. Log lines are
# {"t":...,"text":...,"who":...}, in that order.
#
# Speech comes from SAPI in each process directly, so the audio session belongs
# to that process. The two shells have different image names -- `pwsh` and
# `powershell` -- which is what --hear keys on. Run this from powershell.exe.
#
# Parameterised so the multi-app case is the same harness, not a second copy
# of it: -Hear takes the comma list --hear takes, and the two word sets say
# what must and must not reach the far end.
#   .\tools\hear_isolation.ps1                       # one app, the original run
#   .\tools\hear_isolation.ps1 -Hear 'pwsh,powershell' -Expect elephant,giraffe -Reject @()
param(
    [string]$Exe = '.\target\release\inner-voice.exe',
    [string]$Hear = 'pwsh',
    [string[]]$Expect = @('elephant'),
    [string[]]$Reject = @('giraffe')
)
$ErrorActionPreference = 'Stop'
if (-not (Get-Command pwsh.exe -ErrorAction SilentlyContinue)) { throw 'pwsh.exe (PowerShell 7) is needed as the second voice' }
$log = Join-Path $env:TEMP 'iv-hear-isolation'
# Kill before cleaning, and fail loudly if the clean did not happen: a stale
# inner-voice from an interrupted run holds its .jsonl open, the removal fails
# silently, the fresh app writes a second file beside it, and both are read at
# the end -- so last run's elephant would print PASS for a run that heard
# nothing at all.
Get-Process inner-voice -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Milliseconds 300
Remove-Item -Recurse -Force $log -ErrorAction SilentlyContinue
if (Test-Path $log) { throw "could not clear $log; a stale inner-voice may still hold it" }

$app = Start-Process -PassThru -FilePath $Exe -ArgumentList '--provider','none','--hear',$Hear,'--log',$log
try {
    Start-Sleep -Seconds 30   # model load and warm-up; the panel says "hearing: waiting for pwsh..."
    # An app that died in warm-up is otherwise reported as "not transcribed" 45 s later.
    if ($app.HasExited) { throw "inner-voice exited during warm-up (exit $($app.ExitCode)); check --hear, the model and devices" }
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
$far = ($lines | Where-Object { $_ -notmatch '"who":"YOU"' }) -join ' '
"far end heard: $far"
foreach ($word in $Expect) {
    if ($far -notmatch $word) { throw "A named app was not transcribed as the far end: '$word' is missing" }
}
foreach ($word in $Reject) {
    if ($far -match $word) { throw "An unnamed app leaked into the far end: '$word' is present" }
}
$not = if ($Reject) { " and not the $($Reject -join '/')" } else { '' }
"PASS: --hear $Hear heard the $($Expect -join '/')$not"
