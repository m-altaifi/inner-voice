//! Proves the CUDA build actually works, not just that it linked.
//!
//! This matters here specifically: CUDA 12.8 refuses MSVC 2026, and `build.ps1`
//! gets past that with nvcc's `-allow-unsupported-compiler`, which NVIDIA warns
//! "may cause incorrect run time execution". A clean compile is not evidence.
//! Transcribing known speech is.
//!
//! Skips itself if the model has not been downloaded yet (see README).

use std::path::Path;
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

const MODEL: &str = "models/ggml-large-v3-turbo-q5_0.bin";
const WAV: &str = "tests/data/jfk.wav";

#[test]
fn gpu_transcribes_known_speech() {
    if !Path::new(MODEL).exists() {
        eprintln!("skipping: {MODEL} not downloaded");
        return;
    }

    let samples: Vec<f32> = hound::WavReader::open(WAV)
        .expect("open jfk.wav")
        .into_samples::<i16>()
        .map(|s| s.expect("sample") as f32 / 32768.0)
        .collect();

    // Same settings the live capture path uses — this test is only evidence
    // about production if it runs production's numerics. Flash attention and a
    // reduced `audio_ctx` both change them, which is the whole point of
    // asserting on words below. Mirrors `src/audio.rs`; a binary crate has no
    // lib target to import the constants from.
    let mut cp = WhisperContextParameters::default();
    cp.flash_attn(true);
    let ctx = WhisperContext::new_with_params(MODEL, cp).expect("load model");
    let mut state = ctx.create_state().expect("state");

    let mut p = FullParams::new(SamplingStrategy::BeamSearch {
        beam_size: 5,
        patience: -1.0,
    });
    p.set_language(Some("en"));
    p.set_no_context(true);
    p.set_print_special(false);
    p.set_print_progress(false);
    p.set_print_realtime(false);
    p.set_print_timestamps(false);
    let audio_secs = samples.len() as f32 / 16_000.0;
    let t0 = std::time::Instant::now();
    state.full(p, &samples).expect("transcribe");
    let elapsed = t0.elapsed();
    println!(
        "transcribed {audio_secs:.1}s of audio in {:?} ({:.0} ms per second of speech)",
        elapsed,
        elapsed.as_secs_f32() * 1000.0 / audio_secs
    );

    let text = state
        .as_iter()
        .map(|s| s.to_string())
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();

    println!("transcript: {text}");
    // Garbled CUDA output would still be *some* string, so assert on content.
    assert!(
        text.contains("ask not what your country can do for you"),
        "GPU transcription is wrong — suspect the unsupported-compiler build. Got: {text}"
    );
}
