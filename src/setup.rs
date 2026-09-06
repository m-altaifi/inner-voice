//! `--setup`: every prerequisite, ✓ or ✗, with the fix printed next to it.
//!
//! Before a call is the only time a failing prerequisite is cheap. The provider
//! line *measures* time to first token rather than merely checking the key, so
//! choosing between providers is a decision made from evidence — the same
//! number the panel shows as `first word` on a live turn.
use crate::{audio, coach, extract, provider, speak};
use std::fmt::Write as _;
use std::path::Path;
use wasapi::{DeviceEnumerator, Direction};
use whisper_rs::{WhisperContext, WhisperContextParameters};

pub struct Check {
    pub name: &'static str,
    pub ok: bool,
    pub detail: String,
}

/// What `--setup` needs from the command line, copied out of `Args` so this
/// module does not depend on clap.
pub struct Inputs {
    pub whisper: String,
    pub provider: String,
    pub model: Option<String>,
    pub mic: Option<String>,
    pub loopback: Option<String>,
    pub knowledge: String,
    pub references: String,
    pub agent_cmd: Option<String>,
    pub voice: Option<String>,
}

pub fn run(inputs: &Inputs, enumerator: &DeviceEnumerator) -> Vec<Check> {
    let mut checks = Vec::new();

    let model = Path::new(&inputs.whisper);
    checks.push(match model.metadata() {
        Ok(m) => Check {
            name: "model",
            ok: true,
            detail: format!("{} ({} MB)", inputs.whisper, m.len() / 1_000_000),
        },
        Err(_) => Check {
            name: "model",
            ok: false,
            detail: format!(
                "{} missing — curl.exe -fL -o {} https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3-turbo-q5_0.bin",
                inputs.whisper, inputs.whisper
            ),
        },
    });

    // "Does it run", not accuracy: one warm-up inference on the GPU, timed.
    checks.push(if model.exists() {
        let started = std::time::Instant::now();
        let mut params = WhisperContextParameters::default();
        params.flash_attn(true);
        match WhisperContext::new_with_params(&inputs.whisper, params)
            .and_then(|ctx| ctx.create_state())
        {
            Ok(mut state) => {
                audio::warm(&mut state);
                Check {
                    name: "cuda",
                    ok: true,
                    detail: format!(
                        "model loaded and warmed in {} ms",
                        started.elapsed().as_millis()
                    ),
                }
            }
            Err(e) => Check {
                name: "cuda",
                ok: false,
                detail: format!("{e} — check the NVIDIA driver and CUDA install; see README"),
            },
        }
    } else {
        Check {
            name: "cuda",
            ok: false,
            detail: "skipped: no model to load".into(),
        }
    });

    for (name, dir, wanted) in [
        ("microphone", Direction::Capture, &inputs.mic),
        ("speakers", Direction::Render, &inputs.loopback),
    ] {
        checks.push(
            match crate::pick(enumerator, dir, wanted).and_then(|d| Ok(d.get_friendlyname()?)) {
                Ok(friendly) => Check {
                    name,
                    ok: true,
                    detail: friendly,
                },
                Err(e) => Check {
                    name,
                    ok: false,
                    detail: format!("{e:#} — run --list-devices and set --mic / --loopback"),
                },
            },
        );
    }

    checks.push(
        match crate::pick(enumerator, Direction::Render, &inputs.loopback)
            .and_then(|d| audio::sessions(&d))
        {
            Ok(apps) if apps.is_empty() => Check {
                name: "apps",
                ok: true,
                detail: "nothing is playing right now; --list-apps shows apps as they play".into(),
            },
            Ok(mut apps) => {
                apps.sort_by_key(|a| !a.active);
                let names: Vec<String> = apps
                    .iter()
                    .map(|a| {
                        if a.active {
                            format!("{} (active)", a.name)
                        } else {
                            a.name.clone()
                        }
                    })
                    .collect();
                Check {
                    name: "apps",
                    ok: true,
                    detail: format!("{} — --hear takes any part of a name", names.join(", ")),
                }
            }
            Err(e) => Check {
                name: "apps",
                ok: false,
                detail: format!("{e:#}"),
            },
        },
    );

    checks.push(if inputs.provider == "none" {
        Check {
            name: "provider",
            ok: true,
            detail: "none — transcription only, nothing leaves the machine".into(),
        }
    } else {
        match provider::resolve(&inputs.provider, inputs.model.as_deref()) {
            Ok(p) => match coach::probe(&p) {
                Ok(ttft) => Check {
                    name: "provider",
                    ok: true,
                    detail: format!(
                        "{} {} — first token {} ms",
                        inputs.provider,
                        p.model,
                        ttft.as_millis()
                    ),
                },
                Err(e) => Check {
                    name: "provider",
                    ok: false,
                    detail: format!("{} {} — {e:#}", inputs.provider, p.model),
                },
            },
            Err(e) => Check {
                name: "provider",
                ok: false,
                detail: format!("{e:#}"),
            },
        }
    });

    // Which voice `--speak` will actually use, and what else is installed.
    // SAPI's own default is one of the three Desktop voices, which sound like
    // 1998; this is where a user finds out there are better ones and what to
    // name. Not a failure when a `--voice` misses — the panel still talks.
    checks.push({
        let installed = speak::available();
        let chosen = speak::pick(&installed, inputs.voice.as_deref());
        Check {
            name: "voice",
            ok: true,
            detail: match (chosen, installed.is_empty()) {
                (Some(i), _) => format!(
                    "{} — --voice takes any part of a name, from: {}",
                    installed[i],
                    installed.join(", ")
                ),
                (None, false) => format!(
                    "no voice matches {:?}; --speak would use the default. Installed: {}",
                    inputs.voice.clone().unwrap_or_default(),
                    installed.join(", ")
                ),
                (None, true) => "no OneCore voices installed; --speak uses the SAPI default. Windows 11: Settings > Accessibility > Narrator > Add natural voices".into(),
            },
        }
    });

    checks.push(if extract::ocr_available() {
        Check {
            name: "ocr",
            ok: true,
            detail: "Windows OCR available for image references".into(),
        }
    } else {
        Check {
            name: "ocr",
            ok: false,
            detail: "no Windows OCR language installed — Settings > Time & language > Language & region > add a language, then its Optical character recognition feature (only needed for image files)".into(),
        }
    });

    checks.push(folder("knowledge", &inputs.knowledge, false));
    checks.push(priming(&inputs.knowledge));
    checks.push(folder("references", &inputs.references, true));

    checks.push(match &inputs.agent_cmd {
        Some(exe) if crate::agent::validate_executable(Path::new(exe)).is_err() => Check {
            name: "research",
            ok: false,
            detail: "unsupported research CLI: use Claude Code's claude.exe, or unset IV_AGENT_CMD / --agent-cmd for provider research".into(),
        },
        Some(exe) if Path::new(exe).is_file() => Check {
            name: "research",
            ok: true,
            detail: format!("F8 routes through {exe}"),
        },
        Some(exe) => Check {
            name: "research",
            ok: false,
            detail: format!("--agent-cmd {exe} not found"),
        },
        None => Check {
            name: "research",
            ok: true,
            detail: "optional: F8 uses the provider; set IV_AGENT_CMD to route it through Claude Code"
                .into(),
        },
    });

    checks
}

/// What the corpus does to *whisper*, which its file count cannot show.
///
/// `knowledge::glossary` is pinned into `initial_prompt`, so every term here is
/// a word the transcriber is biased toward hearing — and a bias is only visible
/// in the output as a name that arrives from nowhere. A sample `attendees.csv`
/// this project once shipped put an invented "Sarah Chen" into real transcripts
/// exactly that way. Printing the terms before the call is the cheap version of
/// finding out during one.
fn priming(dir: &str) -> Check {
    let terms = match crate::knowledge::load(Path::new(dir)) {
        Ok(text) => crate::knowledge::glossary(&text),
        // The same unreadable file that stops startup; `folder` counts bytes and
        // would call it fine.
        Err(e) => {
            return Check {
                name: "priming",
                ok: false,
                detail: format!("{dir} could not be read: {e}"),
            };
        }
    };
    let terms = terms
        .trim_start_matches("Glossary: ")
        .trim_end_matches('.')
        .to_string();
    Check {
        name: "priming",
        ok: true,
        detail: if terms.is_empty() {
            "nothing — whisper hears only what is said".into()
        } else {
            // Six is enough to recognise your own corpus and notice someone
            // else's; the whole list can run to 800 characters.
            let shown: Vec<&str> = terms.split(", ").take(6).collect();
            let more = terms.split(", ").count().saturating_sub(shown.len());
            format!(
                "whisper is biased toward: {}{}",
                shown.join(", "),
                if more > 0 {
                    format!(" (+{more} more)")
                } else {
                    String::new()
                }
            )
        },
    }
}

fn folder(name: &'static str, dir: &str, create: bool) -> Check {
    let path = Path::new(dir);
    if !path.is_dir() {
        if create && std::fs::create_dir_all(path).is_ok() {
            return Check {
                name,
                ok: true,
                detail: format!("{dir} — created, empty"),
            };
        }
        return Check {
            name,
            ok: !create,
            detail: format!(
                "{dir} not found{}",
                if create {
                    ""
                } else {
                    " — optional; briefs go here"
                }
            ),
        };
    }
    let (mut files, mut bytes) = (0usize, 0u64);
    if let Ok(entries) = std::fs::read_dir(path) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_file() && extract::supported(&p) {
                files += 1;
                bytes += entry.metadata().map(|m| m.len()).unwrap_or(0);
            }
        }
    }
    Check {
        name,
        ok: true,
        detail: format!("{dir} — {files} files, {} KB", bytes / 1024),
    }
}

pub fn all_ok(checks: &[Check]) -> bool {
    checks.iter().all(|c| c.ok)
}

pub fn render(checks: &[Check]) -> String {
    let mut out = String::new();
    for c in checks {
        let _ = writeln!(
            out,
            "  {} {:<11} {}",
            if c.ok { '✓' } else { '✗' },
            c.name,
            c.detail
        );
    }
    let failed = checks.iter().filter(|c| !c.ok).count();
    if failed == 0 {
        out.push_str("all checks passed.\n");
    } else {
        let _ = writeln!(
            out,
            "{failed} of {} checks failed; fix the ✗ lines above.",
            checks.len()
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn render_marks_each_check_and_aligns_the_details() {
        let checks = [
            Check {
                name: "model",
                ok: true,
                detail: "models/x.bin (547 MB)".into(),
            },
            Check {
                name: "ocr",
                ok: false,
                detail: "no Windows OCR language installed — add one".into(),
            },
            Check {
                name: "research",
                ok: true,
                detail: "optional: not configured".into(),
            },
        ];
        let text = render(&checks);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[0], "  ✓ model       models/x.bin (547 MB)");
        assert_eq!(
            lines[1],
            "  ✗ ocr         no Windows OCR language installed — add one"
        );
        assert!(
            text.ends_with("1 of 3 checks failed; fix the ✗ lines above.\n"),
            "{text:?}"
        );
        assert!(!all_ok(&checks));
        assert!(all_ok(&checks[..1]));
    }
}
