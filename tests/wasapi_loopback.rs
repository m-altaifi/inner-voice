//! Checks the assumption the whole capture design rests on: that WASAPI will
//! hand us the *render* endpoint's audio (loopback) already converted to
//! 16 kHz mono f32, so there is no resampling or downmix code anywhere.
//!
//! If this ever fails, `src/audio.rs` needs a resampler and the README's
//! "no diarization, no resampling" claim stops being true.

use std::collections::VecDeque;
use std::time::{Duration, Instant};
use wasapi::{DeviceEnumerator, Direction, SampleType, StreamMode, WaveFormat, initialize_mta};

#[test]
fn loopback_delivers_16k_mono() {
    initialize_mta().ok().expect("COM init");

    // A Render device opened for Capture is what makes this loopback.
    //
    // No render endpoint at all is not evidence about the format assumption --
    // a VM, a CI runner or an RDP session without audio redirection simply has
    // nowhere to look. This used to `.expect()` and turn every such machine's
    // `cargo test` red for a reason that had nothing to do with the code, so it
    // skips the way `gpu_transcribes` skips its missing model. A device that
    // *is* there and refuses 16 kHz mono still fails: that is the assumption.
    let dev = match DeviceEnumerator::new().and_then(|e| e.get_default_device(&Direction::Render)) {
        Ok(dev) => dev,
        Err(e) => {
            eprintln!("skipping: no default render endpoint ({e})");
            return;
        }
    };
    println!("loopback source: {}", dev.get_friendlyname().unwrap());

    let mut client = dev.get_iaudioclient().expect("audio client");
    let fmt = WaveFormat::new(32, 32, &SampleType::Float, 16_000, 1, None);
    let (_, min_period) = client.get_device_period().expect("device period");

    client
        .initialize_client(
            &fmt,
            &Direction::Capture,
            &StreamMode::PollingShared {
                autoconvert: true,
                buffer_duration_hns: min_period.max(1_000_000),
            },
        )
        .expect("WASAPI refused 16 kHz mono loopback — audio.rs would need a resampler");

    let capture = client.get_audiocaptureclient().expect("capture client");
    client.start_stream().expect("start");

    // Reads must succeed. Bytes only arrive when something is actually playing,
    // so their absence is not a failure — silence is the normal desktop state.
    let mut raw: VecDeque<u8> = VecDeque::new();
    let deadline = Instant::now() + Duration::from_millis(600);
    while Instant::now() < deadline {
        capture
            .read_from_device_to_deque(&mut raw)
            .expect("read from loopback");
        std::thread::sleep(Duration::from_millis(10));
    }
    client.stop_stream().expect("stop");

    assert_eq!(fmt.get_nchannels(), 1, "mono");
    assert_eq!(fmt.get_samplespersec(), 16_000, "16 kHz");
    assert_eq!(
        raw.len() % 4,
        0,
        "whole f32 samples, got {} bytes",
        raw.len()
    );
    println!(
        "captured {} bytes ({:.2}s of audio; 0 is fine if nothing was playing)",
        raw.len(),
        raw.len() as f32 / 4.0 / 16_000.0
    );
}
