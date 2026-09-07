# Proves a name learned in one session comes back in the next -- the whole
# claim of `people.rs`, and the one thing no unit test can reach: it needs real
# audio, a real embedding, and two separate processes with a file between them.
#
# The far end introduces itself out loud. `roster::introduced` takes the name,
# `route` binds it to the voice cluster, `remember` writes it through to the
# book, and a second run must load it and say so without anything being spoken
# again.
#
# It does NOT test whether two different people are told apart. Every Microsoft
# TTS voice shares a vocoder and they score 0.72-0.84 against *each other* --
# above the 0.70 SAME threshold -- so synthetic speech cannot demonstrate
# speaker discrimination in either direction. That still needs a real call with
# `--dump`, and `voiceid.rs`'s thresholds stay unvalidated until it happens.
# What this does prove is the path: heard -> bound -> saved -> reloaded.
#
#   .\tools\voice_memory.ps1
param(
    [string]$Exe = '.\target\release\inner-voice.exe',
    [string]$Hear = 'pwsh',
    [string]$Name = 'Ada Lovelace'
)
$ErrorActionPreference = 'Stop'
if (-not (Get-Command pwsh.exe -ErrorAction SilentlyContinue)) { throw 'pwsh.exe (PowerShell 7) is needed as the far-end voice' }

$book = Join-Path $env:TEMP 'iv-voice-memory.json'
$log = Join-Path $env:TEMP 'iv-voice-memory-log'
# Kill first, then clean, and fail loudly if the clean did not happen: a stale
# inner-voice holds the book open, so run two would "remember" run one's name
# without run one's write ever having been the thing that put it there.
Get-Process inner-voice -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Milliseconds 300
Remove-Item -Force $book, "$book.tmp" -ErrorAction SilentlyContinue
Remove-Item -Recurse -Force $log -ErrorAction SilentlyContinue
if (Test-Path $book) { throw "could not clear $book; a stale inner-voice may still hold it" }

# Long enough to embed: under 1.5 s of audio `voiceid` refuses to judge, so a
# curt "I'm Ada" would be transcribed, bound, and never reach a cluster.
$intro = "Hello everyone, I'm $Name, and I look after the platform team here."
# The sentence has to contain an apostrophe -- "I'm" is the cue `introduced`
# looks for -- and it is handed to a child shell inside a single-quoted string,
# where one apostrophe ends the string and the rest is a syntax error. The
# child then dies silently and the run looks like "heard nothing", which is
# indistinguishable from a broken --hear. Doubling is PowerShell's escape.
$spoken = $intro -replace "'", "''"

"--- run one: $Name introduces themselves ---"
$app = Start-Process -PassThru -FilePath $Exe -ArgumentList '--provider','none','--hear',$Hear,'--people',$book,'--log',$log
try {
    Start-Sleep -Seconds 30   # model load and CUDA warm-up
    if ($app.HasExited) { throw "inner-voice exited during warm-up (exit $($app.ExitCode))" }
    $say = Start-Process -PassThru -FilePath 'pwsh.exe' -ArgumentList '-NoProfile','-Command',
        "`$v = New-Object -ComObject SAPI.SpVoice; 1..4 | ForEach-Object { `$v.Speak('$spoken') | Out-Null; Start-Sleep 2 }"
    $say.WaitForExit()
    Start-Sleep -Seconds 8    # hang window plus the last utterance's transcription
} finally {
    Stop-Process -Id $app.Id -Force -ErrorAction SilentlyContinue
}

$heard = (Get-ChildItem $log -Filter *.jsonl -ErrorAction SilentlyContinue | Get-Content) -join ' '
"far end heard: $heard"
if (-not (Test-Path $book)) {
    throw "no book was written. The name was never bound -- check the transcript above for the introduction, and that models/campplus is present (--setup)"
}
$saved = Get-Content $book -Raw
# Names only. Each person carries a 512-float centroid, so printing the file
# buries the one fact this test is about under 20 KB of numbers.
"book: " + (($saved | ConvertFrom-Json).people | ForEach-Object { "$($_.name) ($($_.turns) turns)" }) -join ', '
if ($saved -notmatch [regex]::Escape($Name)) {
    throw "the book exists but does not contain '$Name'; whisper may have written the name differently -- see the transcript above"
}

"--- run two: nothing is spoken, the name must already be there ---"
# --setup rather than a live run: it reads the same book through the same
# loader and prints it, with no audio, no timing and nothing to race.
# whisper writes its model banner to stderr, which `Stop` turns into a
# terminating error even on exit code 0 -- the run would fail here having
# already proved everything it set out to.
$prior = $ErrorActionPreference
$ErrorActionPreference = 'Continue'
$setup = & $Exe --provider none --people $book --setup 2>&1 | Out-String
$ErrorActionPreference = $prior
$line = ($setup -split "`n" | Where-Object { $_ -match '\bpeople\b' }) -join ''
$line.Trim()
if ($line -notmatch [regex]::Escape($Name)) {
    throw "a second run did not remember '$Name'; the book was written but is not being read back"
}

Remove-Item -Force $book -ErrorAction SilentlyContinue
Remove-Item -Recurse -Force $log -ErrorAction SilentlyContinue
"PASS: '$Name' was learned from speech in one session and reloaded in the next"
