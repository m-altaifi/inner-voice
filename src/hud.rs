//! A hands-off call panel: every action has a system-wide hotkey, so the
//! call app keeps the keyboard and the mouse is never needed. See `KEYS`.
//!
//! One page, not a view-switcher. Advice, the turn it is answering, and the
//! question box are all per-turn, so you need them at the same time — pressing
//! a key to see what was just said is the same mid-sentence interruption as
//! reaching for the mouse. Only the things you consult *deliberately*
//! (references, diagnostics, research, the key list) take over the main pane.
use crate::{Msg, references::References, speak::Speaker};
use crossbeam_channel::{Receiver, Sender, unbounded};
use eframe::egui::{
    self, Color32, FontId, Key, RichText, ScrollArea, TextEdit, TextStyle, ViewportCommand,
};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
use windows::Win32::{
    Foundation::{HWND, POINT, RECT},
    UI::{
        Input::KeyboardAndMouse::{
            GetAsyncKeyState, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, RegisterHotKey,
            SetActiveWindow, VK_F1, VK_F2, VK_F3, VK_F4, VK_F5, VK_F6, VK_LBUTTON,
        },
        Shell::{DragAcceptFiles, DragFinish, DragQueryFileW, HDROP},
        WindowsAndMessaging::{
            GWL_EXSTYLE, GetCursorPos, GetForegroundWindow, GetWindowLongPtrW, GetWindowRect,
            SWP_NOACTIVATE, SWP_NOZORDER, SetForegroundWindow, SetWindowLongPtrW, SetWindowPos,
            WM_DROPFILES, WM_HOTKEY, WS_EX_NOACTIVATE,
        },
    },
};
use winit::platform::windows::EventLoopBuilderExtWindows;

const BG: Color32 = Color32::from_rgb(0x17, 0x1b, 0x22);
const FG: Color32 = Color32::from_rgb(0xe7, 0xed, 0xf3);
const MUTED: Color32 = Color32::from_rgb(0xb0, 0xbb, 0xc8);
const ACCENT: Color32 = Color32::from_rgb(0x7d, 0xc4, 0xff);
const AMBER: Color32 = Color32::from_rgb(0xff, 0xb4, 0x54);
const SOFT_GREEN: Color32 = Color32::from_rgb(0x9f, 0xd8, 0xa0);
const ADVICE: usize = 101;
const TRANSCRIPT: usize = 102;
const REFERENCES: usize = 103;
const DIAGNOSTICS: usize = 104;
const PAUSE: usize = 105;
const RESEARCH: usize = 106;
const CLEAR: usize = 107;
const ASK: usize = 108;
const COACH: usize = 109;
const SOURCES: usize = 115;
const CANCEL: usize = 110;
const MINIMIZE: usize = 111;
const CLOSE: usize = 112;
const HELP: usize = 113;
const PIN: usize = 114;

/// Turns of conversation kept on screen above the advice, at minimum.
///
/// The pane scrolls and holds far more, but the panel is useless if the turn
/// the advice is answering has already scrolled away, so the layout reserves
/// room for this many before the advice gets whatever is left.
const VISIBLE_TURNS: f32 = 8.0;

/// How long the status line carries its `for keys` reminder after launch.
///
/// A permanent hint is permanent noise (LEDGER Decision 7); the empty advice
/// pane already names the key list, and the list is one key away.
const HINT_SECS: u64 = 10;

/// What the panel is doing, for the status line.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Mode {
    Preview,
    Paused,
    Listening { online: bool },
}

/// Where the current advice is in its wait, for the status line.
///
/// Doherty: the loop is ~1–2 s, well past the 400 ms "instant" line, so the
/// panel counts the wait instead of spinning — `thinking 1.4s` live, then
/// `first word 1.2s` frozen. The frozen number is the standing TTFT
/// instrument on every turn of a real call.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Wait {
    Idle,
    Thinking(f32),
    FirstWord(f32),
}

/// The status line, as a pure function so its grammar is testable.
///
/// Order: mode · model · wait · researching · pinned · hint. A named model
/// replaces "coaching on" rather than joining it; the hint is always last.
#[allow(clippy::too_many_arguments)]
fn status_text(
    mode: Mode,
    wait: Wait,
    model: Option<&str>,
    researching: bool,
    pinned: bool,
    hint: bool,
    coaching: bool,
) -> String {
    let mut s = match (mode, model) {
        (Mode::Preview, _) => "Preview — microphone off · no online requests".to_string(),
        (Mode::Paused, _) => "Paused — audio is not being transcribed".to_string(),
        // Silence has two causes once the panel can be left running all day —
        // nothing worth saying, and advice not armed — and they look identical
        // on screen. Only one of them is a fault, so the line says which.
        (Mode::Listening { online: true }, _) if !coaching => {
            format!("Listening · advice on request ({} asks)", fkey(ASK))
        }
        (Mode::Listening { online: true }, Some(model)) => format!("Listening · {model}"),
        (Mode::Listening { online: true }, None) => "Listening · coaching on".to_string(),
        (Mode::Listening { online: false }, _) => "Listening · transcription only".to_string(),
    };
    match wait {
        Wait::Idle => {}
        Wait::Thinking(secs) => s.push_str(&format!(" · thinking {secs:.1}s")),
        Wait::FirstWord(secs) => s.push_str(&format!(" · first word {secs:.1}s")),
    }
    if researching {
        s.push_str(" · researching");
    }
    if pinned {
        s.push_str(" · pinned");
    }
    if hint {
        s.push_str(&format!("   ·   Ctrl+Shift+{} for keys", fkey(HELP)));
    }
    s
}

/// Every action, and the system-wide key that reaches it.
///
/// This panel is an always-on-top overlay used while the *call* app owns the
/// keyboard, so `RegisterHotKey` — which delivers to `hwnd` regardless of who
/// is focused — is the primary interface, not an accelerator for one. A button
/// is only reachable after stealing focus from the call, which is the thing
/// the window exists to avoid.
///
/// Quit is deliberately absent. Alt+F4 already closes the window, is muscle
/// memory, and cannot be hit by fumbling an adjacent F-key mid-sentence.
///
/// Six, not twelve. Every one of the twelve was defensible on its own and the
/// set was not: a panel whose whole claim is *don't make me look away* had a
/// key list you had to look away to read. What survives is what a working day
/// actually presses, and only what must never steal the keyboard from the app
/// being listened to -- pausing and hiding especially, which are the two you
/// reach for when something private happens. The rest became `/` commands: no
/// action was removed, and `every_action_is_reachable_by_exactly_one_key`
/// holds the line that none may lose both.
///
/// F7..F12 are deliberately left unregistered. `RegisterHotKey` is
/// first-come process-wide, so six rows we do not need were six combinations
/// taken from every other app on the machine.
const KEYS: [(u32, usize, &str); 6] = [
    (VK_F1.0 as u32, ADVICE, "Back to advice"),
    (VK_F2.0 as u32, ASK, "Ask, or type a / command"),
    (VK_F3.0 as u32, COACH, "Advice: armed / on request only"),
    (VK_F4.0 as u32, PAUSE, "Pause / resume listening"),
    (VK_F5.0 as u32, MINIMIZE, "Hide / show the panel"),
    (VK_F6.0 as u32, HELP, "This list"),
];

/// A command the panel runs itself needs no `id`; `FORWARD` marks the ones
/// `route` owns, which fall through to `Msg::Question` exactly as before.
const FORWARD: usize = 0;

/// Typed into the question box, for everything a key is the wrong shape for:
/// actions that need a *name*, actions rare enough that a key is dead weight,
/// and actions destructive enough that a fumbled F-key mid-sentence should not
/// reach them.
///
/// This table is the dispatcher as well as the help text. A command used to be
/// an `if text == ...` arm that only the help table knew about, which is two
/// places to forget; now adding one is a row.
const COMMANDS: [(&str, usize, &str); 13] = [
    ("/sources", SOURCES, "pick which apps are heard, from a list"),
    ("/who", FORWARD, "who the panel can put a name to"),
    (
        "/who <name>",
        FORWARD,
        "name the voice that just spoke; remembered on the next call",
    ),
    ("/hear", FORWARD, "report which apps are heard as THEM"),
    (
        "/hear <app>[,<app>]",
        FORWARD,
        "hear only these; an app not yet running is waited for",
    ),
    ("/hear off", FORWARD, "back to the whole speaker mix"),
    ("/transcript", TRANSCRIPT, "the whole conversation"),
    ("/references", REFERENCES, "the files dropped on the panel"),
    ("/research", RESEARCH, "research the last turn"),
    ("/cancel", CANCEL, "stop the research running now"),
    ("/pin", PIN, "pin the panel where it is"),
    ("/clear", CLEAR, "clear references"),
    ("/diagnostics", DIAGNOSTICS, "the diagnostics log"),
];

/// `F2`, for the sentences that name a key. Written once because six of them
/// used to spell the number out, and moving a row silently made each one lie.
fn fkey(id: usize) -> String {
    KEYS.iter()
        .find(|(_, k, _)| *k == id)
        .map(|(vk, _, _)| format!("F{}", vk - VK_F1.0 as u32 + 1))
        .unwrap_or_default()
}

fn legend() -> Vec<(String, &'static str)> {
    KEYS.iter()
        .map(|(vk, _, what)| (format!("Ctrl+Shift+F{}", vk - VK_F1.0 as u32 + 1), *what))
        .collect()
}

pub struct Session {
    pub preview: bool,
    pub research_enabled: bool,
    pub online: bool,
    /// Provider model in the status line, so the on-screen TTFT is attributable.
    pub model: Option<String>,
    pub references: References,
    pub epoch: Arc<AtomicU64>,
    /// The selection the Sources pane reads and writes. Shared with the capture
    /// threads, which is what makes a click take effect without a restart.
    pub tune: Option<Arc<crate::audio::Tune>>,
    /// Loopback device *name*, not a `Device`: COM interfaces are not `Send`,
    /// so the pane opens it on this thread when it needs the app list — the
    /// same rule every capture thread follows.
    pub loopback: String,
}

/// What a drag on the window chrome should do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Grab {
    Move,
    Resize(egui::ResizeDirection),
}

/// A drag the panel is running itself, in physical screen pixels.
///
/// `ViewportCommand::StartDrag` cannot be used here. It posts
/// `WM_NCLBUTTONDOWN`/`HTCAPTION` and leaves the rest to Windows' modal move
/// loop, which wants the window to be the foreground one and wants mouse
/// capture — and this window is `WS_EX_NOACTIVATE` exactly so that it never
/// becomes foreground. The result was a panel that could only be dragged after
/// being activated from the taskbar, which is the thing the flag exists to
/// avoid. Following the cursor by hand needs no capture, no activation and no
/// modal loop, so it works while the call keeps the keyboard.
struct Dragging {
    grab: Grab,
    /// Cursor when the drag started.
    from: POINT,
    /// Window rect when the drag started.
    rect: (i32, i32, i32, i32),
}

/// Where a drag of `(dx, dy)` puts a window that started at `rect`.
///
/// Resizing pushes only the edges the grabbed corner owns, and each edge stops
/// against `min` rather than crossing its opposite — `SetWindowPos` takes the
/// size it is given, so nothing else clamps this.
fn dragged(
    grab: Grab,
    rect: (i32, i32, i32, i32),
    dx: i32,
    dy: i32,
    min: (i32, i32),
) -> (i32, i32, i32, i32) {
    use egui::ResizeDirection as R;
    let (mut left, mut top, mut right, mut bottom) = rect;
    let direction = match grab {
        Grab::Move => return (left + dx, top + dy, right + dx, bottom + dy),
        Grab::Resize(direction) => direction,
    };
    if matches!(direction, R::West | R::NorthWest | R::SouthWest) {
        left = (left + dx).min(right - min.0);
    }
    if matches!(direction, R::East | R::NorthEast | R::SouthEast) {
        right = (right + dx).max(left + min.0);
    }
    if matches!(direction, R::North | R::NorthWest | R::NorthEast) {
        top = (top + dy).min(bottom - min.1);
    }
    if matches!(direction, R::South | R::SouthWest | R::SouthEast) {
        bottom = (bottom + dy).max(top + min.1);
    }
    (left, top, right, bottom)
}

/// Map a pointer inside the borderless window to a move or resize gesture.
///
/// The window has no frame of its own, so the edges are reconstructed: six
/// pixels in from any side resizes, and *everything else drags*. A title strip
/// would be a few pixels of a panel that is otherwise wall-to-wall text, and
/// this window is positioned around whatever the call is showing — so the grip
/// is the whole thing. Selectable labels are turned off to make that possible;
/// see `paint_style`.
///
/// Pinning freezes both gestures. It is the counterpart to grabbing anywhere:
/// once the panel is where you want it, nothing should be able to shove it.
/// Kept pure because it is the one piece of the chrome worth a test.
fn hit_test(x: f32, y: f32, width: f32, height: f32, pinned: bool) -> Option<Grab> {
    use egui::ResizeDirection as R;
    if pinned {
        return None;
    }
    let (left, right, top, bottom) = (x < 6.0, x >= width - 6.0, y < 6.0, y >= height - 6.0);
    let edge = match (left, right, top, bottom) {
        (true, _, true, _) => Some(R::NorthWest),
        (_, true, true, _) => Some(R::NorthEast),
        (true, _, _, true) => Some(R::SouthWest),
        (_, true, _, true) => Some(R::SouthEast),
        (true, ..) => Some(R::West),
        (_, true, ..) => Some(R::East),
        (_, _, true, _) => Some(R::North),
        (_, _, _, true) => Some(R::South),
        _ => None,
    };
    Some(edge.map_or(Grab::Move, Grab::Resize))
}

/// The four line tags `prompt.md` emits, and how the panel shows each.
///
/// This table is the only place the tag vocabulary lives on the panel side:
/// change it in one place and it silently renders as body text in the other.
/// The literals are matched exactly, case-sensitively and only when followed
/// by whitespace, so a sentence beginning "ASKING" is left alone.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Tag {
    Ask,
    Say,
    Note,
    Fix,
}

impl Tag {
    const TABLE: [(Tag, &'static str, &'static str); 4] = [
        (Tag::Ask, "ASK", "ASK NEXT"),
        (Tag::Say, "SAY", "SUGGESTED WORDING"),
        (Tag::Note, "NOTE", "KEEP IN MIND"),
        (Tag::Fix, "FIX", "CLARIFY"),
    ];
    fn heading(self) -> &'static str {
        Self::TABLE
            .iter()
            .find(|(tag, ..)| *tag == self)
            .map_or("", |(_, _, heading)| *heading)
    }
    /// `(heading, body)` colours. Von Restorff: only `ASK` and `FIX` are
    /// saturated, because colouring every line makes nothing stand out.
    /// `NOTE` recedes entirely — it is the thing to hold, not the thing to do.
    fn colours(self) -> (Color32, Color32) {
        match self {
            Tag::Ask => (ACCENT, FG),
            Tag::Fix => (AMBER, FG),
            Tag::Say => (SOFT_GREEN, FG),
            Tag::Note => (MUTED, MUTED),
        }
    }
}

/// Split advice into tagged sections the panel can style.
fn display_advice(text: &str) -> Vec<(Option<Tag>, String)> {
    text.lines()
        .map(|line| {
            let line = line.trim();
            for (tag, literal, _) in Tag::TABLE {
                if let Some(rest) = line
                    .strip_prefix(literal)
                    .filter(|s| s.starts_with(char::is_whitespace))
                {
                    return (Some(tag), rest.trim().to_string());
                }
            }
            (None, line.to_string())
        })
        .filter(|(tag, body)| tag.is_some() || !body.is_empty())
        .collect()
}

fn remember<T>(lines: &mut VecDeque<T>, item: T) {
    lines.push_back(item);
    while lines.len() > 200 {
        lines.pop_front();
    }
}
fn joined(lines: &VecDeque<String>) -> String {
    lines.iter().cloned().collect::<Vec<_>>().join("\n\n")
}

struct State {
    rx: Receiver<Msg>,
    tx: Sender<Msg>,
    hotkeys: Receiver<usize>,
    drops: Receiver<Vec<PathBuf>>,
    session: Session,
    speaker: Option<Speaker>,
    hwnd: HWND,
    /// Who had the keyboard before `ASK` borrowed it, so it can be handed back.
    prior: Option<HWND>,
    /// True only while the question box is meant to receive keystrokes.
    typing: bool,
    /// Frozen in place: no drag, no resize.
    pinned: bool,
    drag: Option<Dragging>,
    /// When the panel opened; the key hint leaves `HINT_SECS` later.
    launched: Instant,
    /// When the live generation's `AdviceStart` arrived.
    asked_at: Option<Instant>,
    /// When its first token landed. `asked_at → first_word_at` is the TTFT.
    first_word_at: Option<Instant>,
    alpha: f32,
    notice: String,
    question: String,
    focus_question: bool,
    view: usize,
    paused: bool,
    /// Apps on the loopback device, as (name, playing now). Refreshed only while
    /// the Sources pane is open — enumerating WASAPI sessions every frame would
    /// put COM work on the render loop for a pane nobody is looking at.
    sources: Vec<(String, bool)>,
    sources_read: Option<std::time::Instant>,
    /// Whether a far-end turn still asks the coach on its own. Owned by
    /// `route`, mirrored here for the status line only.
    coaching: bool,
    seq: u64,
    thinking: bool,
    advice: String,
    research: String,
    researching: bool,
    research_id: u64,
    transcript: VecDeque<(String, String)>,
    diagnostics: VecDeque<String>,
    imports: VecDeque<String>,
    /// Files dropped while online, waiting on the excerpt-sharing consent.
    pending: Vec<PathBuf>,
    /// Where to mirror panel state for `tools/ui_smoke.ps1`, from `IV_UI_DUMP`.
    ///
    /// egui draws into one window with no child controls, so a test script has
    /// nothing to read with `GetDlgItem` the way it could with the Win32 build.
    /// This mirrors the *state* a hotkey changed — deliberately not the render,
    /// which would be a second copy of the layout free to disagree with the
    /// screen; screenshots are what check the drawing.
    dump: Option<(PathBuf, String)>,
}

impl State {
    /// Hold `WS_EX_NOACTIVATE` on the window unless the question box wants it off.
    ///
    /// The flag is what keeps a click on the panel from pulling focus off the
    /// call, and it is equally what stops a keystroke ever reaching the question
    /// box — so it tracks `typing` rather than being set once.
    ///
    /// It has to be re-asserted every frame: winit recomputes the extended style
    /// whenever the window changes state and has no notion of this flag, so a
    /// one-shot set at startup is silently dropped the first time anything moves
    /// (which is how it was lost the first time). Costs a `GetWindowLongPtrW`
    /// and, almost always, nothing else.
    fn keep_unfocusable(&self) {
        unsafe {
            let style = GetWindowLongPtrW(self.hwnd, GWL_EXSTYLE);
            let flag = WS_EX_NOACTIVATE.0 as isize;
            let want = if self.typing {
                style & !flag
            } else {
                style | flag
            };
            if want != style {
                SetWindowLongPtrW(self.hwnd, GWL_EXSTYLE, want);
            }
        }
    }
    /// Let the panel take the keyboard, remembering who had it.
    ///
    /// Typing is the one thing a hotkey cannot do for you, so this is the only
    /// moment the panel is allowed to become the foreground window.
    fn borrow_keyboard(&mut self) {
        self.typing = true;
        self.keep_unfocusable();
        unsafe {
            let now = GetForegroundWindow();
            if now != self.hwnd {
                self.prior = Some(now);
            }
            let _ = SetForegroundWindow(self.hwnd);
            let _ = SetActiveWindow(self.hwnd);
        }
        self.focus_question = true;
    }
    /// Give the keyboard back to whoever had it, and stop activating again.
    fn return_keyboard(&mut self) {
        self.typing = false;
        self.keep_unfocusable();
        if let Some(prior) = self.prior.take() {
            unsafe {
                let _ = SetForegroundWindow(prior);
            }
        }
    }
    fn pump(&mut self) {
        for _ in 0..100 {
            let Ok(message) = self.rx.try_recv() else {
                break;
            };
            match message {
                Msg::Turn(who, text) => {
                    remember(&mut self.transcript, (who.label().to_string(), text))
                }
                Msg::Sys(text) => {
                    if text.contains("failed")
                        || text.contains("stopped")
                        || text.starts_with("coach:")
                        || text.starts_with("research off")
                        || text.starts_with("Preview")
                        || text.starts_with("hearing:")
                        || text.starts_with("speak:")
                        || text.starts_with("naming:")
                        || text.starts_with("knowledge")
                    {
                        self.notice = text.clone();
                    }
                    remember(&mut self.diagnostics, text);
                }
                Msg::ReferenceStatus(text) => {
                    self.notice = text.clone();
                    remember(&mut self.imports, text);
                }
                Msg::Coaching(on) => {
                    self.coaching = on;
                    let text = match on {
                        true => "coach: advice armed — every far-end turn asks",
                        false => &format!(
                            "coach: advice on request only — Ctrl+Shift+{} asks, /research researches",
                            fkey(ASK)
                        ),
                    };
                    self.notice = text.into();
                    remember(&mut self.diagnostics, text.to_string());
                }
                Msg::AdviceStart(id) => {
                    self.seq = id;
                    self.thinking = true;
                    self.asked_at = Some(Instant::now());
                    self.first_word_at = None;
                }
                Msg::Advice(id, text) if id == self.seq => {
                    if self.thinking {
                        self.advice.clear();
                        self.thinking = false;
                        self.first_word_at = Some(Instant::now());
                    }
                    self.advice.push_str(&text);
                }
                Msg::AdviceEnd(id) if id == self.seq => {
                    // A cancelled generation ends without ever streaming, so
                    // there is nothing to read out; still being in `thinking`
                    // is what distinguishes that from a finished one. Retiring
                    // `seq` makes a repeated end for the same id a no-op rather
                    // than a second reading of the last advice.
                    let finished = !self.thinking;
                    self.thinking = false;
                    self.seq = u64::MAX;
                    if finished
                        && !self.paused
                        && let Some(speaker) = &self.speaker
                    {
                        speaker.say(&self.advice);
                    }
                }
                Msg::ToolStart(id) => {
                    self.research_id = id;
                    self.researching = true;
                    self.notice = "Research is running. You can keep reading live advice.".into();
                }
                Msg::ToolEnd(id, result) if id == self.research_id => {
                    self.researching = false;
                    // "finished" over a cancelled or failed job contradicts the
                    // pane the same sentence sends you to, so the notice
                    // follows the outcome.
                    self.notice = match &result {
                        Ok(_) => "Research finished — /research shows it.",
                        Err(_) => "Research stopped — /research shows why.",
                    }
                    .into();
                    self.research =
                        result.unwrap_or_else(|e| format!("Research couldn't finish\n\n{e}"));
                }
                _ => {}
            }
        }
    }
    fn command(&mut self, id: usize, ctx: &egui::Context) {
        match id {
            ADVICE | TRANSCRIPT | REFERENCES | DIAGNOSTICS | HELP | SOURCES => self.view = id,
            MINIMIZE => {
                let hidden = ctx.input(|i| i.viewport().minimized.unwrap_or(false));
                ctx.send_viewport_cmd(ViewportCommand::Minimized(!hidden));
            }
            CLOSE => ctx.send_viewport_cmd(ViewportCommand::Close),
            PIN => self.pinned = !self.pinned,
            PAUSE => {
                self.paused = !self.paused;
                self.session.epoch.fetch_add(1, Ordering::SeqCst);
                self.seq = u64::MAX;
                self.thinking = false;
                self.asked_at = None;
                self.first_word_at = None;
                let _ = self.tx.send(Msg::Pause(self.paused));
            }
            // From another view this only brings the result up; pressing it
            // while already looking at one starts a fresh job.
            RESEARCH => {
                let looking = self.view == RESEARCH;
                self.view = RESEARCH;
                if !self.researching && (looking || self.research.is_empty()) {
                    self.research.clear();
                    let _ = self.tx.send(Msg::Research);
                }
            }
            CANCEL => {
                let _ = self.tx.send(Msg::CancelResearch);
                self.researching = false;
            }
            // Frequent and reversible, unlike the key it took over. `route`
            // owns the setting and echoes it back, which is what refreshes the
            // notice and the status line.
            COACH => {
                self.coaching = !self.coaching;
                let _ = self.tx.send(Msg::Coaching(self.coaching));
            }
            CLEAR => {
                self.session.references.clear();
                self.imports.clear();
                // Synchronous on the keypress: references.rs sends its longer
                // sentence a frame later, and `ui_smoke.ps1` reads `cleared`
                // straight after the press.
                self.notice = "References cleared.".into();
            }
            // Typing is the one thing a hotkey cannot do for you, so the key
            // first brings the caret here and only sends once there is text.
            ASK => {
                let text = self.question.trim().to_string();
                // Run here rather than in `route` because each command belongs
                // to whoever owns the state it changes — a clear also empties
                // the panel's own import list, which `route` cannot reach.
                // `FORWARD` rows fall through to `Msg::Question` untouched.
                if let Some(&(_, target, _)) = COMMANDS
                    .iter()
                    .find(|(name, target, _)| *name == text && *target != FORWARD)
                {
                    self.question.clear();
                    // `return_keyboard`, not `typing = false`. The two arms this
                    // replaced left the panel foreground with `prior` unrestored,
                    // so typing a command took the call app's keyboard and never
                    // handed it back; most actions reach the user through this
                    // path now, which is what made the bug worth finding.
                    self.return_keyboard();
                    self.command(target, ctx);
                    return;
                }
                if text.is_empty() {
                    self.borrow_keyboard();
                } else {
                    let _ = self.tx.send(Msg::Question(text));
                    self.question.clear();
                    self.view = ADVICE;
                    self.return_keyboard();
                }
            }
            _ => {}
        }
    }
    fn wait(&self) -> Wait {
        match (self.asked_at, self.first_word_at) {
            (Some(asked), Some(first)) => {
                Wait::FirstWord(first.duration_since(asked).as_secs_f32())
            }
            (Some(asked), None) if self.thinking => Wait::Thinking(asked.elapsed().as_secs_f32()),
            _ => Wait::Idle,
        }
    }
    /// The app picker. A list you click, which is what a selection should be —
    /// `/hear` still exists for the one thing a list cannot do, naming an app
    /// that has not started yet and so has no audio session to appear in.
    ///
    /// Clickable without focus: a `WS_EX_NOACTIVATE` window still receives
    /// mouse input, which is how dragging already works. Nothing here takes the
    /// keyboard from whatever you are actually in.
    fn sources_pane(&mut self, ui: &mut egui::Ui) {
        let Some(tune) = self.session.tune.clone() else {
            ui.label(RichText::new("No audio in this mode.").color(MUTED));
            return;
        };
        self.refresh_sources();
        let selected = tune.hearing();
        let matches = |app: &str| {
            let app = app.to_lowercase();
            selected.iter().any(|s| app.contains(&s.to_lowercase()))
        };

        ui.label(
            RichText::new("Click to choose what THEM is. Several can be on at once.").color(MUTED),
        );
        ui.add_space(8.0);
        if ui
            .selectable_label(selected.is_empty(), RichText::new("Whole speaker mix").size(16.0))
            .clicked()
        {
            tune.hear_only(Vec::new());
            self.notice = "hearing: the whole speaker mix (up to 2 s)".into();
        }
        ui.add_space(4.0);
        for (name, live) in self.sources.clone() {
            let label = match live {
                true => RichText::new(&name).size(16.0),
                // Selected but silent: it was named before it started, or it
                // stopped. The stream is still waiting for it either way.
                false => RichText::new(format!("{name}  — not playing")).size(16.0).color(MUTED),
            };
            if ui.selectable_label(matches(&name), label).clicked() {
                let now = tune.toggle(&name);
                self.notice = match now.is_empty() {
                    true => "hearing: the whole speaker mix (up to 2 s)".into(),
                    false => format!("hearing: {} (up to 2 s)", now.join(" + ")),
                };
            }
        }
        if self.sources.is_empty() {
            ui.label(
                RichText::new("Nothing is playing yet. Apps appear here as they make sound;                                to name one before it starts, type /hear <app> in the question box.")
                    .color(MUTED),
            );
        }
        ui.add_space(10.0);
        ui.label(
            RichText::new(format!("Listening on {}", self.session.loopback)).color(MUTED),
        );
    }

    /// Enumerate at most once a second, and only while the pane is open.
    fn refresh_sources(&mut self) {
        if self
            .sources_read
            .is_some_and(|t| t.elapsed() < Duration::from_secs(1))
        {
            return;
        }
        self.sources_read = Some(std::time::Instant::now());
        let mut rows: Vec<(String, bool)> = crate::audio::playing(&self.session.loopback)
            .unwrap_or_default()
            .into_iter()
            .map(|name| (name, true))
            .collect();
        // A selected app that is not playing still belongs on the list, or
        // turning it back off would mean typing a command to undo a click.
        if let Some(tune) = &self.session.tune {
            for chosen in tune.hearing() {
                let known = rows
                    .iter()
                    .any(|(n, _)| n.to_lowercase().contains(&chosen.to_lowercase()));
                if !known {
                    rows.push((chosen, false));
                }
            }
        }
        self.sources = rows;
    }

    fn status_line(&self) -> String {
        let mode = if self.session.preview {
            Mode::Preview
        } else if self.paused {
            Mode::Paused
        } else {
            Mode::Listening {
                online: self.session.online,
            }
        };
        status_text(
            mode,
            self.wait(),
            self.session.model.as_deref(),
            self.researching,
            self.pinned,
            self.launched.elapsed() < Duration::from_secs(HINT_SECS),
            self.coaching,
        )
    }
    /// The conversation, newest last. Always on screen: the advice is an answer
    /// to the last turn, and an answer without its question is a riddle.
    fn conversation(&mut self, ui: &mut egui::Ui) {
        ScrollArea::vertical()
            .id_salt("conversation")
            // A `ScrollArea` refuses to be under 64 px tall by default, which is
            // taller than this strip is for the first two turns. It then
            // overflowed its panel, the panel reported a rect starting *below*
            // its own top edge, and the central panel painted its background
            // over the only row there was. The strip's own height is the
            // authority here, so let the scroll area be as short as the panel.
            .min_scrolled_height(0.0)
            .stick_to_bottom(true)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if self.transcript.is_empty() {
                    ui.label(
                        RichText::new("Your conversation will appear here.")
                            .color(MUTED)
                            .italics(),
                    );
                }
                for (who, text) in &self.transcript {
                    // Miller: THEM brighter than YOU, because THEM is what you
                    // react to. Your own words are context, not a prompt.
                    let (label, body) = if who == "YOU" {
                        (MUTED, MUTED)
                    } else {
                        (ACCENT, FG)
                    };
                    ui.horizontal_top(|ui| {
                        ui.add_sized(
                            [72.0, ui.text_style_height(&TextStyle::Body)],
                            egui::Label::new(RichText::new(who).color(label).monospace()),
                        );
                        // The body goes in a *vertical* child, not straight into
                        // this horizontal one. A horizontal layout hands its
                        // children unbounded width, so a long turn ran off the
                        // right-hand edge instead of wrapping — and a panel
                        // whose sentences you cannot read the end of is worse
                        // than one that is too tall. A vertical child is where
                        // a paragraph belongs: it gets the width that is left
                        // and wraps into it. `--preview` seeds a turn longer
                        // than the panel so a screenshot can show this; every
                        // seeded line used to fit on one row, which is why no
                        // screenshot ever caught it.
                        ui.vertical(|ui| {
                            ui.label(RichText::new(text).color(body));
                        });
                    });
                }
            });
    }
    /// The big pane: advice by default, or whatever was deliberately opened.
    fn main_pane(&mut self, ui: &mut egui::Ui) {
        ScrollArea::vertical()
            .id_salt("main")
            // Advice streams in, so it wants the newest text; egui unsticks by
            // itself when the reader scrolls up and re-sticks at the end. The
            // other views are documents — they start at the top and stay.
            .stick_to_bottom(self.view == ADVICE)
            .auto_shrink([false, false])
            .show(ui, |ui| match self.view {
                TRANSCRIPT => {
                    for (who, text) in &self.transcript {
                        let (label, body) = if who == "YOU" { (MUTED, MUTED) } else { (ACCENT, FG) };
                        ui.label(RichText::new(who).color(label).monospace());
                        ui.label(RichText::new(text).color(body));
                        ui.add_space(8.0);
                    }
                }
                REFERENCES => {
                    ui.label(RichText::new(self.session.references.preview()).color(FG));
                    ui.add_space(12.0);
                    ui.label(RichText::new("IMPORT ACTIVITY").color(ACCENT).strong());
                    ui.label(RichText::new(joined(&self.imports)).color(MUTED));
                }
                SOURCES => self.sources_pane(ui),
                DIAGNOSTICS => {
                    ui.label(RichText::new(joined(&self.diagnostics)).color(MUTED));
                }
                HELP => {
                    ui.label(
                        RichText::new("These work while your call app has focus.").color(MUTED),
                    );
                    ui.add_space(8.0);
                    egui::Grid::new("keys").spacing([18.0, 6.0]).show(ui, |ui| {
                        for (key, what) in legend() {
                            ui.label(RichText::new(key).color(ACCENT).monospace());
                            ui.add(egui::Label::new(RichText::new(what).color(FG)).wrap());
                            ui.end_row();
                        }
                    });
                    ui.add_space(10.0);
                    ui.label(
                        RichText::new("Typed in the question box:").color(MUTED),
                    );
                    ui.add_space(6.0);
                    egui::Grid::new("commands").spacing([18.0, 6.0]).show(ui, |ui| {
                        for (command, _, what) in COMMANDS {
                            ui.label(RichText::new(command).color(ACCENT).monospace());
                            ui.add(egui::Label::new(RichText::new(what).color(FG)).wrap());
                            ui.end_row();
                        }
                    });
                    ui.add_space(8.0);
                    ui.label(RichText::new("Alt+F4 closes the panel.").color(MUTED));
                }
                RESEARCH => {
                    let text = if self.research.is_empty() {
                        "Research results will appear here. Live coaching continues in Advice."
                    } else {
                        &self.research
                    };
                    ui.label(RichText::new(text).color(FG));
                }
                _ if self.advice.is_empty() => {
                    let mode = if self.session.online {
                        "Advice appears after the other person speaks. Drop reference files anywhere on this window."
                    } else {
                        "Online advice needs a configured provider. References still work locally."
                    };
                    ui.label(RichText::new("Ready to help").color(ACCENT).strong());
                    ui.label(RichText::new(mode).color(MUTED));
                    ui.add_space(10.0);
                    ui.label(
                        RichText::new(format!("Ctrl+Shift+{} lists every key.", fkey(HELP)))
                            .color(MUTED),
                    );
                }
                _ => {
                    // Never blank: the last turn's advice stays up while the
                    // next one is prepared, greyed so the eye knows it is old.
                    let stale = self.thinking;
                    for (tag, body) in display_advice(&self.advice) {
                        let (head, text) = match (tag, stale) {
                            (_, true) => (MUTED, MUTED),
                            (Some(tag), false) => tag.colours(),
                            (None, false) => (FG, FG),
                        };
                        if let Some(tag) = tag {
                            ui.add_space(6.0);
                            ui.label(RichText::new(tag.heading()).color(head).strong().size(13.0));
                        }
                        ui.label(RichText::new(body).color(text).size(19.0));
                    }
                }
            });
    }
    /// Start, continue and finish a self-driven window drag. See `Dragging`.
    ///
    /// Must run *after* the panels are drawn: on the frame of a press
    /// `egui_wants_pointer_input` reduces to "did a widget take this press" (its
    /// hover clause is guarded by `!any_down()`), and that is only known once
    /// the widgets have been laid out. Call it earlier and the question box,
    /// scrollbars and consent buttons all start dragging the window.
    fn move_window(&mut self, ctx: &egui::Context) {
        // Either signal saying "up" ends the drag, because each can miss a
        // release on its own: without mouse capture a release outside the window
        // never reaches egui, and the async key state can be left stale by
        // injected input. A drag that outlives its release glues the panel to
        // the pointer, so the *first* sign of release wins.
        let held = unsafe { GetAsyncKeyState(VK_LBUTTON.0 as i32) } as u16 & 0x8000 != 0
            && ctx.input(|i| i.pointer.any_down());
        if !held {
            self.drag = None;
        }
        if self.drag.is_none()
            && held
            && ctx.input(|i| i.pointer.any_pressed())
            && !ctx.egui_wants_pointer_input()
            && let Some(pos) = ctx.input(|i| i.pointer.press_origin())
        {
            let size = ctx.input(|i| i.viewport_rect()).size();
            if let Some(grab) = hit_test(pos.x, pos.y, size.x, size.y, self.pinned) {
                unsafe {
                    let mut from = POINT::default();
                    let mut rect = RECT::default();
                    if GetCursorPos(&mut from).is_ok()
                        && GetWindowRect(self.hwnd, &mut rect).is_ok()
                    {
                        let rect = (rect.left, rect.top, rect.right, rect.bottom);
                        self.drag = Some(Dragging { grab, from, rect });
                    }
                }
            }
        }
        let Some(drag) = &self.drag else {
            return;
        };
        unsafe {
            let mut now = POINT::default();
            if GetCursorPos(&mut now).is_err() {
                return;
            }
            // `min_inner_size` is in points; the rect is in pixels.
            let scale = ctx.pixels_per_point();
            let min = ((460.0 * scale) as i32, (420.0 * scale) as i32);
            let (left, top, right, bottom) = dragged(
                drag.grab,
                drag.rect,
                now.x - drag.from.x,
                now.y - drag.from.y,
                min,
            );
            let _ = SetWindowPos(
                self.hwnd,
                None,
                left,
                top,
                right - left,
                bottom - top,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
        ctx.request_repaint();
    }
    fn mirror_state(&mut self) {
        let Some((path, last)) = &self.dump else {
            return;
        };
        let now = serde_json::json!({
            "view": self.view,
            "turns": self.transcript.len(),
            "notice": self.notice,
            "advice": self.advice,
            "researching": self.researching,
            "pinned": self.pinned,
            "dragging": self.drag.is_some(),
            "wait": match self.wait() {
                Wait::Idle => "",
                Wait::Thinking(_) => "thinking",
                Wait::FirstWord(_) => "first word",
            },
            "question": self.question,
            "references": self.session.references.preview(),
            "imports": self.imports.len(),
            // The picker's two halves: what it offers and what is selected.
            // A row that is listed but never selectable, or a selection with no
            // row, is the failure this makes visible to a script.
            "sources": self.sources.iter().map(|(n, live)| {
                serde_json::json!({ "name": n, "playing": live })
            }).collect::<Vec<_>>(),
            "hearing": self.session.tune.as_ref().map(|t| t.hearing()).unwrap_or_default(),
        })
        .to_string();
        if *last != now {
            let _ = std::fs::write(path, &now);
            self.dump = Some((path.clone(), now));
        }
    }
    /// The excerpt-sharing gate. Dropping files while coaching is online can
    /// send passages to the provider, which is the user's decision, not ours.
    fn consent(&mut self, ctx: &egui::Context) {
        if self.pending.is_empty() {
            return;
        }
        egui::Window::new("Add reference material")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.set_max_width(420.0);
                ui.label(
                    "These files are read locally and never copied; files in the references \
                     folder, and drops remembered by path, return at every start. Relevant \
                     excerpts may be sent to your coaching or research provider when answering \
                     questions or reacting to the call.",
                );
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if ui.button("Add these references").clicked() {
                        self.session
                            .references
                            .import(std::mem::take(&mut self.pending));
                        self.view = REFERENCES;
                    }
                    if ui.button("Cancel").clicked() {
                        self.pending.clear();
                    }
                });
            });
    }
}

impl eframe::App for State {
    fn clear_color(&self, _: &egui::Visuals) -> [f32; 4] {
        let [r, g, b, _] = BG.to_normalized_gamma_f32();
        [r, g, b, self.alpha]
    }
    fn ui(&mut self, ui: &mut egui::Ui, _: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.keep_unfocusable();
        self.pump();
        while let Ok(id) = self.hotkeys.try_recv() {
            self.command(id, &ctx);
        }
        // The consent gate is the same one as before, drawn rather than a modal
        // message box, which would block the event loop while it is up.
        let dropped: Vec<PathBuf> = self.drops.try_iter().flatten().collect();
        if !dropped.is_empty() {
            if self.session.online {
                self.pending = dropped;
            } else {
                self.session.references.import(dropped);
                self.view = REFERENCES;
            }
        }
        // Escape used to quit outright. That is a hostile key to give a window
        // that floats over a live call, so it only backs out of the question
        // box and hands the keyboard back; Alt+F4 closes.
        if ctx.input(|i| i.key_pressed(Key::Escape)) {
            self.question.clear();
            self.return_keyboard();
        }

        egui::Panel::top("chrome")
            .exact_size(if self.notice.is_empty() { 44.0 } else { 84.0 })
            .show_separator_line(false)
            .show(ui, |ui| {
                ui.add_space(10.0);
                ui.label(
                    RichText::new(self.status_line())
                        .color(MUTED)
                        .size(13.0)
                        .monospace(),
                );
                ui.add_space(4.0);
                if !self.notice.is_empty() {
                    ui.label(RichText::new(&self.notice).color(ACCENT).size(13.0));
                }
            });
        egui::Panel::bottom("ask")
            .exact_size(46.0)
            .show_separator_line(false)
            .show(ui, |ui| {
                ui.add_space(7.0);
                let field = ui.add(
                    TextEdit::singleline(&mut self.question)
                        .hint_text(format!(
                            "Ctrl+Shift+{}, then ask — or type / for the rest…",
                            fkey(ASK)
                        ))
                        .desired_width(f32::INFINITY),
                );
                if std::mem::take(&mut self.focus_question) {
                    field.request_focus();
                }
                if field.lost_focus() && ctx.input(|i| i.key_pressed(Key::Enter)) {
                    self.command(ASK, &ctx);
                }
            });
        // The conversation sits under the advice, just above the question box:
        // newest speech is then next to where you answer it, and the advice —
        // what you are actually reading — holds the top of the panel. It keeps a
        // floor of VISIBLE_TURNS rows and the advice gets everything left over.
        // Added after `ask`, so `ask` stays the outermost bottom panel.
        let row = ui.text_style_height(&TextStyle::Body) + ui.spacing().item_spacing.y;
        let want = (VISIBLE_TURNS * row + 16.0).max(ui.available_height() * 0.30);
        // Grow with the conversation up to that: reserving the full height from
        // the first turn would leave a dead gap between one line of speech and
        // the question box for the opening minutes of every call.
        let needed = self.transcript.len().max(1) as f32 * row + 16.0;
        egui::Panel::bottom("conversation")
            .exact_size(needed.min(want))
            .show(ui, |ui| self.conversation(ui));
        egui::CentralPanel::default().show(ui, |ui| self.main_pane(ui));

        self.consent(&ctx);
        self.mirror_state();

        self.move_window(&ctx);
        // Same cadence as the old 80 ms timer: fast enough for streaming advice,
        // idle enough that an overlay is not busy-drawing over someone's call.
        // A drag needs every frame it can get, and asks for them itself.
        ctx.request_repaint_after(std::time::Duration::from_millis(80));
    }
    fn on_exit(&mut self, _: Option<&eframe::glow::Context>) {
        let _ = self.tx.send(Msg::CancelResearch);
    }
}

/// Take the paths out of a `WM_DROPFILES` handle and release it.
fn dropped_paths(drop: HDROP) -> Vec<PathBuf> {
    unsafe {
        let count = DragQueryFileW(drop, u32::MAX, None);
        let paths = (0..count.min(24))
            .map(|index| {
                let mut path = vec![0u16; DragQueryFileW(drop, index, None) as usize + 1];
                let n = DragQueryFileW(drop, index, Some(&mut path));
                PathBuf::from(String::from_utf16_lossy(&path[..n as usize]))
            })
            .collect();
        DragFinish(drop);
        paths
    }
}

pub fn run(
    rx: Receiver<Msg>,
    speaker: Option<Speaker>,
    alpha: u8,
    commands: Sender<Msg>,
    session: Session,
) -> anyhow::Result<()> {
    let (hotkey_tx, hotkey_rx) = unbounded();
    let (drop_tx, drop_rx) = unbounded();
    let research_enabled = session.research_enabled;

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([620.0, 740.0])
            .with_min_inner_size([460.0, 420.0])
            .with_always_on_top()
            .with_decorations(false)
            .with_transparent(true)
            // Files come through `WM_DROPFILES`, not winit's OLE drop target:
            // `OleInitialize` demands an STA thread and `main` has already put
            // this one in MTA to enumerate WASAPI devices, so asking winit for
            // drag-and-drop aborts the app at window creation with
            // `RPC_E_CHANGED_MODE`. The classic protocol needs no COM at all.
            .with_drag_and_drop(false)
            // Never take the foreground on open; the call app keeps it.
            .with_active(false),
        // winit owns the message loop, so a hotkey only reaches us by looking
        // at the raw messages on their way past.
        event_loop_builder: Some(Box::new(move |builder| {
            builder.with_msg_hook(move |raw| {
                // Win32 `MSG` is { HWND, UINT, WPARAM, LPARAM, ... }; every
                // field up to the two read here is pointer-sized on x64.
                let fields = raw as *const usize;
                let (message, wparam) = unsafe { (*fields.add(1) as u32, *fields.add(2)) };
                match message {
                    // We registered each hotkey with its action id.
                    WM_HOTKEY => {
                        let _ = hotkey_tx.send(wparam);
                        false // winit is welcome to it as well
                    }
                    WM_DROPFILES => {
                        let _ = drop_tx.send(dropped_paths(HDROP(wparam as _)));
                        true // consumed, including `DragFinish`
                    }
                    _ => false,
                }
            });
        })),
        ..Default::default()
    };

    eframe::run_native(
        "Inner Voice — Call companion",
        options,
        Box::new(move |cc| {
            let hwnd = match cc.window_handle().map(|h| h.as_raw()) {
                Ok(RawWindowHandle::Win32(h)) => HWND(isize::from(h.hwnd) as *mut _),
                other => return Err(format!("no Win32 window handle: {other:?}").into()),
            };
            paint_style(&cc.egui_ctx);
            unsafe { DragAcceptFiles(hwnd, true) };
            let mut state = State {
                rx,
                tx: commands,
                hotkeys: hotkey_rx,
                drops: drop_rx,
                session,
                speaker,
                hwnd,
                prior: None,
                alpha: alpha as f32 / 255.0,
                notice: String::new(),
                question: String::new(),
                focus_question: false,
                view: ADVICE,
                paused: false,
                sources: Vec::new(),
                sources_read: None,
                // `route` sends the real value before the first turn; assuming
                // armed here only means the status line is never briefly wrong
                // in the direction that would make a muted coach look broken.
                coaching: true,
                seq: 0,
                thinking: false,
                advice: String::new(),
                research: String::new(),
                researching: false,
                research_id: 0,
                transcript: VecDeque::new(),
                diagnostics: VecDeque::new(),
                imports: VecDeque::new(),
                pending: Vec::new(),
                typing: false,
                pinned: false,
                drag: None,
                launched: Instant::now(),
                asked_at: None,
                first_word_at: None,
                dump: std::env::var_os("IV_UI_DUMP").map(|p| (p.into(), String::new())),
            };
            // A hotkey belongs to whoever registered it first, process-wide, so
            // a clash is silent and would leave an action with no way to reach
            // it. Report the losers rather than shipping a dead key.
            let mut lost = Vec::new();
            for (vk, id, what) in KEYS {
                if !research_enabled && (id == RESEARCH || id == CANCEL) {
                    continue;
                }
                let registered = unsafe {
                    RegisterHotKey(
                        Some(hwnd),
                        id as i32,
                        MOD_CONTROL | MOD_SHIFT | MOD_NOREPEAT,
                        vk,
                    )
                };
                if registered.is_err() {
                    lost.push(format!("Ctrl+Shift+F{} ({what})", vk - VK_F1.0 as u32 + 1));
                }
            }
            if !lost.is_empty() {
                let note = format!("Another app already owns {}", lost.join(", "));
                state.notice = note.clone();
                remember(&mut state.diagnostics, note);
            }
            Ok(Box::new(state))
        }),
    )
    .map_err(|e| anyhow::anyhow!("could not open the panel: {e}"))
}

fn paint_style(ctx: &egui::Context) {
    // The panel is one fixed dark palette in both themes: it floats over
    // someone else's window, so it should not restyle itself with the OS.
    ctx.set_theme(egui::ThemePreference::Dark);
    ctx.all_styles_mut(|style| {
        style.text_styles = [
            (TextStyle::Heading, FontId::proportional(20.0)),
            (TextStyle::Body, FontId::proportional(15.0)),
            (TextStyle::Monospace, FontId::monospace(13.0)),
            (TextStyle::Button, FontId::proportional(15.0)),
            (TextStyle::Small, FontId::proportional(12.0)),
        ]
        .into();
        style.visuals = egui::Visuals::dark();
        style.visuals.panel_fill = BG;
        style.visuals.window_fill = BG;
        style.visuals.override_text_color = Some(FG);
        style.spacing.item_spacing = egui::vec2(8.0, 6.0);
        // Dragging anywhere moves the window, so text must not swallow the
        // gesture by selecting instead. Nothing is lost: the panel is
        // `WS_EX_NOACTIVATE` and never holds keyboard focus, so Ctrl+C could
        // never have reached a selection in the first place.
        style.interaction.selectable_labels = false;
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn the_whole_panel_is_a_grip_until_it_is_pinned() {
        use egui::ResizeDirection as R;
        assert_eq!(
            hit_test(2.0, 2.0, 620.0, 740.0, false),
            Some(Grab::Resize(R::NorthWest))
        );
        assert_eq!(
            hit_test(619.0, 739.0, 620.0, 740.0, false),
            Some(Grab::Resize(R::SouthEast))
        );
        assert_eq!(
            hit_test(2.0, 400.0, 620.0, 740.0, false),
            Some(Grab::Resize(R::West))
        );
        // Not just a title strip: the middle of the panel moves it too.
        assert_eq!(hit_test(100.0, 20.0, 620.0, 740.0, false), Some(Grab::Move));
        assert_eq!(
            hit_test(300.0, 400.0, 620.0, 740.0, false),
            Some(Grab::Move)
        );
        // Pinned freezes both gestures, edges included.
        assert_eq!(hit_test(300.0, 400.0, 620.0, 740.0, true), None);
        assert_eq!(hit_test(2.0, 2.0, 620.0, 740.0, true), None);
    }
    #[test]
    fn a_drag_moves_the_window_and_a_resize_only_pushes_its_own_edges() {
        use egui::ResizeDirection as R;
        let rect = (100, 100, 720, 840); // 620 x 740
        let min = (460, 420);
        // Moving keeps the size and shifts both corners.
        assert_eq!(dragged(Grab::Move, rect, 40, -30, min), (140, 70, 760, 810));
        // The south-east corner moves only the right and bottom edges.
        assert_eq!(
            dragged(Grab::Resize(R::SouthEast), rect, 50, 60, min),
            (100, 100, 770, 900)
        );
        // The north-west corner moves only the left and top edges.
        assert_eq!(
            dragged(Grab::Resize(R::NorthWest), rect, 50, 60, min),
            (150, 160, 720, 840)
        );
        // One edge at a time for the sides.
        assert_eq!(
            dragged(Grab::Resize(R::West), rect, 50, 60, min),
            (150, 100, 720, 840)
        );
        // An edge stops at the minimum instead of crossing its opposite.
        assert_eq!(
            dragged(Grab::Resize(R::West), rect, 5_000, 0, min),
            (260, 100, 720, 840)
        );
        assert_eq!(
            dragged(Grab::Resize(R::SouthEast), rect, -5_000, -5_000, min),
            (100, 100, 560, 520)
        );
    }
    #[test]
    fn advice_labels_are_readable_without_losing_content() {
        let shown = display_advice(
            "ASK What is the rollback plan?\nASKING is not a tag\nNOTE Owner unclear",
        );
        assert_eq!(
            shown[0],
            (Some(Tag::Ask), "What is the rollback plan?".into())
        );
        // Only the exact tag followed by whitespace becomes a heading.
        assert_eq!(shown[1], (None, "ASKING is not a tag".into()));
        assert_eq!(shown[2], (Some(Tag::Note), "Owner unclear".into()));
        assert_eq!(Tag::Ask.heading(), "ASK NEXT");
        assert_eq!(Tag::Fix.heading(), "CLARIFY");
    }
    #[test]
    fn only_ask_and_fix_are_saturated() {
        // Von Restorff: colour every line and nothing stands out.
        assert_eq!(Tag::Ask.colours().0, ACCENT);
        assert_eq!(Tag::Fix.colours().0, AMBER);
        assert_eq!(Tag::Say.colours().0, SOFT_GREEN);
        assert_eq!(Tag::Note.colours(), (MUTED, MUTED));
        assert_ne!(Tag::Say.colours().0, ACCENT);
    }
    /// The README is the only interface a new user has before the panel is
    /// running, and it went stale silently: after twelve keys became six it
    /// still told people to press Ctrl+Shift+F4, F7, F8, F10 and F12, and
    /// named `/coach on` and `/coach off`, none of which exist. Following the
    /// documentation did nothing at all, which is worse than a missing page.
    ///
    /// Two halves, because the drift had both shapes: a key the README names
    /// that is no longer registered, and a command the README never learned.
    #[test]
    fn the_readme_documents_the_keys_and_commands_that_exist() {
        let readme = include_str!("../README.md");

        // `.env.example` too: the same stale names outlived the first sweep
        // there and in `--setup`'s own output, because a key written as a
        // literal in prose is unreachable from `fkey` and moves with nothing.
        // Anything user-facing should name the *command* -- `/research` does
        // not renumber when a key does.
        for (where_, text) in [("README", readme), (".env.example", include_str!("../.env.example"))]
        {
            for line in text.lines() {
                // The one line that may name them says they are free.
                if line.contains("free for every other app") {
                    continue;
                }
                for n in KEYS.len() + 1..=12 {
                    assert!(
                        !line.contains(&format!("Ctrl+Shift+F{n}")),
                        "{where_} presses Ctrl+Shift+F{n}, which nothing registers: {line}"
                    );
                    assert!(
                        !line.contains(&format!("F{n}")),
                        "{where_} names F{n}, which is not a key any more -- name the command: {line}"
                    );
                }
            }
        }

        for (name, _, _) in COMMANDS {
            // `/who <name>` is documented with its placeholder; match the verb.
            let verb = name.split_whitespace().next().unwrap();
            assert!(
                readme.contains(verb),
                "README never mentions the {verb} command"
            );
        }
    }

    #[test]
    fn every_action_is_reachable_by_exactly_one_key() {
        let mut ids: Vec<usize> = KEYS.iter().map(|(_, id, _)| *id).collect();
        ids.sort_unstable();
        let mut unique = ids.clone();
        unique.dedup();
        assert_eq!(ids, unique, "two keys share an action");
        assert_eq!(KEYS.len(), legend().len());
        // Quit has no key on purpose; Alt+F4 closes.
        assert!(!ids.contains(&CLOSE));

        // The cut from twelve keys to six moved seven actions to commands. An
        // action with neither is dead code no user can run, so the whole list
        // is checked at once rather than one grandfathered case at a time —
        // which is what the per-id version had turned into.
        let typed: Vec<usize> = COMMANDS.iter().map(|(_, id, _)| *id).collect();
        for (id, name) in [
            (ADVICE, "advice"),
            (ASK, "ask"),
            (COACH, "coach"),
            (PAUSE, "pause"),
            (MINIMIZE, "minimize"),
            (HELP, "help"),
            (TRANSCRIPT, "transcript"),
            (REFERENCES, "references"),
            (RESEARCH, "research"),
            (CANCEL, "cancel"),
            (PIN, "pin"),
            (CLEAR, "clear"),
            (DIAGNOSTICS, "diagnostics"),
            (SOURCES, "sources"),
        ] {
            assert!(
                ids.contains(&id) || typed.contains(&id),
                "{name} has neither a key nor a command"
            );
            assert!(
                !(ids.contains(&id) && typed.contains(&id)),
                "{name} has both a key and a command; one of them is the lie"
            );
        }
        // What earned a key is what must never cost the call app its keyboard.
        // A command has to be typed, and typing borrows focus — so pausing and
        // hiding, the two you reach for when something private happens, cannot
        // become commands however rarely they are pressed.
        for id in [PAUSE, MINIMIZE, COACH, ASK, ADVICE, HELP] {
            assert!(ids.contains(&id), "this action has to keep its key");
        }
        // A `FORWARD` row is handled by `route` because it writes state the
        // panel cannot reach — `/hear` the capture selection, `/who` the voice
        // book. One naming anything else would be typed into silence.
        for (name, id, _) in COMMANDS {
            assert!(name.starts_with('/'), "{name} is not a command");
            assert!(
                id != FORWARD || ["/hear", "/who"].iter().any(|p| name.starts_with(p)),
                "{name} forwards to a router that does not handle it"
            );
        }
    }
    #[test]
    fn status_line_counts_the_wait_and_only_hints_early() {
        assert_eq!(
            status_text(
                Mode::Listening { online: true },
                Wait::Thinking(1.42),
                None,
                false,
                false,
                false,
                true
            ),
            "Listening · coaching on · thinking 1.4s"
        );
        assert_eq!(
            status_text(
                Mode::Listening { online: true },
                Wait::FirstWord(1.2),
                None,
                false,
                true,
                false,
                true
            ),
            "Listening · coaching on · first word 1.2s · pinned"
        );
        // The hint is the last thing on the line and only while it is shown.
        assert_eq!(
            status_text(Mode::Preview, Wait::Idle, None, false, false, true, true),
            "Preview — microphone off · no online requests   ·   Ctrl+Shift+F6 for keys"
        );
        // A named model replaces the generic "coaching on" (sub-project 3 passes it).
        assert_eq!(
            status_text(
                Mode::Listening { online: true },
                Wait::Idle,
                Some("gemini-3.5-flash-lite"),
                true,
                false,
                false,
                true
            ),
            "Listening · gemini-3.5-flash-lite · researching"
        );
        assert_eq!(
            status_text(
                Mode::Listening { online: false },
                Wait::Idle,
                None,
                false,
                false,
                false,
                true
            ),
            "Listening · transcription only"
        );
        assert_eq!(
            status_text(Mode::Paused, Wait::Thinking(9.0), None, false, false, false, true),
            "Paused — audio is not being transcribed · thinking 9.0s"
        );
    }

    /// Left running through a working day, "no advice on screen" has two causes
    /// and only one of them is a fault. The line has to separate them.
    #[test]
    fn a_muted_coach_says_so_rather_than_looking_idle() {
        let line = |coaching| {
            status_text(
                Mode::Listening { online: true },
                Wait::Idle,
                Some("gemini-3.5-flash-lite"),
                false,
                false,
                false,
                coaching,
            )
        };
        assert_eq!(line(true), "Listening · gemini-3.5-flash-lite");
        assert_eq!(line(false), "Listening · advice on request (F2 asks)");
        // Still listening, and still says so: this is not Pause, which stops
        // transcription and so writes nothing down at all.
        assert!(line(false).starts_with("Listening"));
    }
}
