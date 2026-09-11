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
//!
//! The clusters live in `people::Book`, which starts the call holding everyone
//! ever named rather than empty. Matching is otherwise unchanged: a known
//! person is simply a cluster that already has a centroid and a name, so
//! recognising someone from last week and recognising them from earlier in this
//! call are the same code path — which is why the book is not a second lookup
//! bolted on beside this one.

use anyhow::{Context, Result};
use ort::session::Session;
use std::path::Path;
use std::sync::{Arc, Mutex};

use crate::people::{Book, Person};

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

/// Whole conference calls exist, but eight *new* voices on one far-end stream
/// means the clustering has already failed. At the cap we answer `None` rather
/// than evict: a recycled slot silently renames someone.
///
/// Counted per call, not per book. Eight strangers in one meeting is a fault;
/// eight hundred people across a year of calls is just a year, and capping the
/// book itself would stop recognising the ninth person you ever met.
const MAX_NEW: usize = 8;

/// Under ~1.5 s the embedding is dominated by whatever phonemes happened to be
/// in the clip, not by the speaker. Cheaper and safer to refuse than to run it.
const MIN_SAMPLES: usize = crate::audio::RATE * 3 / 2;

pub struct VoiceId {
    session: Session,
    /// Shared with `route`, which owns the names. See `people`.
    book: Arc<Mutex<Book>>,
    /// `people.len()` when the call started, plus `MAX_NEW`.
    cap: usize,
}

impl VoiceId {
    pub fn new(model: &Path, book: Arc<Mutex<Book>>) -> Result<Self> {
        let cap = book.lock().map(|b| b.people.len()).unwrap_or(0) + MAX_NEW;
        Ok(Self {
            // No execution provider: 54 ms on CPU per utterance, against turns
            // that arrive seconds apart, and the GPU is busy with whisper.
            session: Session::builder()?
                .commit_from_file(model)
                .with_context(|| format!("loading {}", model.display()))?,
            book,
            cap,
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
        let cap = self.cap;
        // A poisoned lock means the naming thread panicked. The call is worth
        // more than the attribution, so this degrades to anonymous turns.
        let mut book = self.book.lock().ok()?;
        let i = assign(&mut book.people, &emb, cap)?;
        // `assign` is pure and has no clock, so the date is stamped here. The
        // unsure band returns early above: a turn nobody could attribute is not
        // evidence that anybody was heard.
        book.heard(i);
        Some(i)
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
fn assign(voices: &mut Vec<Person>, emb: &[f32], cap: usize) -> Option<usize> {
    // Must be a unit vector. A zero or NaN one scores 0 against everybody,
    // which reads as "a new speaker" and plants a cluster that then matches
    // nothing for the rest of the call.
    if !cos(emb, emb).is_normal() {
        return None;
    }
    let best = voices
        .iter()
        .enumerate()
        // A stored centroid of the wrong width came from another model. `cos`
        // zips, so scoring it would silently compare a prefix, and a prefix can
        // clear 0.70 and hand this voice someone else's name.
        .filter(|(_, p)| p.centroid.len() == emb.len())
        .map(|(i, p)| (i, cos(&p.centroid, emb)))
        .max_by(|a, b| a.1.total_cmp(&b.1));

    match best {
        Some((i, s)) if s >= SAME => {
            let p = &mut voices[i];
            p.turns += 1;
            // Running mean, then back onto the unit sphere so `cos` stays a
            // dot product. Drifts with the speaker (headset moved, voice
            // tiring) without keeping every embedding of the call around — and
            // now across calls too, so a voice first heard on a bad headset is
            // not frozen at it forever.
            for (a, b) in p.centroid.iter_mut().zip(emb) {
                *a += (b - *a) / p.turns as f32;
            }
            unit(&mut p.centroid)?;
            Some(i)
        }
        // Close to someone, but not close enough to say so.
        Some((_, s)) if s >= NEW => None,
        _ if voices.len() >= cap => None,
        _ => {
            voices.push(Person {
                name: None,
                centroid: emb.to_vec(),
                turns: 1,
                // Stamped by `Book::heard` the moment this returns. Zero is the
                // safe placeholder: `forget` reads it as ancient, and an
                // anonymous cluster is never written to the file anyway.
                last_seen: 0,
            });
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

    /// The book a call starts with. `cap` is per call, so tests that push the
    /// limit pass `n + MAX_NEW` exactly as `VoiceId::new` does.
    fn known(names: &[(&str, usize)]) -> Vec<Person> {
        names
            .iter()
            .map(|(n, axis)| Person {
                name: Some(n.to_string()),
                centroid: basis(*axis),
                turns: 1,
                last_seen: crate::people::now(),
            })
            .collect()
    }

    fn cap(voices: &[Person]) -> usize {
        voices.len() + MAX_NEW
    }

    #[test]
    fn identical_voice_reuses_its_cluster() {
        let mut v = Vec::new();
        let a = basis(0);
        assert_eq!(assign(&mut v, &a, MAX_NEW), Some(0));
        assert_eq!(assign(&mut v, &a, MAX_NEW), Some(0));
        assert_eq!(v.len(), 1, "one voice, one cluster");
        assert_eq!(v[0].turns, 2, "centroid absorbed both utterances");
        assert_eq!(v[0].name, None, "clustering does not invent names");
    }

    /// The whole point of the book: someone named on an earlier call is
    /// recognised on this one without saying their name again.
    #[test]
    fn a_voice_from_a_previous_call_keeps_its_name() {
        let mut v = known(&[("Ada Lovelace", 0), ("Grace Hopper", 1)]);
        let room = cap(&v);
        assert_eq!(assign(&mut v, &basis(1), room), Some(1));
        assert_eq!(v[1].name.as_deref(), Some("Grace Hopper"));
        assert_eq!(v[1].turns, 2, "and goes on learning the voice");
        assert_eq!(v.len(), 2, "a known voice is not a new cluster");
    }

    /// A stranger on a call with people already in the book is still a
    /// stranger — the book must not stretch to claim them.
    #[test]
    fn a_stranger_beside_known_voices_is_new_and_nameless() {
        let mut v = known(&[("Ada Lovelace", 0)]);
        let room = cap(&v);
        assert_eq!(assign(&mut v, &basis(2), room), Some(1));
        assert_eq!(v[1].name, None);
        // And the unsure band still applies against a *stored* voice, which is
        // the case that would put last week's name on today's stranger.
        let mut v = known(&[("Ada Lovelace", 0)]);
        let room = cap(&v);
        assert_eq!(assign(&mut v, &tilted(1, 0.60), room), None);
        assert_eq!(v.len(), 1, "an unsure turn must not invent a speaker");
    }

    /// A book written by a different embedding model would otherwise be scored
    /// on its first 16 dimensions, and a prefix can clear 0.70.
    #[test]
    fn a_centroid_of_the_wrong_width_is_never_matched() {
        let mut v = vec![Person {
            name: Some("From another model".into()),
            centroid: vec![1.0; DIM * 2],
            turns: 9,
            last_seen: crate::people::now(),
        }];
        let room = cap(&v);
        assert_eq!(assign(&mut v, &basis(0), room), Some(1));
        assert_eq!(v[1].name, None, "scored a prefix and adopted the name");
    }

    #[test]
    fn a_different_voice_gets_a_new_cluster() {
        let mut v = Vec::new();
        assert_eq!(assign(&mut v, &basis(0), MAX_NEW), Some(0));
        assert_eq!(assign(&mut v, &basis(1), MAX_NEW), Some(1));
    }

    #[test]
    fn the_band_between_the_thresholds_is_unsure() {
        let mut v = Vec::new();
        assign(&mut v, &basis(0), MAX_NEW);
        // 0.60: too far to be them, too close to be someone else.
        assert_eq!(assign(&mut v, &tilted(1, 0.60), MAX_NEW), None);
        assert_eq!(v.len(), 1, "an unsure turn must not invent a speaker");
        assert_eq!(v[0].turns, 1, "nor pollute the centroid it nearly matched");
    }

    #[test]
    fn the_thresholds_are_inclusive_at_the_edges() {
        let mut v = Vec::new();
        assign(&mut v, &basis(0), MAX_NEW);
        assert_eq!(assign(&mut v, &tilted(1, SAME), MAX_NEW), Some(0));
        let mut v = Vec::new();
        assign(&mut v, &basis(0), MAX_NEW);
        assert_eq!(assign(&mut v, &tilted(1, NEW), MAX_NEW), None);
    }

    #[test]
    fn the_cap_refuses_rather_than_evicts() {
        let mut v = Vec::new();
        for i in 0..MAX_NEW {
            assert_eq!(assign(&mut v, &basis(i), MAX_NEW), Some(i));
        }
        assert_eq!(assign(&mut v, &basis(MAX_NEW), MAX_NEW), None);
        assert_eq!(v.len(), MAX_NEW, "nobody was renamed to make room");
        // The voices already known still resolve.
        assert_eq!(assign(&mut v, &basis(0), MAX_NEW), Some(0));
    }

    /// The cap counts this call's strangers, not the address book. A book of
    /// eight would otherwise refuse to hear a ninth person ever again.
    #[test]
    fn a_full_book_does_not_use_up_this_calls_budget() {
        let mut v = known(&[
            ("A", 0),
            ("B", 1),
            ("C", 2),
            ("D", 3),
            ("E", 4),
            ("F", 5),
            ("G", 6),
            ("H", 7),
        ]);
        let room = cap(&v);
        assert_eq!(v.len(), MAX_NEW, "the book alone is already at the old cap");
        assert_eq!(assign(&mut v, &basis(8), room), Some(8));
        assert_eq!(v[8].name, None);
    }

    #[test]
    fn a_dead_embedding_is_unsure_not_a_nan_cluster() {
        let mut v = Vec::new();
        assert_eq!(assign(&mut v, &basis(0), MAX_NEW), Some(0));
        // All-zero would divide by zero in the centroid update.
        assert_eq!(assign(&mut v, &[0f32; DIM], MAX_NEW), None);
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
        let book = Arc::new(Mutex::new(Book::load(None)));
        let mut v = VoiceId::new(Path::new(MODEL), book).expect("load model");

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

    const SEG: &str = "models/sherpa-onnx-pyannote-segmentation-3-0/model.onnx";
    /// The export's fixed window: 10 s at 16 kHz. See its embedded metadata.
    const SEG_WINDOW: usize = 160_000;

    /// Does a segmentation model fit the budget, and do the real dump clips
    /// actually contain the overlapping speech that would explain the
    /// over-splitting seen on this call?
    ///
    /// `assign` cannot answer either question: it is handed one embedding for a
    /// whole utterance and has no way to know two people made it. This runs
    /// pyannote-segmentation-3.0, whose seven outputs are the powerset of three
    /// speakers — indices 4..6 are *pairs*, so overlap is read off the model
    /// rather than inferred.
    ///
    /// Skips itself if either the model or a `--dump` clip is missing.
    #[test]
    fn segmentation_finds_overlap_and_fits_the_budget() {
        use std::time::Instant;
        if !Path::new(SEG).exists() {
            eprintln!("skipping: {SEG} not downloaded");
            return;
        }
        let clips: Vec<_> = std::fs::read_dir("clips")
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "wav"))
            .filter_map(|p| Some((p.display().to_string(), wav(p.to_str()?)?)))
            .collect();
        if clips.is_empty() {
            eprintln!("skipping: no clips/*.wav from --dump");
            return;
        }

        let mut session = Session::builder()
            .expect("builder")
            .commit_from_file(SEG)
            .expect("load segmentation model");

        let mut worst = 0u128;
        for (name, mut audio) in clips {
            let secs = audio.len() as f32 / crate::audio::RATE as f32;
            audio.resize(SEG_WINDOW.max(audio.len()), 0.0);
            let frames_used = (secs * 16_000.0 / 270.0) as usize;

            let t = Instant::now();
            let x = ([1, 1, audio.len()], audio);
            let out = session
                .run(ort::inputs!["x" => ort::value::Tensor::from_array(x).expect("tensor")])
                .expect("run");
            let (shape, y) = out
                .get("y")
                .expect("output `y`")
                .try_extract_tensor::<f32>()
                .expect("extract");
            let ms = t.elapsed().as_millis();
            worst = worst.max(ms);

            let classes = *shape.last().expect("rank") as usize;
            assert_eq!(classes, 7, "powerset of 3 speakers, max 2 at once");
            // Only the frames the clip actually covers; the rest is zero pad.
            let mut seen = [0usize; 7];
            for f in y.chunks(classes).take(frames_used) {
                let (best, _) = f
                    .iter()
                    .enumerate()
                    .max_by(|a, b| a.1.total_cmp(b.1))
                    .expect("argmax");
                seen[best] += 1;
            }
            let total: usize = seen.iter().sum::<usize>().max(1);
            let overlap = seen[4..].iter().sum::<usize>();
            let voices = (1..4).filter(|&i| seen[i] * 20 > total).count();
            println!(
                "{name:<34} {secs:>5.1}s  {ms:>4}ms  voices {voices}  overlap {:>4.1}%  {seen:?}",
                100.0 * overlap as f32 / total as f32
            );
        }
        // It rides the scoped thread beside CAM++ (54 ms) and whisper (~100 ms),
        // so it is free only while it stays under them. A regression here is a
        // regression in time to first word.
        assert!(
            worst < 100,
            "segmentation cost {worst} ms, budget is whisper"
        );
    }
}
