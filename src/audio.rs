//! Capture, voice activity detection, transcription.
//!
//! Mic and system loopback are two separate WASAPI streams, and that *is* the
//! speaker separation: exact labels, no diarization model, nothing to tune.
//! WASAPI's `autoconvert` hands us 16 kHz mono f32 directly, so there is no
//! resampling or downmix code here at all.

use crate::{Msg, Who};
use anyhow::Result;
use crossbeam_channel::{Sender, unbounded};
use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};
use wasapi::{
    AudioCaptureClient, AudioClient, Device, DeviceEnumerator, Direction, SampleType, SessionState,
    StreamMode, WaveFormat, initialize_mta,
};
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperState};

pub const RATE: usize = 16_000;
pub const FRAME_MS: usize = 20;
pub const FRAME: usize = RATE * FRAME_MS / 1000;

/// One app with an audio session on the loopback endpoint.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppSession {
    pub pid: u32,
    /// At least one of its streams is running right now.
    pub active: bool,
    /// The session's display name, else the executable's stem. Most apps set
    /// no display name, so it is usually the stem — `Discord`, `chrome`.
    pub name: String,
}

/// Every app with a session on `dev`, system sounds excluded.
///
/// A session that cannot be read is skipped, not fatal: sessions come and go
/// while the list is being walked, and the reacquire loop calls this every two
/// seconds — one unrelated app closing mid-walk must not cost the caller its
/// whole listing.
pub fn sessions(dev: &Device) -> Result<Vec<AppSession>> {
    let list = dev
        .get_iaudiosessionmanager()?
        .get_audiosessionenumerator()?;
    let mut out = Vec::new();
    for i in 0..list.get_count()? {
        let session = list.get_session(i)?;
        let Ok(pid) = session.get_process_id() else {
            continue;
        };
        if pid == 0 {
            continue; // the system-sounds session has no process
        }
        // Our own render session, which exists because the panel captures the
        // endpoint (and speaks, with --speak). Excluded for the same reason
        // system sounds are: it is not an app anyone is listening to, it is
        // never a valid `--hear` target, and the coach's source line would
        // otherwise report the panel back to itself as "currently playing:
        // inner-voice".
        if pid == std::process::id() {
            continue;
        }
        let mut name = session.get_display_name().unwrap_or_default();
        if name.is_empty() {
            name = image_stem(pid).unwrap_or_default();
        }
        if name.is_empty() {
            continue;
        }
        let Ok(state) = session.get_state() else {
            continue;
        };
        out.push(AppSession {
            pid,
            active: matches!(state, SessionState::Active),
            name,
        });
    }
    Ok(out)
}

/// Which session `--hear <name>` means: case-insensitive substring on the
/// name, an active session over a silent one, else the first listed.
pub fn resolve(sessions: &[AppSession], wanted: &str) -> Option<u32> {
    let wanted = wanted.to_lowercase();
    let matching: Vec<&AppSession> = sessions
        .iter()
        .filter(|s| s.name.to_lowercase().contains(&wanted))
        .collect();
    matching
        .iter()
        .find(|s| s.active)
        .or(matching.first())
        .map(|s| s.pid)
}

/// Whether `pid` is still running. A process-loopback stream whose target
/// exits delivers silence, not an error, so this is how a closed app is
/// noticed and re-acquired under its new pid.
pub fn alive(pid: u32) -> bool {
    use windows::Win32::Foundation::{CloseHandle, STILL_ACTIVE};
    use windows::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    unsafe {
        let Ok(handle) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
            return false;
        };
        let mut code = 0u32;
        let running =
            GetExitCodeProcess(handle, &mut code).is_ok() && code == STILL_ACTIVE.0 as u32;
        let _ = CloseHandle(handle);
        running
    }
}

fn image_stem(pid: u32) -> Option<String> {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{
        OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
        QueryFullProcessImageNameW,
    };
    use windows::core::PWSTR;
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buffer = vec![0u16; 1024];
        let mut len = buffer.len() as u32;
        let ok = QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_WIN32,
            PWSTR(buffer.as_mut_ptr()),
            &mut len,
        )
        .is_ok();
        let _ = CloseHandle(handle);
        if !ok {
            return None;
        }
        let path = String::from_utf16_lossy(&buffer[..len as usize]);
        std::path::Path::new(&path)
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
    }
}

const PRE_MS: usize = 240; // pre-roll, so we don't clip the first phoneme
const HANG_MS: usize = 500; // silence that ends a turn
const MIN_VOICED_MS: usize = 300; // shorter than this is a cough, not a turn
const MAX_MS: usize = 15_000; // flush a monologue instead of waiting it out
const GATE_FLOOR: f32 = 0.004;

/// Beam search, not greedy.
///
/// Greedy commits to the highest-probability token at every step and cannot
/// take it back, which is what produces confident non-words — "slect" for
/// "select" — because nothing downstream ever gets to say the sentence became
/// impossible. A beam keeps 5 hypotheses and lets later audio settle earlier
/// tokens. Measured on this GPU it costs ~22 ms a turn, which the flash
/// attention win already paid for several times over.
const BEAM: i32 = 5;

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|s| s * s).sum::<f32>() / x.len() as f32).sqrt()
}

/// Energy VAD -> whole utterances.
///
/// Emits once speech is followed by `HANG_MS` of quiet, or once `MAX_MS` is
/// buffered so a long monologue still produces transcripts while it runs.
pub struct Segmenter {
    gate: f32,
    pre: VecDeque<f32>,
    buf: Vec<f32>,
    silent: usize,
    voiced: usize,
    speaking: bool,
}

impl Segmenter {
    pub fn new(gate: f32) -> Self {
        Self {
            gate,
            pre: VecDeque::new(),
            buf: Vec::new(),
            silent: 0,
            voiced: 0,
            speaking: false,
        }
    }

    /// Feed one `FRAME`-sized frame. Returns a finished utterance.
    pub fn push(&mut self, frame: &[f32]) -> Option<Vec<f32>> {
        let loud = rms(frame) > self.gate;

        if !self.speaking {
            self.pre.extend(frame.iter().copied());
            while self.pre.len() > PRE_MS * RATE / 1000 {
                self.pre.pop_front();
            }
            if loud {
                self.speaking = true;
                self.voiced = 1;
                self.silent = 0;
                self.buf = self.pre.drain(..).collect();
            }
            return None;
        }

        self.buf.extend_from_slice(frame);
        if loud {
            self.voiced += 1;
            self.silent = 0;
        } else {
            self.silent += 1;
        }

        if self.silent < HANG_MS / FRAME_MS && self.buf.len() < MAX_MS * RATE / 1000 {
            return None;
        }
        let out = (self.voiced >= MIN_VOICED_MS / FRAME_MS).then(|| std::mem::take(&mut self.buf));
        self.reset();
        out
    }

    fn reset(&mut self) {
        self.buf.clear();
        self.pre.clear();
        self.silent = 0;
        self.voiced = 0;
        self.speaking = false;
    }
}

/// Read-only knobs both capture threads share.
pub struct Tune {
    pub epoch: Arc<AtomicU64>,
    /// Glossary primed into whisper's decoder — see `knowledge::glossary`.
    /// Behind a lock because the corpus can be re-read mid-call.
    pub prompt: std::sync::RwLock<String>,
    /// Where to save every utterance for later listening, if asked.
    pub dump: Option<std::path::PathBuf>,
    /// Speaker-embedding model. `None` — including a missing file — simply means
    /// far-end turns stay unattributed, which is a working call, not an error.
    pub voices: Option<std::path::PathBuf>,
    /// Voices named on earlier calls, shared with `route`, which owns the
    /// names while this side owns the embeddings. See `people`.
    pub book: std::sync::Arc<std::sync::Mutex<crate::people::Book>>,
    /// Which apps are `THEM`. Empty is the whole speaker mix, as before.
    ///
    /// A *selection*, not a launch-time decision: `/hear` in the question box
    /// rewrites it mid-call. Names are substrings resolved when a stream opens,
    /// so an app that is not running yet can be registered and waited for —
    /// which is the point, since `sessions()` only ever lists what is playing
    /// right now.
    pub hear: std::sync::RwLock<Vec<String>>,
    /// Bumped whenever `hear` changes. Every stream below polls it and bails
    /// within 2 s, which is what makes the switch take effect without a
    /// restart; a generation rather than a flag so a stream started under the
    /// old selection can never mistake itself for the new one.
    pub hear_gen: AtomicU64,
}

impl Tune {
    /// What is heard as `THEM` right now. Empty is the whole speaker mix.
    pub fn hearing(&self) -> Vec<String> {
        self.hear.read().unwrap_or_else(|e| e.into_inner()).clone()
    }
    /// Change it. The generation is bumped *after* the write, never before: a
    /// stream that sees the new generation must find the new selection already
    /// there, or it re-serves the old one and the switch silently does nothing.
    ///
    /// One path for both surfaces — the Sources pane and `/hear` — so the
    /// ordering rule above is written down once.
    pub fn hear_only(&self, apps: Vec<String>) {
        *self.hear.write().unwrap_or_else(|e| e.into_inner()) = apps;
        self.hear_gen.fetch_add(1, Ordering::SeqCst);
    }
    /// Add or remove one app, which is what clicking a row in the picker does.
    /// Returns the selection afterwards.
    pub fn toggle(&self, app: &str) -> Vec<String> {
        let mut apps = self.hearing();
        let lower = app.to_lowercase();
        // Matched the way a stream resolves it, not by equality: `/hear chr`
        // selects "chrome", and clicking that row must turn the same selection
        // *off* rather than adding a second, redundant entry for it.
        match apps.iter().position(|a| lower.contains(&a.to_lowercase())) {
            Some(i) => {
                apps.remove(i);
            }
            None => apps.push(app.to_string()),
        }
        self.hear_only(apps.clone());
        apps
    }
}

/// Names of the apps playing on a named render device, for the Sources pane.
///
/// Takes the device *name* and opens it here because COM interfaces are not
/// `Send`: the panel runs on the main thread (already MTA, `main` put it there
/// to enumerate devices) and so must resolve its own.
/// Returns the whole `AppSession`, not just the name: the pane wants every
/// app that *could* be selected, and the coach wants only the ones making noise
/// right now. One enumeration, two questions — measured at 1.1 ms, so the
/// router thread can afford to ask it per request.
pub fn playing(device: &str) -> Result<Vec<AppSession>> {
    let dev = DeviceEnumerator::new()?
        .get_device_collection(&Direction::Render)?
        .get_device_with_name(device)?;
    let mut apps = sessions(&dev)?;
    // Active first: the one making noise now is the one being looked for.
    apps.sort_by_key(|a| !a.active);
    Ok(apps)
}

/// `--hear`/`/hear` take a comma list, because WASAPI's activation params name
/// exactly one process tree: hearing two apps is two streams, and the list is
/// where that plurality is written down once.
pub fn parse_hear(spec: &str) -> Vec<String> {
    spec.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

/// Save one utterance next to what whisper made of it.
///
/// Accuracy complaints are unfalsifiable without the audio that caused them:
/// this turns "it misheard me" into a clip and a transcript sitting side by
/// side. 16-bit PCM so the files stay small and `hound` can read them back.
fn save(dir: &std::path::Path, label: &str, audio: &[f32], text: &str) -> Result<()> {
    std::fs::create_dir_all(dir)?;
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_millis();
    // Timestamped rather than counted: two threads write here and a clock needs
    // no lock between them.
    let stem = dir.join(format!("{ms}-{label}"));
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: RATE as u32,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut w = hound::WavWriter::create(stem.with_extension("wav"), spec)?;
    for &s in audio {
        w.write_sample((s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16)?;
    }
    w.finalize()?;
    std::fs::write(stem.with_extension("txt"), text)?;
    Ok(())
}

fn params(prompt: &str) -> FullParams<'static, 'static> {
    let mut p = FullParams::new(SamplingStrategy::BeamSearch {
        beam_size: BEAM,
        patience: -1.0, // whisper.cpp ignores it; -1.0 is its own "unset"
    });
    p.set_language(Some("en"));
    p.set_no_context(true); // short chunks: prior text would only seed hallucinations
    p.set_suppress_blank(true);
    // No `set_audio_ctx`: a reduced encoder window was worth ~22 ms and could
    // not be shown to be accuracy-neutral on real speech — only on TTS and on
    // `jfk.wav`, which whisper has memorised. Accuracy is the scarcer resource
    // here, so the encoder runs its full 1500 states. See LEDGER.
    if !prompt.is_empty() {
        // ponytail: whisper-rs leaks this CString per call (~800 B). Bounded by
        // turns in a call; revisit if a session ever runs for days.
        p.set_initial_prompt(prompt);
    }
    p.set_print_special(false);
    p.set_print_progress(false);
    p.set_print_realtime(false);
    p.set_print_timestamps(false);
    p
}

/// Run one inference nobody is waiting for.
///
/// The first call on a fresh CUDA context pays kernel setup — over 100 ms, and
/// it lands on whatever the other side says first, which is the worst possible
/// moment. Doing it here spends that during the VAD's calibration second, while
/// the call is still being joined.
pub fn warm(state: &mut WhisperState) {
    let _ = state.full(params(""), &vec![0f32; RATE]); // output is not the point
}

/// Above this, whisper itself says the segment is not speech, and we drop it.
///
/// Whisper invents fluent boilerplate out of near-silence -- "Thanks for
/// watching", "Please subscribe", subtitle credits -- and every one of those is
/// three or more words, so it clears `--min-words`, becomes a real turn, enters
/// the 24-turn history, is written to the JSONL log, and buys a paid provider
/// request. Nothing downstream could tell it from speech, because as *text* it
/// is not distinguishable from speech; the only place the difference still
/// exists is the decoder, which scores it and was never asked.
///
/// Dropping it here rather than at `route`'s gate is what makes it free: an
/// utterance whose segments all fail this leaves `transcribe` empty, and the
/// existing `!text.is_empty()` check below already declines to send a turn at
/// all. No new message, no new field, no new plumbing.
///
/// ponytail: UNVALIDATED on this machine's audio, and deliberately timid --
/// OpenAI's reference uses 0.6, but only in conjunction with an average
/// logprob test this does not do, so 0.6 alone would be the more aggressive
/// setting rather than the same one. A dropped real turn is worse than a
/// wasted request, so this errs at "near-certain silence" and nothing else.
/// Tune it from `--dump` audio: a dropped utterance still writes its clip with
/// empty text, so the clips folder already shows exactly what this rejected.
const NO_SPEECH: f32 = 0.9;

/// Returns the text rather than sending it: the caller also needs the speaker
/// embedding, which is computed concurrently, and the turn cannot be stamped
/// until both have landed.
fn transcribe(state: &mut WhisperState, audio: &[f32], tune: &Tune) -> Result<String> {
    let prompt = tune
        .prompt
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    state.full(params(&prompt), audio)?;
    Ok(speech(
        state
            .as_iter()
            .map(|s| (s.no_speech_probability(), s.to_string())),
    ))
}

/// Join the segments whisper is confident were speech, and drop the rest.
///
/// A free function over `(no_speech_probability, text)` rather than a `filter`
/// inside `transcribe`, for one reason: `transcribe` needs a `WhisperState` and
/// a 574 MB model, so a filter written inline there can only be tested by
/// something that owns both — and `tests/gpu_transcribes.rs`, the one test that
/// does, calls whisper directly and never goes through this function at all.
/// Inverting the threshold to `-1.0` left that test's transcript byte for byte
/// identical, which is what proved it was no evidence. This seam is the whole
/// difference between a guard and a guard nobody can check.
fn speech(segments: impl Iterator<Item = (f32, String)>) -> String {
    segments
        .filter(|(no_speech, _)| *no_speech <= NO_SPEECH)
        .map(|(_, text)| text)
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string()
}

/// Hand a finished utterance to this stream's transcriber.
///
/// The queue is unbounded on purpose: whisper decodes an order of magnitude
/// faster than speech arrives, so it cannot grow during a conversation, and
/// bounding it could only be enforced by dropping audio — which is the exact
/// failure moving transcription off the capture thread was meant to remove.
fn send(tx: &Sender<(u64, Vec<f32>)>, epoch: u64, utt: Vec<f32>) -> Result<()> {
    tx.send((epoch, utt))
        .map_err(|_| anyhow::anyhow!("transcriber stopped"))
}

/// One capture stream, start to finish. `dir` being `Render` is what makes this
/// loopback (see `initialize_client` in the wasapi crate).
///
/// The device is opened here rather than passed in: COM interfaces are not
/// `Send`, so each thread enumerates its own after its own `initialize_mta`.
pub struct Input {
    pub who: Who,
    pub dir: Direction,
    pub name: String,
    pub gate_override: Option<f32>,
    pub mute: Option<Arc<AtomicBool>>,
}

/// An open capture stream.
struct Stream {
    client: AudioClient,
    capture: AudioCaptureClient,
}

impl Stream {
    /// Stop before drop, so the next client — the reacquired app — opens
    /// against a released endpoint.
    fn close(self) {
        let _ = self.client.stop_stream();
    }
}

/// Loopback on the whole endpoint: everything the speakers play.
fn open_endpoint(dev: &Device) -> Result<Stream> {
    let mut client = dev.get_iaudioclient()?;
    let fmt = WaveFormat::new(32, 32, &SampleType::Float, RATE, 1, None);
    let (_default_period, min_period) = client.get_device_period()?;
    client.initialize_client(
        &fmt,
        &Direction::Capture,
        &StreamMode::PollingShared {
            autoconvert: true,
            // >=500 ms of slack against scheduler jitter. Transcription no
            // longer runs on this thread (see the worker below), so this is now
            // headroom rather than the only thing preventing dropped audio —
            // which it was, badly, at the previous 100 ms. 32 KB either way.
            buffer_duration_hns: min_period.max(5_000_000),
        },
    )?;
    let capture = client.get_audiocaptureclient()?;
    client.start_stream()?;
    Ok(Stream { client, capture })
}

/// Loopback on one process tree: only that app, whatever else is playing.
///
/// `get_device_period` is unsupported on this client (crate docs), so the
/// slack is fixed rather than derived. The same 16 kHz mono f32 is requested
/// as for the endpoint, so nothing downstream learns which client fed it.
fn open_process(pid: u32) -> Result<Stream> {
    let fmt = WaveFormat::new(32, 32, &SampleType::Float, RATE, 1, None);
    let mut client = AudioClient::new_application_loopback_client(pid, true)?;
    client.initialize_client(
        &fmt,
        &Direction::Capture,
        &StreamMode::PollingShared {
            autoconvert: true,
            buffer_duration_hns: 5_000_000,
        },
    )?;
    let capture = client.get_audiocaptureclient()?;
    client.start_stream()?;
    Ok(Stream { client, capture })
}

/// Everything the capture loop needs that is not the stream itself.
#[derive(Clone, Copy)]
struct Feed<'a> {
    label: &'a str,
    tx: &'a Sender<Msg>,
    tune: &'a Arc<Tune>,
    utt_tx: &'a Sender<(u64, Vec<f32>)>,
    gate_override: Option<f32>,
    mute: Option<&'a Arc<AtomicBool>>,
}

pub fn run(input: Input, ctx: Arc<WhisperContext>, tx: Sender<Msg>, tune: Arc<Tune>) -> Result<()> {
    let Input {
        who,
        dir,
        name,
        gate_override,
        mute,
    } = input;
    initialize_mta().ok()?;
    let dev = DeviceEnumerator::new()?
        .get_device_collection(&dir)?
        .get_device_with_name(&name)?;

    // Transcription runs on its own thread, and this one only ever reads,
    // gates and segments — all microseconds.
    //
    // It used to call whisper inline. That meant ~100 ms for a short turn and
    // ~300 ms for a monologue flush during which this loop read nothing from
    // WASAPI, so the ring buffer was the only thing standing between a slow turn
    // and lost audio, and the VAD's own clock stopped along with it: the silence
    // that ends a turn was not being counted while the GPU worked. Capture is a
    // realtime deadline and inference is not, so they do not belong on one
    // thread.
    // Only the far end has a selectable source. `run` is shared by both capture
    // threads, and a mic that followed `tune.hear` would abandon the microphone
    // for an app's loopback the moment `--hear` was set — which is exactly what
    // it did once: `hear_isolation.ps1` failed with the target app's sentence
    // logged as YOU, because both threads had opened the same process stream.
    let selectable = who.is_them();
    // `who` moves to the worker, which is what stamps each turn; the capture
    // loop keeps only the display label for its own status messages.
    let label = who.label();
    let (utt_tx, utt_rx) = unbounded::<(u64, Vec<f32>)>();
    {
        let (ctx, tx, tune) = (ctx.clone(), tx.clone(), tune.clone());
        let label = label.clone();
        std::thread::spawn(move || {
            // Built here because `WhisperState` is not `Send`. One state per
            // stream off the one shared context is what lets YOU and THEM decode
            // at the same time without a lock.
            let mut state = match ctx.create_state() {
                Ok(s) => s,
                Err(e) => {
                    let _ = tx.send(Msg::Sys(format!("{label} whisper: {e}")));
                    return;
                }
            };
            warm(&mut state);

            // Only the far end needs identifying — the microphone is the user by
            // construction. A model that will not load costs names, not the call.
            let mut voices = match (who.is_them(), &tune.voices) {
                (true, Some(path)) => match crate::voiceid::VoiceId::new(path, tune.book.clone()) {
                    Ok(v) => Some(v),
                    Err(e) => {
                        let _ = tx.send(Msg::Sys(format!("speaker id off: {e}")));
                        None
                    }
                },
                _ => None,
            };

            while let Ok((epoch, utt)) = utt_rx.recv() {
                if epoch % 2 != 0 || tune.epoch.load(Ordering::SeqCst) != epoch {
                    continue;
                }
                // Whisper is on the GPU, the embedding on the CPU, and neither
                // needs the other's answer — so they run at once and identifying
                // a speaker costs max(~100 ms, ~54 ms) rather than their sum.
                let (voice, text) = std::thread::scope(|s| {
                    let embed = s.spawn(|| voices.as_mut().and_then(|v| v.identify(&utt)));
                    let text = transcribe(&mut state, &utt, &tune);
                    // A panic in the embedding thread means no name, not no turn.
                    (embed.join().unwrap_or(None), text)
                });

                // One bad utterance must not end transcription for the call.
                let text = match text {
                    Ok(t) => t,
                    Err(e) => {
                        let _ = tx.send(Msg::Sys(format!("{label} transcribe: {e}")));
                        continue;
                    }
                };
                if tune.epoch.load(Ordering::SeqCst) != epoch {
                    continue;
                }
                let turn = match who {
                    Who::You => Who::You,
                    // Naming happens in `route`, which is the only place that sees
                    // both sides — being addressed by name usually happens on the
                    // other stream.
                    Who::Them { .. } => Who::Them { voice, name: None },
                };
                if let Some(dir) = &tune.dump {
                    // A failed dump must never take the call down with it.
                    if let Err(e) = save(dir, &turn.label(), &utt, &text) {
                        let _ = tx.send(Msg::Sys(format!("dump: {e}")));
                    }
                }
                if !text.is_empty() && tx.send(Msg::Turn(turn, text)).is_err() {
                    return;
                }
            }
        });
    }

    let feed = Feed {
        label: &label,
        tx: &tx,
        tune: &tune,
        utt_tx: &utt_tx,
        gate_override,
        mute: mute.as_ref(),
    };
    // Calibration measures a *room* through a mic — noise floor, fan, street.
    // A process-loopback stream has none of that: it is the app's own digital
    // output, silence in it is exactly 0.0, and it delivers nothing at all
    // until the app plays. So the calibration second would land on the first
    // sentence and set the gate from speech — measured at 0.5942 once, which
    // gated every later turn out. The floor is the right gate here, and
    // `--sys-gate` still overrides it. It also opts out of the mute: the app's
    // stream is another process, so the spoken advice is never in it and
    // deafening it would only lose the far end. The endpoint pumps keep it.
    let app_feed = Feed {
        gate_override: Some(gate_override.unwrap_or(GATE_FLOOR)),
        mute: None,
        ..feed
    };

    if !selectable {
        // The mic: one endpoint stream for the life of the thread, exactly as
        // before. It must not enter the supervisor even to be handed an empty
        // selection, or a `/hear` would bump the generation and restart the
        // microphone for nothing.
        let stream = open_endpoint(&dev)?;
        return pump(&stream, &feed, &mut || true);
    }

    // The supervisor. Each pass reads the selection, serves it until the
    // generation moves, then reads it again — so `/hear` mid-call is a switch
    // and not a restart. `pump` polls `still_there` every 2 s, which is also
    // this loop's reaction time; a source change is not worth a faster poll on
    // a thread with a realtime deadline.
    loop {
        let generation = tune.hear_gen.load(Ordering::SeqCst);
        let wanted = tune.hear.read().unwrap_or_else(|e| e.into_inner()).clone();
        let current = || tune.hear_gen.load(Ordering::SeqCst) == generation;

        if wanted.is_empty() {
            // The endpoint mix, exactly as before — except that it now yields
            // when the selection changes instead of owning the thread forever.
            let stream = open_endpoint(&dev)?;
            let outcome = pump(&stream, &feed, &mut || current());
            stream.close();
            match outcome {
                // A real stream error with the selection unchanged is what it
                // always was: fatal, and `main` reports it.
                Err(e) if current() => return Err(e),
                _ => continue,
            }
        }

        // One process-loopback client per app: WASAPI's activation params take
        // a single process tree, so hearing two apps is two streams. They share
        // the one whisper worker through `utt_tx`.
        // ponytail: that serialises inference — two apps talking at the same
        // instant queue rather than decode in parallel. One worker per app is
        // the upgrade, and costs a `WhisperState` (~50 MB) each.
        let solo = wanted.len() == 1;
        // Borrowed once and captured by `move`, so each thread takes a copy of
        // the *references* — moving the `Feed`s themselves would hand the first
        // app everything the second one needs.
        let (feed, app_feed, device, dir) = (&feed, &app_feed, &name, &dir);
        std::thread::scope(|scope| {
            for app in &wanted {
                scope.spawn(move || {
                    if let Err(e) = hear_app(app, device, dir, feed, app_feed, solo, generation) {
                        let _ = feed.tx.send(Msg::Sys(format!("hearing: {app} stopped: {e}")));
                    }
                });
            }
        });
        // Every app thread has ended, which means the selection moved (or each
        // gave up). Re-read it rather than spin: if nothing changed, the next
        // pass rebuilds the same set and the reacquire waits are back.
        if current() {
            std::thread::sleep(Duration::from_secs(2));
        }
    }
}

/// One app's process tree, reacquired for as long as it stays in the selection.
///
/// Runs on its own thread with its own `initialize_mta` and its own `Device`,
/// because COM interfaces are not `Send` — the same rule `main` follows when it
/// resolves devices to *names* and lets each capture thread reopen by name.
#[allow(clippy::too_many_arguments)]
fn hear_app(
    app: &str,
    device: &str,
    dir: &Direction,
    feed: &Feed,
    app_feed: &Feed,
    solo: bool,
    generation: u64,
) -> Result<()> {
    initialize_mta().ok()?;
    let dev = DeviceEnumerator::new()?
        .get_device_collection(dir)?
        .get_device_with_name(device)?;
    let (tx, tune) = (feed.tx, feed.tune);
    let current = || tune.hear_gen.load(Ordering::SeqCst) == generation;

    // The app may not be running yet — the call app usually starts second —
    // and may restart with a new pid mid-call. Neither is an error here.
    let mut waiting_said = false;
    while current() {
        let pid = match sessions(&dev).map(|list| resolve(&list, app)) {
            Ok(Some(pid)) => pid,
            Ok(None) => {
                if !waiting_said {
                    tx.send(Msg::Sys(format!("hearing: waiting for {app}…")))?;
                    waiting_said = true;
                }
                std::thread::sleep(Duration::from_secs(2));
                continue;
            }
            Err(e) => {
                tx.send(Msg::Sys(format!("hearing: {e}")))?;
                std::thread::sleep(Duration::from_secs(2));
                continue;
            }
        };
        waiting_said = false;
        let stream = match open_process(pid) {
            Ok(stream) => stream,
            // Falling back to the whole mix is right for one app and wrong for
            // several: the mix already contains the others, so a second stream
            // over it would transcribe them twice.
            Err(e) if solo => {
                tx.send(Msg::Sys(format!(
                    "hearing: {app} failed: {e}; using the whole speaker mix"
                )))?;
                let stream = open_endpoint(&dev)?;
                let outcome = pump(&stream, feed, &mut || current());
                stream.close();
                return outcome.or(Ok(()));
            }
            Err(e) => {
                tx.send(Msg::Sys(format!("hearing: {app} failed: {e}")))?;
                std::thread::sleep(Duration::from_secs(2));
                continue;
            }
        };
        tx.send(Msg::Sys(format!("hearing: {app} (pid {pid})")))?;
        if let Err(e) = pump(&stream, app_feed, &mut || alive(pid) && current())
            && current()
        {
            tx.send(Msg::Sys(format!("hearing: {app} stopped: {e}")))?;
        }
        stream.close();
        // A read error with the app still running is a device problem, not a
        // restart: reopening immediately would spin this capture thread and
        // flood Diagnostics with two `Sys` per pass. The app-closed path keeps
        // its reacquire latency — `alive` is false there.
        if alive(pid) && current() {
            std::thread::sleep(Duration::from_secs(2));
        }
    }
    Ok(())
}

/// Read, gate, segment — all microseconds — until the stream fails or
/// `still_there` says the source is gone. Never transcribes: see the worker.
fn pump(stream: &Stream, feed: &Feed, still_there: &mut dyn FnMut() -> bool) -> Result<()> {
    let Feed {
        label,
        tx,
        tune,
        utt_tx,
        gate_override,
        mute,
    } = feed;
    let mut raw: VecDeque<u8> = VecDeque::new();
    let mut frame = vec![0f32; FRAME];
    let mut cal: Vec<f32> = Vec::new();
    let mut seg = gate_override.map(Segmenter::new);
    if let Some(g) = gate_override {
        tx.send(Msg::Sys(format!("{label} gate {g:.4} (fixed)")))?;
    }
    let mut last = Instant::now();
    let calibrating_since = Instant::now();
    let mut checked = Instant::now();
    let mut previous_epoch = tune.epoch.load(Ordering::SeqCst);

    loop {
        if checked.elapsed() >= Duration::from_secs(2) {
            checked = Instant::now();
            if !still_there() {
                anyhow::bail!("the app closed");
            }
        }
        // A silent render endpoint supplies no packets. Finish calibration by
        // wall clock so the first spoken second is not consumed as room noise.
        if seg.is_none() && calibrating_since.elapsed() >= Duration::from_secs(1) {
            let g = calibrated_gate(&mut cal);
            tx.send(Msg::Sys(format!("{label} gate {g:.4}")))?;
            seg = Some(Segmenter::new(g));
        }
        stream.capture.read_from_device_to_deque(&mut raw)?;
        let epoch = tune.epoch.load(Ordering::SeqCst);
        if epoch != previous_epoch {
            raw.clear();
            if let Some(seg) = seg.as_mut() {
                seg.reset();
            }
            previous_epoch = epoch;
        }
        if !epoch.is_multiple_of(2) {
            raw.clear();
            std::thread::sleep(Duration::from_millis(10));
            continue;
        }

        if raw.len() < FRAME * 4 {
            // Loopback delivers nothing while the far end is silent, so the hang
            // timer would stall and the last turn would never flush. Feed it quiet.
            if last.elapsed() >= Duration::from_millis(FRAME_MS as u64)
                && let Some(seg) = seg.as_mut()
            {
                last = Instant::now();
                frame.fill(0.0);
                if let Some(utt) = seg.push(&frame) {
                    send(utt_tx, epoch, utt)?;
                }
            }
            std::thread::sleep(Duration::from_millis(5));
            continue;
        }

        while raw.len() >= FRAME * 4 {
            for s in frame.iter_mut() {
                let b = [
                    raw.pop_front().unwrap(),
                    raw.pop_front().unwrap(),
                    raw.pop_front().unwrap(),
                    raw.pop_front().unwrap(),
                ];
                *s = f32::from_le_bytes(b);
            }
            last = Instant::now();

            if seg.is_none() {
                // First second of audio sets the gate. Rooms, mics and headsets
                // all differ; --mic-gate / --sys-gate override it.
                cal.push(rms(&frame));
                if cal.len() >= 1000 / FRAME_MS {
                    let g = calibrated_gate(&mut cal);
                    tx.send(Msg::Sys(format!("{label} gate {g:.4}")))?;
                    seg = Some(Segmenter::new(g));
                }
                continue;
            }
            let seg = seg.as_mut().unwrap();

            if mute.is_some_and(|m| m.load(Ordering::Relaxed)) {
                frame.fill(0.0); // the AI is talking; don't hear ourselves
            }
            if let Some(utt) = seg.push(&frame) {
                send(utt_tx, epoch, utt)?;
            }
        }
    }
}

fn calibrated_gate(cal: &mut [f32]) -> f32 {
    cal.sort_by(f32::total_cmp);
    cal.get(cal.len() / 2)
        .copied()
        .unwrap_or(0.0)
        .mul_add(4.0, 0.0)
        .clamp(GATE_FLOOR, 0.95)
}

#[cfg(test)]
mod tests {
    /// Whisper invents fluent boilerplate out of near-silence, and as *text* it
    /// is indistinguishable from speech -- "Thanks for watching" is three words,
    /// clears `--min-words`, enters the 24-turn history, is written to the log
    /// and buys a paid provider request. The only place the difference still
    /// exists is the decoder's own score, so this is where it has to be caught.
    #[test]
    fn whisper_boilerplate_scored_as_silence_never_becomes_a_turn() {
        let hallucinated = vec![(0.97, "Thanks for watching!".to_string())];
        assert_eq!(super::speech(hallucinated.into_iter()), "");

        // Empty is the whole point: `run`'s `!text.is_empty()` check then
        // declines to send a turn, so nothing reaches the log, the history or
        // the coach. That is why this needs no new message or field.
        let real = vec![(0.01, "we can ship on the eleventh".to_string())];
        assert_eq!(
            super::speech(real.into_iter()),
            "we can ship on the eleventh"
        );
    }

    /// A turn is usually several segments, and whisper scores each one. Dropping
    /// the whole utterance because one segment was quiet would lose real speech;
    /// keeping the whole utterance because one segment was loud would defeat the
    /// filter. It is per segment, and the survivors are rejoined.
    #[test]
    fn one_dead_segment_does_not_take_the_rest_of_the_turn_with_it() {
        let mixed = vec![
            (0.02, "so the migration lands Tuesday".to_string()),
            (0.99, "Thank you.".to_string()),
            (0.03, "and we hold traffic until it does".to_string()),
        ];
        assert_eq!(
            super::speech(mixed.into_iter()),
            "so the migration lands Tuesday and we hold traffic until it does"
        );
    }

    /// The threshold errs at near-certain silence on purpose: a dropped real
    /// turn is worse than a wasted request, and this number is unvalidated on
    /// real audio. Anything whisper is merely unsure about still gets through.
    #[test]
    fn an_unsure_segment_is_kept_rather_than_dropped() {
        let unsure = vec![(0.6, "no, that number is wrong".to_string())];
        assert_eq!(super::speech(unsure.into_iter()), "no, that number is wrong");
    }


    fn tune_hearing(apps: &[&str]) -> Tune {
        Tune {
            prompt: std::sync::RwLock::new(String::new()),
            dump: None,
            voices: None,
            epoch: Arc::new(AtomicU64::new(0)),
            hear: std::sync::RwLock::new(apps.iter().map(|s| s.to_string()).collect()),
            hear_gen: AtomicU64::new(0),
            book: std::sync::Arc::new(std::sync::Mutex::new(crate::people::Book::load(None))),
        }
    }

    /// Clicking a row must turn off a selection that was made by substring, or
    /// `/hear chr` then a click on "chrome" would add a second entry for the
    /// same app and the row would stay lit.
    #[test]
    fn clicking_a_row_undoes_a_selection_made_by_substring() {
        let tune = tune_hearing(&["chr"]);
        assert!(tune.toggle("chrome").is_empty());
        assert_eq!(tune.hear_gen.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn toggling_adds_removes_and_retires_the_streams_each_time() {
        let tune = tune_hearing(&[]);
        assert_eq!(tune.toggle("chrome"), ["chrome"]);
        assert_eq!(tune.toggle("zoom"), ["chrome", "zoom"]);
        assert_eq!(tune.toggle("chrome"), ["zoom"]);
        // Every change has to bump the generation, or a stream serving the old
        // selection never learns to bail.
        assert_eq!(tune.hear_gen.load(Ordering::SeqCst), 3);
        assert!(tune.hearing() == vec!["zoom".to_string()]);
    }

    #[test]
    fn hear_takes_a_comma_list_and_ignores_the_gaps() {
        assert_eq!(parse_hear("chrome"), ["chrome"]);
        assert_eq!(parse_hear(" chrome , zoom "), ["chrome", "zoom"]);
        // A trailing comma is what a half-typed list looks like; it must not
        // become an empty name, which `resolve` would match against every app.
        assert_eq!(parse_hear("chrome,,zoom,"), ["chrome", "zoom"]);
        assert!(parse_hear("  ,  ").is_empty());
        assert!(parse_hear("").is_empty());
    }
    use super::*;

    #[test]
    fn silent_endpoint_calibrates_without_consuming_first_speech() {
        let gate = calibrated_gate(&mut []);
        assert_eq!(gate, GATE_FLOOR);
        let mut seg = Segmenter::new(gate);
        push_ms(&mut seg, 0.2, 1000);
        assert_eq!(push_ms(&mut seg, 0.0, 600).len(), 1);
    }

    /// Push `ms` of a tone at `amp` (rms == amp), collecting any utterances.
    fn push_ms(seg: &mut Segmenter, amp: f32, ms: usize) -> Vec<Vec<f32>> {
        let frame: Vec<f32> = (0..FRAME)
            .map(|i| if i % 2 == 0 { amp } else { -amp })
            .collect();
        (0..ms / FRAME_MS)
            .filter_map(|_| seg.push(&frame))
            .collect()
    }

    #[test]
    fn emits_one_utterance_after_the_hang() {
        let mut s = Segmenter::new(0.05);
        assert!(push_ms(&mut s, 0.0, 300).is_empty());
        assert!(push_ms(&mut s, 0.3, 1000).is_empty(), "no flush mid-speech");
        let out = push_ms(&mut s, 0.0, 900);
        assert_eq!(out.len(), 1);
        let n = out[0].len();
        assert!(n > RATE && n < RATE * 3, "preroll+speech+hang, got {n}");
    }

    #[test]
    fn drops_a_blip_below_min_speech() {
        let mut s = Segmenter::new(0.05);
        push_ms(&mut s, 0.0, 300);
        push_ms(&mut s, 0.3, 60); // a cough
        assert!(push_ms(&mut s, 0.0, 900).is_empty());
    }

    #[test]
    fn flushes_mid_monologue_at_the_cap() {
        let mut s = Segmenter::new(0.05);
        assert!(!push_ms(&mut s, 0.3, MAX_MS + 4000).is_empty());
    }

    #[test]
    fn hear_picks_the_active_session_by_name() {
        let s = |pid, active, name: &str| AppSession {
            pid,
            active,
            name: name.into(),
        };
        let list = [
            s(10, false, "Discord"),
            s(11, true, "Discord"),
            s(20, true, "chrome"),
        ];
        // Case-insensitive substring; the one actually playing wins.
        assert_eq!(resolve(&list, "discord"), Some(11));
        assert_eq!(resolve(&list, "CHROME"), Some(20));
        assert_eq!(resolve(&list, "zoom"), None);
        // Nothing active yet: the first match, so a quiet app still gets hooked
        // and picked up the moment it speaks.
        assert_eq!(resolve(&list[..1], "disc"), Some(10));
        assert_eq!(resolve(&[], "discord"), None);
    }
}
