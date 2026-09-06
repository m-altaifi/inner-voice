//! inner-voice — a realtime coach for live calls.
//!
//! Mic = YOU, system loopback = THEM. Two capture streams *are* the speaker
//! separation. Whisper transcribes each turn on the GPU, Claude reacts to
//! theirs, and the panel shows what to ask or say. You decide whether to.

mod agent;
mod audio;
mod coach;
mod history;
mod hud;
mod knowledge;
mod log;
mod process;
mod provider;
mod references;
mod roster;
mod search;
mod speak;
mod voiceid;

use anyhow::{Context, Result};
use clap::Parser;
use crossbeam_channel::{Receiver, Sender, unbounded};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use wasapi::{Device, DeviceEnumerator, Direction, initialize_mta};
use whisper_rs::{WhisperContext, WhisperContextParameters};

/// Everything the panel renders.
pub enum Msg {
    Turn(Who, String),
    Sys(String),
    AdviceStart(u64),
    Advice(u64, String),
    AdviceEnd(u64),
    Research,
    CancelResearch,
    Question(String),
    Pause(bool),
    ReferenceStatus(String),
    ToolStart(u64),
    ToolEnd(u64, std::result::Result<String, String>),
}

/// Who said it.
///
/// A nameless far-end voice is a resting state rather than a gap: `prompt.md`'s
/// rule is that a wrong name is worse than no name, so a voice nothing has
/// identified stays `THEM` indefinitely instead of being guessed at. Only `Them`
/// can carry a name — the microphone is the user by construction, which is the
/// whole basis of the YOU/THEM split.
#[derive(Clone, PartialEq)]
pub enum Who {
    You,
    /// `voice` is the far-end cluster this utterance was assigned to, `name` the
    /// roster name bound to that cluster. They are separate because they are
    /// learned separately and from different evidence: the audio says *which*
    /// voice, the transcript says what it is *called*. Either can be absent, and
    /// both absent is the ordinary resting state, not an error.
    Them {
        voice: Option<usize>,
        name: Option<String>,
    },
}

impl Who {
    /// What the panel and the model's transcript both call this speaker.
    pub fn label(&self) -> &str {
        match self {
            Who::You => "YOU",
            Who::Them {
                name: Some(name), ..
            } => name,
            Who::Them { .. } => "THEM",
        }
    }

    pub fn is_them(&self) -> bool {
        matches!(self, Who::Them { .. })
    }
}

#[derive(Parser)]
#[command(about = "Realtime call coach. Mic = YOU, system audio = THEM.")]
struct Args {
    /// Path to a whisper.cpp GGML model
    #[arg(
        long,
        env = "IV_WHISPER",
        default_value = "models/ggml-large-v3-turbo-q5_0.bin"
    )]
    whisper: String,

    /// Provider: anthropic, openai, deepseek, gemini, openrouter, or none
    /// (none = no coach, no key, no network; everything else still runs)
    #[arg(long, env = "IV_PROVIDER", default_value = "anthropic")]
    provider: String,

    /// Model id. Required for openai, gemini and openrouter.
    #[arg(long, env = "IV_MODEL")]
    model: Option<String>,

    /// Coaching persona
    #[arg(long, env = "IV_PROMPT", default_value = "prompt.md")]
    prompt: String,

    /// Folder of .md / .xlsx / .csv you teach it before the call
    #[arg(long, env = "IV_KNOWLEDGE", default_value = "knowledge")]
    knowledge: String,

    /// Capture a named input device instead of the default mic
    #[arg(long, env = "IV_MIC")]
    mic: Option<String>,

    /// Capture a named output device's loopback instead of the default speakers
    #[arg(long, env = "IV_LOOPBACK")]
    loopback: Option<String>,

    /// List audio devices and exit
    #[arg(long)]
    list_devices: bool,
    /// Offline UI preview: no audio capture, model loading, or online requests
    #[arg(long)]
    preview: bool,

    /// Override the auto-calibrated mic VAD threshold
    #[arg(long, env = "IV_MIC_GATE", value_parser = parse_gate)]
    mic_gate: Option<f32>,

    /// Override the auto-calibrated loopback VAD threshold
    #[arg(long, env = "IV_SYS_GATE", value_parser = parse_gate)]
    sys_gate: Option<f32>,

    /// Read advice aloud (deafens capture while it talks)
    #[arg(long, env = "IV_SPEAK")]
    speak: bool,

    /// Save every utterance here as a .wav next to the .txt whisper made of it
    #[arg(long, env = "IV_DUMP")]
    dump: Option<String>,

    /// Folder for the append-only JSONL transcript. Empty string turns it off.
    #[arg(long, env = "IV_LOG", default_value = "logs")]
    log: String,

    /// Speaker-embedding model. Missing file just means no far-end names.
    #[arg(
        long,
        env = "IV_VOICES",
        default_value = "models/campplus_sv_en_voxceleb_16k.onnx"
    )]
    voices: String,

    /// Ignore THEM turns shorter than this many words
    #[arg(long, env = "IV_MIN_WORDS", default_value_t = 3)]
    min_words: usize,

    /// Panel opacity, 0-255
    #[arg(long, env = "IV_ALPHA", default_value_t = 240)]
    alpha: u8,

    /// Enable F8 research using this Codex .exe (no shell command strings)
    #[arg(long, env = "IV_AGENT_CMD")]
    agent_cmd: Option<std::path::PathBuf>,

    /// Working directory for read-only research
    #[arg(long, env = "IV_AGENT_ROOT", default_value = "knowledge")]
    agent_root: std::path::PathBuf,

    /// Maximum research duration in seconds
    #[arg(long, env = "IV_AGENT_TIMEOUT", default_value_t = 90, value_parser = clap::value_parser!(u64).range(5..=300))]
    agent_timeout: u64,

    /// Optional literal filename search to include in F8 research
    #[arg(long, env = "IV_SEARCH_QUERY")]
    search_query: Option<String>,

    /// Search filenames through Everything, print up to 20 paths, and exit
    #[arg(long)]
    search_files: Option<String>,

    /// Everything command-line client executable
    #[arg(long, env = "IV_ES", default_value = "es.exe")]
    es: std::path::PathBuf,
}

fn parse_gate(value: &str) -> std::result::Result<f32, String> {
    let gate: f32 = value
        .parse()
        .map_err(|_| "gate must be a number".to_string())?;
    if !gate.is_finite() || gate <= 0.0 || gate >= 1.0 {
        return Err("gate must be finite and between 0 and 1 (exclusive)".into());
    }
    Ok(gate)
}

fn pick(enumerator: &DeviceEnumerator, dir: Direction, name: &Option<String>) -> Result<Device> {
    match name {
        Some(n) => enumerator
            .get_device_collection(&dir)?
            .get_device_with_name(n)
            .with_context(|| format!("no {dir} device matching {n:?}")),
        None => Ok(enumerator.get_default_device(&dir)?),
    }
}

fn list(enumerator: &DeviceEnumerator) -> Result<()> {
    for dir in [Direction::Capture, Direction::Render] {
        println!("\n{dir} devices:");
        let devices = enumerator.get_device_collection(&dir)?;
        for i in 0..devices.get_nbr_devices()? {
            println!("  {}", devices.get_device_at_index(i)?.get_friendlyname()?);
        }
    }
    println!("\n--mic takes a Capture name, --loopback takes a Render name.");
    Ok(())
}

/// Attach `name` to `voice`, unless doing so would claim one person is two.
///
/// If the name already sits on a different cluster, the audio and the transcript
/// disagree — one of them is wrong and there is no way to tell which. Dropping
/// both claims leaves two anonymous voices, which is recoverable; picking one
/// puts somebody else's words under a real person's name, which is not.
fn bind(names: &mut HashMap<usize, String>, voice: usize, name: &str) {
    if let Some((&other, _)) = names.iter().find(|(_, n)| n.as_str() == name)
        && other != voice
    {
        names.remove(&other);
        names.remove(&voice);
        return;
    }
    names.insert(voice, name.to_string());
}

/// Turns go to the panel either way; a finished THEM turn also asks Claude.
///
/// The log is written here rather than in the capture workers because this is
/// the one place every turn passes through exactly once, in order, on a single
/// thread — so the file's order is the call's order for free.
struct ContextServices {
    agent: Option<agent::Agent>,
    references: references::References,
}

fn route(
    rx: Receiver<Msg>,
    tx: Sender<Msg>,
    coach: Option<coach::Coach>,
    min_words: usize,
    log: Option<log::Log>,
    roster: roster::Roster,
    services: ContextServices,
) {
    let ContextServices { agent, references } = services;
    let mut history = history::History::default();
    let mut names: HashMap<usize, String> = HashMap::new();
    // A name spoken in the previous turn, waiting to see who answers to it.
    let mut called: Option<String> = None;
    let mut advice = (0, String::new());
    let mut paused = false;

    while let Ok(mut m) = rx.recv() {
        match &m {
            Msg::Pause(value) => {
                paused = *value;
                if paused && let Some(coach) = &coach {
                    coach.cancel();
                }
                continue;
            }
            Msg::Question(question) => {
                if let Some(coach) = &coach {
                    coach.ask(format!(
                        "{}\n\nUser question: {}{}",
                        history.render(),
                        serde_json::json!(question),
                        references.retrieve(question)
                    ));
                } else {
                    local_answer(&tx, &references, question);
                }
                continue;
            }
            Msg::Research => {
                if let Some(agent) = &agent {
                    let transcript = history.render();
                    agent.ask(format!("{transcript}{}", references.retrieve(&transcript)));
                } else {
                    let _ = tx.send(Msg::Sys(
                        "research off: set --agent-cmd to a Codex executable".into(),
                    ));
                }
                continue;
            }
            Msg::CancelResearch => {
                if let Some(agent) = &agent {
                    agent.cancel();
                }
                continue;
            }
            Msg::AdviceStart(id) => advice = (*id, String::new()),
            Msg::Advice(id, text) if *id == advice.0 => advice.1.push_str(text),
            Msg::AdviceEnd(id) if *id == advice.0 && !advice.1.is_empty() => {
                record(&log, &tx, "COACH", &advice.1);
                // A cancel can end a generation the worker also ended; clearing
                // here is what keeps the second one from logging it twice.
                advice.1.clear();
            }
            Msg::ToolEnd(id, result) => match result {
                // A completed job remains available to later coach turns. A
                // failure is worth logging and showing, but handing the coach
                // "Research failed: timed out" as untrusted reference material
                // costs it a real speech turn for the next 24 and teaches it
                // nothing, so only an answer goes into the window.
                Ok(text) => {
                    record(&log, &tx, &format!("RESEARCH {id}"), text);
                    history.push(history::Turn::Research {
                        id: *id,
                        text: text.clone(),
                    });
                }
                Err(e) => record(
                    &log,
                    &tx,
                    &format!("RESEARCH {id}"),
                    &format!("failed: {e}"),
                ),
            },
            _ => {}
        }
        if let Msg::Turn(who, text) = &mut m {
            if paused {
                continue;
            }
            if let Who::Them { voice, name } = who
                && let Some(v) = *voice
            {
                // A voice naming itself is the strongest evidence there is.
                // Someone answering to a name called a moment ago is weaker,
                // so it only fills a blank and never overrules an intro.
                if let Some(n) = roster.self_intro(text) {
                    bind(&mut names, v, n);
                } else if let Some(n) = called.as_deref()
                    && !names.contains_key(&v)
                {
                    bind(&mut names, v, n);
                }
                *name = names.get(&v).cloned();
            }
            // Either side can address someone, which is exactly why this lives
            // here: "Sarah, what do you think?" is usually said by YOU, on the
            // other capture thread entirely.
            called = roster.addressed(text).map(str::to_string);

            record(&log, &tx, who.label(), text);
            history.push(history::Turn::Speech {
                who: who.clone(),
                text: text.clone(),
            });
            if let Some(coach) = &coach
                && who.is_them()
                && text.split_whitespace().count() >= min_words
            {
                coach.ask(format!("{}{}", history.render(), references.retrieve(text)));
            }
        }
        if tx.send(m).is_err() {
            return;
        }
    }
}

fn record(log: &Option<log::Log>, tx: &Sender<Msg>, who: &str, text: &str) {
    if let Some(log) = log
        && let Err(e) = log.append(who, text)
    {
        let _ = tx.send(Msg::Sys(format!("logging failed: {e:#}")));
    }
}

fn local_answer(tx: &Sender<Msg>, references: &references::References, question: &str) {
    let _ = tx.send(Msg::AdviceStart(0));
    let _ = tx.send(Msg::Advice(0, references.local_answer(question)));
    let _ = tx.send(Msg::AdviceEnd(0));
}

fn main() -> Result<()> {
    // Before Args::parse, so clap's `env` fallbacks see it. Walks up from the
    // cwd, so running the exe from target/release still finds the repo's .env.
    let _ = dotenvy::dotenv();
    let args = Args::parse();
    if args.preview {
        let (tx, rx) = unbounded();
        let (commands, input) = unbounded();
        let references = references::References::new(tx.clone());
        let _ = tx.send(Msg::AdviceStart(1));
        let _ = tx.send(Msg::Advice(
            1,
            "ASK What would make you delay the launch?\nNOTE The rollback owner is still unclear."
                .into(),
        ));
        let _ = tx.send(Msg::AdviceEnd(1));
        let _ = tx.send(Msg::Turn(
            Who::Them {
                voice: None,
                name: None,
            },
            "We expect to launch next week.".into(),
        ));
        let local_references = references.clone();
        std::thread::spawn(move || {
            while let Ok(message) = input.recv() {
                if let Msg::Question(question) = message {
                    local_answer(&tx, &local_references, &question);
                } else if matches!(message, Msg::Research) {
                    let _ = tx.send(Msg::Sys("Preview mode: no audio capture or online requests. Dropped files can still be previewed locally.".into()));
                }
            }
        });
        return hud::run(
            rx,
            None,
            args.alpha,
            commands,
            hud::Session {
                research_enabled: false,
                online: false,
                references,
                epoch: Arc::new(std::sync::atomic::AtomicU64::new(0)),
                preview: true,
            },
        );
    }
    if let Some(query) = &args.search_files {
        let executable = args
            .es
            .canonicalize()
            .context("finding Everything es.exe; set --es")?;
        for path in search::files(&executable, query, &AtomicBool::new(false))? {
            println!("{}", path.display());
        }
        return Ok(());
    }
    initialize_mta().ok()?;
    let enumerator = DeviceEnumerator::new()?;

    if args.list_devices {
        return list(&enumerator);
    }

    // `none` runs everything except the coach: capture, VAD, whisper, naming,
    // log and panel all work, no key is needed and nothing leaves the machine.
    // Resolved before the 570 MB model loads either way, so a bad key still
    // fails in a second rather than a minute.
    let provider = match args.provider.as_str() {
        "none" => None,
        name => Some(provider::resolve(name, args.model.as_deref())?),
    };
    let mut prompt = if provider.is_some() {
        std::fs::read_to_string(&args.prompt).with_context(|| format!("reading {}", args.prompt))?
    } else {
        String::new()
    };
    let agent_config = args
        .agent_cmd
        .as_ref()
        .map(|path| -> Result<agent::Config> {
            anyhow::ensure!(
                provider.is_some(),
                "research requires a provider; --provider none guarantees no network"
            );
            let executable = path
                .canonicalize()
                .context("finding --agent-cmd executable")?;
            anyhow::ensure!(
                executable
                    .extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("exe")),
                "--agent-cmd must be a Codex .exe, not a shell script"
            );
            let root = args
                .agent_root
                .canonicalize()
                .context("finding --agent-root directory")?;
            anyhow::ensure!(root.is_dir(), "--agent-root must be a directory");
            Ok(agent::Config {
                executable,
                root,
                timeout: std::time::Duration::from_secs(args.agent_timeout),
                search: args.search_query.clone(),
                // Only resolved when a filename search is actually
                // configured; otherwise it is never read, and demanding an
                // indexer nobody asked for would fail the whole startup.
                // Silently keeping a relative path here used to surface as a
                // confusing error 90 s into a research job instead.
                es: match args.search_query {
                    Some(_) => args
                        .es
                        .canonicalize()
                        .context("finding Everything es.exe; set --es")?,
                    None => args.es.clone(),
                },
            })
        })
        .transpose()?;

    // Taught corpus is pinned into the system prompt, never retrieved.
    let corpus = knowledge::load(std::path::Path::new(&args.knowledge))?;
    let corpus_note = if corpus.is_empty() {
        "no knowledge/ corpus".to_string()
    } else {
        prompt.push_str(
            "\n\n# Source of truth\n\nThe following is what the user taught you \
             before this call. Treat it as authoritative and prefer it over your \
             own assumptions. If a fact is not here and not in the transcript, say \
             so instead of inventing one.\n",
        );
        prompt.push_str(&corpus);
        format!("knowledge: {} KB pinned", corpus.len() / 1024)
    };

    // Resolve to names here so a bad --mic fails before the model loads; the
    // capture threads reopen by name because COM handles are not `Send`.
    let mic_name = pick(&enumerator, Direction::Capture, &args.mic)?.get_friendlyname()?;
    let sys_name = pick(&enumerator, Direction::Render, &args.loopback)?.get_friendlyname()?;

    // Flash attention nearly halves the encoder on this GPU (80 ms vs 144 ms for
    // a 4 s turn) for one flag. It changes the numerics, so `gpu_transcribes`
    // asserts on real words with it on.
    let mut wparams = WhisperContextParameters::default();
    wparams.flash_attn(true);
    let ctx = Arc::new(
        WhisperContext::new_with_params(&args.whisper, wparams)
            .with_context(|| format!("loading {} (see README for the download)", args.whisper))?,
    );

    // The corpus feeds two different models: Claude gets all of it as source of
    // truth, whisper gets only the proper nouns, because its prompt window is
    // ~224 tokens and names are the part it actually mishears.
    // A missing embedding model is not an error: the call runs, the far end is
    // just never named. Checked here so the panel can say so at startup rather
    // than staying mysteriously anonymous all call.
    let voices = Some(std::path::PathBuf::from(&args.voices)).filter(|p| p.exists());
    let roster = roster::Roster::load(std::path::Path::new(&args.knowledge));
    let names_note = match (&voices, roster.is_empty()) {
        (Some(_), false) => "naming: on".to_string(),
        (Some(_), true) => "naming: no attendees.csv, far end stays THEM".to_string(),
        (None, _) => format!("naming: no {}, far end stays THEM", args.voices),
    };

    let tune = Arc::new(audio::Tune {
        prompt: knowledge::glossary(&corpus),
        dump: args.dump.map(std::path::PathBuf::from),
        voices,
        epoch: Arc::new(std::sync::atomic::AtomicU64::new(0)),
    });

    let (turn_tx, turn_rx) = unbounded();
    let (ui_tx, ui_rx) = unbounded();
    let references = references::References::new(ui_tx.clone());
    let mute = Arc::new(AtomicBool::new(false));
    let speaker = args.speak.then(|| speak::Speaker::new(mute.clone()));

    // Empty rather than Option so `IV_LOG=` in .env switches it off without a
    // second flag to keep in sync.
    let log = match args.log.is_empty() {
        true => None,
        false => Some(log::Log::new(std::path::Path::new(&args.log))?),
    };
    let log_note = match &log {
        Some(l) => format!("logging to {}", l.path().display()),
        None => "not logging".into(),
    };

    // Worth saying out loud: a silent advice pane looks identical whether the
    // coach is switched off or merely broken.
    let coach_note = match &provider {
        Some(p) => format!("coach: {} {}", args.provider, p.model),
        None => "coach: off, transcript only".to_string(),
    };
    let research_enabled = agent_config.is_some();
    let online = provider.is_some();
    let agent = agent_config.map(|config| agent::Agent::new(config, turn_tx.clone()));
    let coach = provider.map(|p| coach::Coach::new(p, prompt, turn_tx.clone()));
    {
        let (rx, tx) = (turn_rx, ui_tx.clone());
        let references = references.clone();
        std::thread::spawn(move || {
            route(
                rx,
                tx,
                coach,
                args.min_words,
                log,
                roster,
                ContextServices { agent, references },
            )
        });
    }

    // YOU hears itself; THEM is muted while the AI speaks, so its own voice
    // never comes back around as the other party.
    for (who, dir, name, gate, mute) in [
        (
            Who::You,
            Direction::Capture,
            mic_name.clone(),
            args.mic_gate,
            None,
        ),
        (
            Who::Them {
                voice: None,
                name: None,
            },
            Direction::Render,
            sys_name.clone(),
            args.sys_gate,
            Some(mute),
        ),
    ] {
        let (ctx, tx, tune) = (ctx.clone(), turn_tx.clone(), tune.clone());
        std::thread::spawn(move || {
            let label = who.label().to_string(); // `who` is moved into run
            let input = audio::Input {
                who,
                dir,
                name,
                gate_override: gate,
                mute,
                hear: None,
            };
            if let Err(e) = audio::run(input, ctx, tx.clone(), tune) {
                let _ = tx.send(Msg::Sys(format!("{label} stopped: {e}")));
            }
        });
    }

    let _ = ui_tx.send(Msg::Sys(corpus_note));
    if !tune.prompt.is_empty() {
        // Worth surfacing: a glossary that came out empty looks identical to one
        // that worked, right up until a name comes back wrong.
        let _ = ui_tx.send(Msg::Sys(format!(
            "primed {} terms: {}",
            tune.prompt.matches(", ").count() + 1,
            tune.prompt.trim_start_matches("Glossary: ")
        )));
    }
    if let Some(d) = &tune.dump {
        let _ = ui_tx.send(Msg::Sys(format!("dumping utterances to {}", d.display())));
    }
    let _ = ui_tx.send(Msg::Sys(log_note));
    let _ = ui_tx.send(Msg::Sys(names_note));
    let _ = ui_tx.send(Msg::Sys(coach_note));
    let _ = ui_tx.send(Msg::Sys(format!("YOU <- {mic_name}")));
    let _ = ui_tx.send(Msg::Sys(format!("THEM <- {sys_name} (loopback)")));
    hud::run(
        ui_rx,
        speaker,
        args.alpha,
        turn_tx,
        hud::Session {
            research_enabled,
            online,
            references,
            epoch: tune.epoch.clone(),
            preview: false,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invalid_gates_fail_before_loading_models() {
        for value in ["NaN", "inf", "-1", "0", "1", "abc"] {
            assert!(parse_gate(value).is_err(), "{value}");
        }
        assert_eq!(parse_gate("0.02").unwrap(), 0.02);
    }
    #[test]
    fn conflicting_name_claims_clear_both_voices() {
        let mut names = HashMap::from([(1, "Sarah".into()), (2, "Marcus".into())]);
        bind(&mut names, 2, "Sarah");
        assert!(names.is_empty());
    }
}
