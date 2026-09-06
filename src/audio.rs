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
    pub prompt: String,
    /// Where to save every utterance for later listening, if asked.
    pub dump: Option<std::path::PathBuf>,
    /// Speaker-embedding model. `None` — including a missing file — simply means
    /// far-end turns stay unattributed, which is a working call, not an error.
    pub voices: Option<std::path::PathBuf>,
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
fn warm(state: &mut WhisperState) {
    let _ = state.full(params(""), &vec![0f32; RATE]); // output is not the point
}

/// Returns the text rather than sending it: the caller also needs the speaker
/// embedding, which is computed concurrently, and the turn cannot be stamped
/// until both have landed.
fn transcribe(state: &mut WhisperState, audio: &[f32], tune: &Tune) -> Result<String> {
    state.full(params(&tune.prompt), audio)?;
    Ok(state
        .as_iter()
        .map(|s| s.to_string())
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string())
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
    /// Hear only this app's process tree as `THEM` (`--hear`). `None` is the
    /// endpoint mix, as before.
    pub hear: Option<String>,
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
        hear,
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
    // `who` moves to the worker, which is what stamps each turn; the capture
    // loop keeps only the display label for its own status messages.
    let label = who.label().to_string();
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
                (true, Some(path)) => match crate::voiceid::VoiceId::new(path) {
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
                    if let Err(e) = save(dir, turn.label(), &utt, &text) {
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
    let Some(app) = hear else {
        // The endpoint mix, exactly as before: a stream error ends the thread
        // and `main` reports it.
        let stream = open_endpoint(&dev)?;
        return pump(&stream, &feed, &mut || true);
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

    // The app may not be running yet — the call app usually starts second —
    // and may restart with a new pid mid-call. Neither is an error here.
    let mut waiting_said = false;
    loop {
        let pid = match sessions(&dev).map(|list| resolve(&list, &app)) {
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
            Err(e) => {
                tx.send(Msg::Sys(format!(
                    "hearing: {app} failed: {e}; using the whole speaker mix"
                )))?;
                let stream = open_endpoint(&dev)?;
                return pump(&stream, &feed, &mut || true);
            }
        };
        tx.send(Msg::Sys(format!("hearing: {app} (pid {pid})")))?;
        if let Err(e) = pump(&stream, &app_feed, &mut || alive(pid)) {
            tx.send(Msg::Sys(format!("hearing: {app} stopped: {e}")))?;
        }
        stream.close();
        // A read error with the app still running is a device problem, not a
        // restart: reopening immediately would spin this capture thread and
        // flood Diagnostics with two `Sys` per pass. The app-closed path keeps
        // its reacquire latency — `alive` is false there.
        if alive(pid) {
            std::thread::sleep(Duration::from_secs(2));
        }
    }
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
