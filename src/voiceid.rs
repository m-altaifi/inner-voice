//! Which far-end voice just spoke.
//!
//! The far end arrives as one mixed loopback stream, so unlike YOU/THEM there
//! is nothing structural to separate two people sharing a speakerphone. This
//! embeds each utterance the VAD already cut out (CAM++, 512-d) and matches it
//! against the voices heard so far. Online clustering, not diarization: there
//! is no recording to re-segment, only the turn that just ended.
//!
//! `None` is a first-class answer here — "never guess, a wrong name is worse
//! than no name" is the product rule, and the caller shows THEM for it.

use anyhow::{Context, Result};
use ort::session::Session;
use std::path::Path;

/// ponytail: PROVISIONAL, NOT VALIDATED. Must be tuned on real call audio
/// captured with `--dump`. The only measurement so far is two Microsoft TTS
/// voices scoring 0.72–0.84 against *each other* — synthetic voices share a
/// vocoder, so that is evidence about the synthesiser, not about people. Real
/// speakers over a codec will land somewhere else entirely, and the gap
/// between these two numbers is the whole safety margin: widen it if the
/// panel ever attributes a turn to the wrong person.
///
/// The one real-speech measurement so far says 0.70 is too high: two halves
/// of the same speaker in `tests/data/jfk.wav` score 0.591, which lands in the
/// unsure band. Left at 0.70 deliberately — erring towards "not sure" is the
/// safe direction, and the fix is a tuning pass on `--dump` audio, not a guess.
const SAME: f32 = 0.70;
/// Below this, nothing heard so far is a plausible match — a new voice.
const NEW: f32 = 0.50;

/// Whole conference calls exist, but eight distinct voices on one far-end
/// stream means the clustering has already failed. At the cap we answer `None`
/// rather than evict: a recycled slot silently renames someone.
const MAX_VOICES: usize = 8;

/// Under ~1.5 s the embedding is dominated by whatever phonemes happened to be
/// in the clip, not by the speaker. Cheaper and safer to refuse than to run it.
const MIN_SAMPLES: usize = crate::audio::RATE * 3 / 2;

/// A learned voice: unit-length centroid, and how many utterances built it.
type Voice = (Vec<f32>, u32);

pub struct VoiceId {
    session: Session,
    voices: Vec<Voice>,
}

impl VoiceId {
    pub fn new(model: &Path) -> Result<Self> {
        Ok(Self {
            // No execution provider: 54 ms on CPU per utterance, against turns
            // that arrive seconds apart, and the GPU is busy with whisper.
            session: Session::builder()?
                .commit_from_file(model)
                .with_context(|| format!("loading {}", model.display()))?,
            voices: Vec::new(),
        })
    }

    /// Cluster index for this utterance, or `None` for "not sure".
    /// 16 kHz mono f32.
    pub fn identify(&mut self, audio: &[f32]) -> Option<usize> {
        if audio.len() < MIN_SAMPLES {
            return None;
        }
        // A failed inference is indistinguishable from ambiguity to the caller
        // — both mean "show THEM" — so it collapses into the same answer
        // rather than growing an error path nobody can act on mid-call.
        let emb = self.embed(audio).ok()?;
        assign(&mut self.voices, &emb)
    }

    /// One utterance -> one unit-length embedding.
    fn embed(&mut self, audio: &[f32]) -> Result<Vec<f32>> {
        // The model eats kaldi fbank, not waveform. `knf_rs` mean-normalises.
        let fbank = knf_rs::compute_fbank(audio).map_err(|e| anyhow::anyhow!("{e}"))?;
        let (frames, bins) = fbank.dim();
        // A plain tuple on purpose: `ort` is on ndarray 0.17 and `knf-rs` on
        // 0.16, so the two `Array2` types are unrelated to the compiler and
        // handing one to the other does not typecheck.
        let x = ([1, frames, bins], fbank.into_raw_vec_and_offset().0);
        let out = self
            .session
            .run(ort::inputs!["x" => ort::value::Tensor::from_array(x)?])?;
        let (_shape, data) = out
            .get("embedding")
            .context("model has no `embedding` output")?
            .try_extract_tensor::<f32>()?;

        let mut emb = data.to_vec();
        // Normalised once here so every later comparison is a dot product.
        unit(&mut emb).context("degenerate embedding")?;
        Ok(emb)
    }
}

/// Scale to unit length. `None` if there is no direction to keep — a silent or
/// broken clip can produce one, and dividing by it yields NaNs that would
/// compare as "not sure" forever after.
fn unit(v: &mut [f32]) -> Option<()> {
    let n = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    (n.is_normal()).then(|| v.iter_mut().for_each(|x| *x /= n))
}

fn cos(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

/// Match an embedding against the voices heard so far, learning as it goes.
///
/// Split out from the model so the decision — the part that can attribute a
/// turn to the wrong person — is testable without the 28 MB .onnx.
///
/// Two thresholds, not one: a single cutoff has to call every borderline case
/// either a match or a new speaker, and both mistakes are visible on the panel.
/// The band between them is where honest uncertainty lives.
fn assign(voices: &mut Vec<Voice>, emb: &[f32]) -> Option<usize> {
    // Must be a unit vector. A zero or NaN one scores 0 against everybody,
    // which reads as "a new speaker" and plants a cluster that then matches
    // nothing for the rest of the call.
    if !cos(emb, emb).is_normal() {
        return None;
    }
    let best = voices
        .iter()
        .enumerate()
        .map(|(i, (c, _))| (i, cos(c, emb)))
        .max_by(|a, b| a.1.total_cmp(&b.1));

    match best {
        Some((i, s)) if s >= SAME => {
            let (c, n) = &mut voices[i];
            *n += 1;
            // Running mean, then back onto the unit sphere so `cos` stays a
            // dot product. Drifts with the speaker (headset moved, voice
            // tiring) without keeping every embedding of the call around.
            for (a, b) in c.iter_mut().zip(emb) {
                *a += (b - *a) / *n as f32;
            }
            unit(c)?;
            Some(i)
        }
        // Close to someone, but not close enough to say so.
        Some((_, s)) if s >= NEW => None,
        _ if voices.len() >= MAX_VOICES => None,
        _ => {
            voices.push((emb.to_vec(), 1));
            Some(voices.len() - 1)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MODEL: &str = "models/campplus_sv_en_voxceleb_16k.onnx";
    const WAV: &str = "tests/data/jfk.wav";
    const DIM: usize = 16;

    /// Unit vector with `w` on axis 0 and the rest on axis `axis`, so its
    /// cosine against `basis(0, 1.0)` is exactly `w`.
    fn tilted(axis: usize, w: f32) -> Vec<f32> {
        let mut v = vec![0f32; DIM];
        v[0] = w;
        v[axis] = (1.0 - w * w).sqrt();
        v
    }

    fn basis(axis: usize) -> Vec<f32> {
        tilted(axis, 0.0)
    }

    #[test]
    fn identical_voice_reuses_its_cluster() {
        let mut v = Vec::new();
        let a = basis(0);
        assert_eq!(assign(&mut v, &a), Some(0));
        assert_eq!(assign(&mut v, &a), Some(0));
        assert_eq!(v.len(), 1, "one voice, one cluster");
        assert_eq!(v[0].1, 2, "centroid absorbed both utterances");
    }

    #[test]
    fn a_different_voice_gets_a_new_cluster() {
        let mut v = Vec::new();
        assert_eq!(assign(&mut v, &basis(0)), Some(0));
        assert_eq!(assign(&mut v, &basis(1)), Some(1));
    }

    #[test]
    fn the_band_between_the_thresholds_is_unsure() {
        let mut v = Vec::new();
        assign(&mut v, &basis(0));
        // 0.60: too far to be them, too close to be someone else.
        assert_eq!(assign(&mut v, &tilted(1, 0.60)), None);
        assert_eq!(v.len(), 1, "an unsure turn must not invent a speaker");
        assert_eq!(v[0].1, 1, "nor pollute the centroid it nearly matched");
    }

    #[test]
    fn the_thresholds_are_inclusive_at_the_edges() {
        let mut v = Vec::new();
        assign(&mut v, &basis(0));
        assert_eq!(assign(&mut v, &tilted(1, SAME)), Some(0));
        let mut v = Vec::new();
        assign(&mut v, &basis(0));
        assert_eq!(assign(&mut v, &tilted(1, NEW)), None);
    }

    #[test]
    fn the_cap_refuses_rather_than_evicts() {
        let mut v = Vec::new();
        for i in 0..MAX_VOICES {
            assert_eq!(assign(&mut v, &basis(i)), Some(i));
        }
        assert_eq!(assign(&mut v, &basis(MAX_VOICES)), None);
        assert_eq!(v.len(), MAX_VOICES, "nobody was renamed to make room");
        // The voices already known still resolve.
        assert_eq!(assign(&mut v, &basis(0)), Some(0));
    }

    #[test]
    fn a_dead_embedding_is_unsure_not_a_nan_cluster() {
        let mut v = Vec::new();
        assert_eq!(assign(&mut v, &basis(0)), Some(0));
        // All-zero would divide by zero in the centroid update.
        assert_eq!(assign(&mut v, &[0f32; DIM]), None);
    }

    fn wav(path: &str) -> Option<Vec<f32>> {
        Some(
            hound::WavReader::open(path)
                .ok()?
                .into_samples::<i16>()
                .map(|s| s.expect("sample") as f32 / 32768.0)
                .collect(),
        )
    }

    /// End to end on real audio, and the only place any measured cosine in this
    /// repo comes from — run with `RUST_TEST_NOCAPTURE=1` when tuning.
    ///
    /// It asserts the product rule, not the thresholds: one voice must never
    /// come back as two speakers. `None` is always an acceptable answer, so
    /// this stays true whatever SAME and NEW end up being.
    ///
    /// Skips itself if the model has not been downloaded yet (see README).
    #[test]
    fn one_voice_never_comes_back_as_two() {
        if !Path::new(MODEL).exists() {
            eprintln!("skipping: {MODEL} not downloaded");
            return;
        }
        let jfk = wav(WAV).expect("open jfk.wav");
        let (a, b) = jfk.split_at(jfk.len() / 2);
        let mut v = VoiceId::new(Path::new(MODEL)).expect("load model");

        assert_eq!(v.identify(&a[..8_000]), None, "0.5 s is too short to judge");

        // The TTS clips are the spike's, and share a vocoder — their numbers
        // say nothing about real speakers. Printed anyway: they are free, and
        // the jfk pair beside them is the one honest data point.
        let mut clips = vec![("jfk-1st", a.to_vec()), ("jfk-2nd", b.to_vec())];
        for name in ["david_a", "david_b", "hazel_a", "zira_a"] {
            if let Some(s) = wav(&format!("tests/data/spk/{name}.wav")) {
                clips.push((name, s));
            }
        }
        let embs: Vec<_> = clips
            .iter()
            .map(|(n, s)| (*n, v.embed(s).expect("embed")))
            .collect();
        for (i, (n, e)) in embs.iter().enumerate() {
            for (m, f) in &embs[i + 1..] {
                println!("cos {n:>8} {m:>8}  {:.3}", cos(e, f));
            }
        }

        assert_eq!(v.identify(a), Some(0), "first voice heard is cluster 0");
        assert_ne!(
            v.identify(b),
            Some(1),
            "same speaker split into two clusters"
        );
    }
}
