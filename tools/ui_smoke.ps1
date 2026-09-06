param([Parameter(Mandatory=$true)][int]$PreviewProcessId, [switch]$CloseAfterCheck)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
Add-Type @'
using System;
using System.Runtime.InteropServices;
public class PreviewNative {
    [StructLayout(LayoutKind.Sequential)] public struct Rect { public int Left, Top, Right, Bottom; }
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out Rect r);
    [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr h, out Rect r);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern IntPtr GetWindowLongPtrW(IntPtr h, int index);
    [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint m, IntPtr w, IntPtr l);
    [DllImport("user32.dll")] public static extern bool RegisterHotKey(IntPtr h, int id, uint mods, uint vk);
    [DllImport("user32.dll")] public static extern bool UnregisterHotKey(IntPtr h, int id);
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr after, int x, int y, int cx, int cy, uint flags);
    [DllImport("user32.dll")] public static extern void mouse_event(uint f, uint x, uint y, uint d, UIntPtr e);
    [DllImport("kernel32.dll")] public static extern IntPtr GlobalAlloc(uint flags, UIntPtr bytes);
    [DllImport("kernel32.dll")] public static extern IntPtr GlobalLock(IntPtr h);
    [DllImport("kernel32.dll")] public static extern bool GlobalUnlock(IntPtr h);
}
'@
$preview = Get-Process -Id $PreviewProcessId
$window = $preview.MainWindowHandle
if ($window -eq 0) { throw 'Preview has no window' }
# The panel has no child controls to read with GetDlgItem, so it mirrors its
# state to IV_UI_DUMP instead. The caller must set that before launching.
$dump = $env:IV_UI_DUMP
if (-not $dump) { throw 'Set IV_UI_DUMP before launching the preview' }
function Read-State {
    $deadline = [DateTime]::UtcNow.AddSeconds(6)
    do {
        Start-Sleep -Milliseconds 100
        try { return (Get-Content -Raw $dump | ConvertFrom-Json) } catch {}
    } while ([DateTime]::UtcNow -lt $deadline)
    throw "No panel state at $dump"
}
# WM_HOTKEY takes exactly the path a real key press does: winit's message hook
# reads it off the queue and dispatches on the id. Synthetic keystrokes cannot
# be used here - injected input is dropped when the foreground window outranks
# the script, which silently failed against the Win32 build too.
function Press([int]$id) {
    [void][PreviewNative]::PostMessage($window, 0x312, [IntPtr]$id, [IntPtr]::Zero)
    Start-Sleep -Milliseconds 300
}
function Wait-For([scriptblock]$check, [string]$what) {
    $deadline = [DateTime]::UtcNow.AddSeconds(10)
    do {
        $state = Read-State
        if (& $check $state) { return $state }
        Start-Sleep -Milliseconds 150
    } while ([DateTime]::UtcNow -lt $deadline)
    throw $what
}
function Drop-File([string]$name) {
    $path = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot "..\$name"))
    $bytes = [Text.Encoding]::Unicode.GetBytes($path + [char]0 + [char]0)
    $drop = [PreviewNative]::GlobalAlloc(0x42, [UIntPtr]::new([uint64](20 + $bytes.Length)))
    $pointer = [PreviewNative]::GlobalLock($drop)
    [Runtime.InteropServices.Marshal]::WriteInt32($pointer, 0, 20)
    [Runtime.InteropServices.Marshal]::WriteInt32($pointer, 16, 1)
    [Runtime.InteropServices.Marshal]::Copy($bytes, 0, [IntPtr]::Add($pointer, 20), $bytes.Length)
    [void][PreviewNative]::GlobalUnlock($drop)
    [void][PreviewNative]::PostMessage($window, 0x233, $drop, [IntPtr]::Zero)
}
function Capture-Window([string]$name) {
    $rect = New-Object PreviewNative+Rect
    [void][PreviewNative]::GetWindowRect($window, [ref]$rect)
    $bitmap = New-Object System.Drawing.Bitmap ($rect.Right-$rect.Left), ($rect.Bottom-$rect.Top)
    $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
    try {
        $graphics.CopyFromScreen($rect.Left, $rect.Top, 0, 0, $bitmap.Size)
        $bitmap.Save((Join-Path $PSScriptRoot $name))
    } finally { $graphics.Dispose(); $bitmap.Dispose() }
}

$state = Read-State
if ($state.notice -match 'Another app already owns') { throw "Hotkeys were lost: $($state.notice)" }

# The overlay must never take the keyboard: that is the whole premise.
$outside = [PreviewNative]::GetForegroundWindow()
if ($outside -eq $window) { throw 'Panel stole the foreground on open' }
if (([PreviewNative]::GetWindowLongPtrW($window, -20).ToInt64() -band 0x08000000) -eq 0) {
    throw 'Panel is missing WS_EX_NOACTIVATE'
}

# Borderless: with no decorations the frame and the client area coincide.
$frame = New-Object PreviewNative+Rect
$client = New-Object PreviewNative+Rect
[void][PreviewNative]::GetWindowRect($window, [ref]$frame)
[void][PreviewNative]::GetClientRect($window, [ref]$client)
if (($frame.Right-$frame.Left) -ne $client.Right -or ($frame.Bottom-$frame.Top) -ne $client.Bottom) { throw 'Window still has non-client borders' }

# Clicking the panel must not pull focus off the call - the Win32 build could
# not manage that, and it is why WS_EX_NOACTIVATE is set. Injected mouse input
# is discarded when the foreground window outranks this script, and then the
# check would pass without a click ever happening, so say which one it was
# rather than reporting a green that means nothing.
$clicked = [PreviewNative]::SetCursorPos([int](($frame.Left+$frame.Right)/2), [int](($frame.Top+$frame.Bottom)/2))
if ($clicked) {
    [PreviewNative]::mouse_event(0x02,0,0,0,[UIntPtr]::Zero); Start-Sleep -Milliseconds 200
    [PreviewNative]::mouse_event(0x04,0,0,0,[UIntPtr]::Zero); Start-Sleep -Milliseconds 500
    if ([PreviewNative]::GetForegroundWindow() -ne $outside) { throw 'Clicking the panel stole the foreground' }
    # That click starts and ends a window drag. If the injected release went
    # missing the panel would still be following the cursor, and every position
    # assertion below would be measuring that instead of what it meant to.
    [PreviewNative]::mouse_event(0x04,0,0,0,[UIntPtr]::Zero)
    [void](Wait-For { param($s) -not $s.dragging } 'A drag from the injected click never ended')
} else {
    Write-Host 'SKIPPED: click test - this session cannot inject mouse input (SetCursorPos refused)'
}

# The app owns its hotkeys for real: registration is first-come process-wide,
# so a combination it holds must be refused to everyone else.
foreach ($vk in 0x70, 0x71, 0x7A) {          # Ctrl+Shift+F1, F2, F11
    if ([PreviewNative]::RegisterHotKey([IntPtr]::Zero, 900 + $vk, 0x0002 -bor 0x0004, $vk)) {
        [void][PreviewNative]::UnregisterHotKey([IntPtr]::Zero, 900 + $vk)
        throw "Panel does not actually own Ctrl+Shift+F$($vk - 0x6F)"
    }
}

# The conversation is always on screen, so a view switch only changes the main
# pane. 102 = whole conversation, 113 = key list, 101 = advice.
if ($state.turns -lt 1) { throw 'Preview seeded no conversation' }
if ($state.advice -notmatch 'ASK') { throw 'Preview seeded no advice' }
Press 102
if ((Read-State).view -ne 102) { throw 'Hotkey did not switch to the conversation' }
Press 113
if ((Read-State).view -ne 113) { throw 'Hotkey did not open the key list' }
Capture-Window 'preview-keys.png'
Press 101
if ((Read-State).view -ne 101) { throw 'Hotkey did not return to advice' }
Capture-Window 'preview-advice.png'

# Pause must not disturb the panel; under --preview the status line reports
# preview first, so this checks the toggle survives rather than its wording.
Press 105
Press 105
if ((Read-State).view -ne 101) { throw 'Pause disturbed the view' }

# References. Files arrive as WM_DROPFILES rather than through winit's OLE
# drop target, because OleInitialize needs an STA thread and main is already MTA
# for WASAPI - so this posts the same message the shell would.
Drop-File 'prompt.md'
$state = Wait-For { param($s) $s.references -match 'prompt.md' } 'File drop did not complete'
if ($state.view -ne 103) { throw 'A dropped file did not open the references pane' }
foreach ($extra in 'README.md', 'CLAUDE.md', 'LEDGER.md') { Drop-File $extra; Start-Sleep -Milliseconds 700 }
$state = Wait-For { param($s) $s.references -match 'LEDGER.md' } 'Extra reference drops did not complete'
if ($state.imports -lt 4) { throw "Import activity lost a file: $($state.imports)" }
Capture-Window 'preview-references.png'

# Moving and pinning. The drag gesture needs injected mouse input, which this
# session cannot produce at all - SetCursorPos itself is refused - so it is NOT
# checked here. What is: that the window is movable, and that the pin key flips
# the flag hit_test reads. The mapping from pointer to gesture is unit-tested in
# `the_whole_panel_is_a_grip_until_it_is_pinned`; dragging by hand is the only
# way to confirm the two ends meet.
[void](Wait-For { param($s) -not $s.dragging } 'Panel is still dragging; cannot test its position')
[void][PreviewNative]::GetWindowRect($window, [ref]$frame)
$moved = $frame.Left + 40
[void][PreviewNative]::SetWindowPos($window, [IntPtr]::Zero, $moved, $frame.Top + 30, 0, 0, 0x0001 -bor 0x0004 -bor 0x0010)
Start-Sleep -Milliseconds 400
$after = New-Object PreviewNative+Rect
[void][PreviewNative]::GetWindowRect($window, [ref]$after)
if ($after.Left -ne $moved) { throw "Panel did not move: wanted $moved, got $($after.Left)" }
if ((Read-State).pinned) { throw 'Panel starts pinned; it should start free to move' }
Press 114
if (-not (Read-State).pinned) { throw 'Pin hotkey did not pin the panel' }
Press 114
if ((Read-State).pinned) { throw 'Pin hotkey did not unpin the panel' }

# Clear references removes indexed content without touching the originals.
Press 107
$state = Wait-For { param($s) $s.references -notmatch 'prompt.md' } 'Clear references did not clear the preview'
if ($state.notice -notmatch 'cleared') { throw 'Clear references said nothing' }
Press 101

$focus = if ($clicked) { 'no focus theft on open or click' } else { 'no focus theft on open (click skipped)' }
"PASS: $focus, borderless layout, real hotkey ownership, hotkey dispatch, advice/key-list/conversation panes, pause toggle, file drop, window move, pin toggle, clear references. NOT covered: the drag gesture itself - drag the panel by hand." 
if ($CloseAfterCheck) { Press 112 }
