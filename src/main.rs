//! inner-voice — a realtime coach for live calls.
//!
//! Mic = YOU, system loopback = THEM. Two capture streams *are* the speaker
//! separation. Whisper transcribes each turn on the GPU, Claude reacts to
//! theirs, and the panel shows what to ask or say. You decide whether to.

mod agent;
mod audio;
mod coach;
mod extract;
mod history;
mod hud;
mod knowledge;
mod log;
mod process;
mod provider;
mod references;
mod roster;
mod search;
mod setup;
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

    /// Research persona (F8). Falls back to a built-in prompt if missing.
    #[arg(long, env = "IV_RESEARCH_PROMPT", default_value = "research.md")]
    research_prompt: String,

    /// Folder of .md / .xlsx / .csv you teach it before the call
    #[arg(long, env = "IV_KNOWLEDGE", default_value = "knowledge")]
    knowledge: String,

    /// Folder of reference files that persist across sessions and reload when edited
    #[arg(long, env = "IV_REFERENCES", default_value = "references")]
    references: String,

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

    /// Read advice aloud. With --hear the voice is never captured; without it
    /// the call is deafened while the voice talks
    // A bool flag's default parser is the strict one — `true`/`false` only — and
    // it is applied to the env value too, so `IV_SPEAK=1` in a .env aborted
    // startup with an argument error. Boolish is clap's documented pairing with
    // SetTrue and takes 1/0/yes/no/on/off as well.
    #[arg(long, env = "IV_SPEAK", value_parser = clap::builder::BoolishValueParser::new())]
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

    /// Route F8 research through Claude Code's claude.exe instead of the provider
    #[arg(long, env = "IV_AGENT_CMD")]
    agent_cmd: Option<std::path::PathBuf>,

    /// Working directory for read-only research; knowledge/ and references/ live under it
    #[arg(long, env = "IV_AGENT_ROOT", default_value = ".")]
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

    /// Hear only these apps as THEM: a comma list, each any part of a name
    /// (see --list-apps). `/hear` in the question box changes it mid-call.
    /// Without it, THEM is everything the speakers play.
    #[arg(long, env = "IV_HEAR")]
    hear: Option<String>,

    /// List the apps with audio on the loopback device and exit
    #[arg(long)]
    list_apps: bool,

    /// Check every prerequisite — model, CUDA, devices, provider (one tiny
    /// request, timed), OCR, folders — print ✓/✗ with the fix, and exit
    #[arg(long)]
    setup: bool,
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

pub(crate) fn pick(
    enumerator: &DeviceEnumerator,
    dir: Direction,
    name: &Option<String>,
) -> Result<Device> {
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

/// Persona plus the taught corpus, in the shape the coach pins behind its
/// cache breakpoint. One place, because the same text is built at startup and
/// again on every reload.
fn build_prompt(persona: &str, corpus: &str) -> String {
    if corpus.is_empty() {
        return persona.to_string();
    }
    format!(
        "{persona}\n\n# Source of truth\n\nThe following is what the user taught you \
         before this call. Treat it as authoritative and prefer it over your own \
         assumptions. If a fact is not here and not in the transcript, say so instead \
         of inventing one.\n{corpus}"
    )
}

/// Turns go to the panel either way; a finished THEM turn also asks Claude.
///
/// The log is written here rather than in the capture workers because this is
/// the one place every turn passes through exactly once, in order, on a single
/// thread — so the file's order is the call's order for free.
struct ContextServices {
    agent: Option<agent::Agent>,
    references: references::References,
    research_prompt: String,
    corpus: knowledge::Corpus,
    persona: String,
    tune: Arc<audio::Tune>,
}

/// Before anything goes to the coach: if the folder changed, the coach and
/// whisper both learn it now, and the panel says so.
///
/// Runs on the router thread, which initialises no apartment of its own and
/// does not need one: `main` put the process in an MTA, and windows-core's
/// factory cache joins it (`CoIncrementMTAUsage` on `CO_E_NOTINITIALIZED`), so
/// an image in `knowledge/` reaches `extract::ocr` from here.
fn refresh_corpus(
    corpus: &mut knowledge::Corpus,
    roster: &mut roster::Roster,
    persona: &str,
    coach: &Option<coach::Coach>,
    tune: &audio::Tune,
    tx: &Sender<Msg>,
) {
    match corpus.refresh() {
        Ok(true) => {
            // `attendees.csv` is in this same folder, and on-disk is the source
            // of truth (Decision 9) — so a name added mid-call binds from the
            // next turn, rather than waiting for a restart.
            *roster = roster::Roster::load(corpus.dir());
            if let Some(coach) = coach {
                coach.set_prompt(build_prompt(persona, &corpus.text));
            }
            *tune.prompt.write().unwrap_or_else(|e| e.into_inner()) =
                knowledge::glossary(&corpus.text);
            let _ = tx.send(Msg::Sys(format!(
                "knowledge reloaded: {} files, {} KB",
                corpus.files(),
                corpus.text.len() / 1024
            )));
        }
        Ok(false) => {}
        Err(e) => {
            let _ = tx.send(Msg::Sys(format!("knowledge reload failed: {e:#}")));
        }
    }
}

fn route(
    rx: Receiver<Msg>,
    tx: Sender<Msg>,
    coach: Option<coach::Coach>,
    min_words: usize,
    log: Option<log::Log>,
    mut roster: roster::Roster,
    services: ContextServices,
) {
    let ContextServices {
        agent,
        references,
        research_prompt,
        mut corpus,
        persona,
        tune,
    } = services;
    let mut history = history::History::default();
    let mut names: HashMap<usize, String> = HashMap::new();
    // A name spoken in the previous turn, waiting to see who answers to it.
    let mut called: Option<String> = None;
    let mut last_them = String::new();
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
                // The panel has no controls by design, so F7's box is the one
                // place a name can be typed — which makes it the app selector
                // too. A command here costs no hotkey and no new surface, and
                // unlike a list of what is playing it can register an app that
                // has not started yet.
                if let Some(spec) = question.strip_prefix("/hear") {
                    set_hearing(&tune, &tx, spec.trim());
                    continue;
                }
                refresh_corpus(&mut corpus, &mut roster, &persona, &coach, &tune, &tx);
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
            // The CLI wins where it is configured, so exactly one lane is live
            // for the whole session and the two id spaces never interleave on
            // the panel's single research slot.
            Msg::Research => {
                refresh_corpus(&mut corpus, &mut roster, &persona, &coach, &tune, &tx);
                let transcript = history.render();
                if let Some(agent) = &agent {
                    agent.ask(format!("{transcript}{}", references.retrieve(&transcript)));
                } else if let Some(coach) = &coach {
                    // Research the newest THEM line — that is the question the
                    // user pressed F8 about — with the wide passage window.
                    let query = if last_them.is_empty() {
                        &transcript
                    } else {
                        &last_them
                    };
                    coach.research(
                        research_prompt.clone(),
                        format!("{transcript}{}", references.retrieve_deep(query)),
                    );
                } else {
                    let _ = tx.send(Msg::Sys(
                        "research off: needs a provider, or --agent-cmd".into(),
                    ));
                }
                continue;
            }
            Msg::CancelResearch => {
                if let Some(agent) = &agent {
                    agent.cancel();
                } else if let Some(coach) = &coach {
                    coach.cancel_research();
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
            if who.is_them() {
                last_them = text.clone();
            }

            record(&log, &tx, who.label(), text);
            history.push(history::Turn::Speech {
                who: who.clone(),
                text: text.clone(),
            });
            refresh_corpus(&mut corpus, &mut roster, &persona, &coach, &tune, &tx);
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

/// `/hear` — report, or rewrite, which apps are `THEM`.
///
/// Bare `/hear` reports rather than clears: clearing is the destructive read,
/// and a user checking what is selected should not have to risk it. `off`,
/// `mix` and `all` are the ways back to the whole speaker mix.
fn set_hearing(tune: &Arc<audio::Tune>, tx: &Sender<Msg>, spec: &str) {
    fn describe(apps: &[String]) -> String {
        if apps.is_empty() {
            "the whole speaker mix".to_string()
        } else {
            apps.join(" + ")
        }
    }
    if spec.is_empty() {
        let apps = tune.hear.read().unwrap_or_else(|e| e.into_inner());
        let _ = tx.send(Msg::Sys(format!(
            "hearing: {} — /hear <app>[,<app>] to change, /hear off for the mix",
            describe(&apps)
        )));
        return;
    }
    let wanted = match spec {
        "off" | "mix" | "all" => Vec::new(),
        spec => audio::parse_hear(spec),
    };
    *tune.hear.write().unwrap_or_else(|e| e.into_inner()) = wanted.clone();
    // After the write, never before: a stream that sees the new generation must
    // find the new selection already there, or it re-serves the old one.
    tune.hear_gen
        .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let _ = tx.send(Msg::Sys(format!(
        "hearing: switching to {} (up to 2 s)",
        describe(&wanted)
    )));
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
        let references = references::References::new(tx.clone(), None);
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
                model: None,
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
    if args.list_apps {
        let dev = pick(&enumerator, Direction::Render, &args.loopback)?;
        println!("\nApps with audio on {}:", dev.get_friendlyname()?);
        for s in audio::sessions(&dev)? {
            println!(
                "  {:>6}  {:<7}  {}",
                s.pid,
                if s.active { "active" } else { "silent" },
                s.name
            );
        }
        println!("\n--hear takes any part of a name; an active one wins.");
        return Ok(());
    }

    // Stays here, below `initialize_mta()`: `extract::ocr_available()` activates
    // WinRT and would report "no OCR" on a machine that has one if COM were not
    // already up on this thread.
    if args.setup {
        // The provider line inherits the pooled agent's 60 s timeout, so a
        // provider that connects and then stalls otherwise looks like a hang.
        println!(
            "\ninner-voice --setup — checking; the provider line waits for one reply (up to 60 s)…"
        );
        let checks = setup::run(
            &setup::Inputs {
                whisper: args.whisper.clone(),
                provider: args.provider.clone(),
                model: args.model.clone(),
                mic: args.mic.clone(),
                loopback: args.loopback.clone(),
                knowledge: args.knowledge.clone(),
                references: args.references.clone(),
                agent_cmd: args.agent_cmd.as_ref().map(|p| p.display().to_string()),
            },
            &enumerator,
        );
        print!("\n{}", setup::render(&checks));
        std::process::exit(if setup::all_ok(&checks) { 0 } else { 1 });
    }

    // `none` runs everything except the coach: capture, VAD, whisper, naming,
    // log and panel all work, no key is needed and nothing leaves the machine.
    // Resolved before the 570 MB model loads either way, so a bad key still
    // fails in a second rather than a minute.
    let provider = match args.provider.as_str() {
        "none" => None,
        name => Some(provider::resolve(name, args.model.as_deref())?),
    };
    let prompt = if provider.is_some() {
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
            agent::validate_executable(&executable)?;
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

    // Taught corpus is pinned into the system prompt, never retrieved — and
    // re-read on change, so on-disk stays the source of truth mid-call.
    let corpus = knowledge::Corpus::load(std::path::Path::new(&args.knowledge))?;
    let corpus_note = if corpus.text.is_empty() {
        "no knowledge/ corpus".to_string()
    } else {
        format!("knowledge: {} KB pinned", corpus.text.len() / 1024)
    };
    let persona = prompt;
    let prompt = build_prompt(&persona, &corpus.text);

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
        prompt: std::sync::RwLock::new(knowledge::glossary(&corpus.text)),
        dump: args.dump.map(std::path::PathBuf::from),
        voices,
        epoch: Arc::new(std::sync::atomic::AtomicU64::new(0)),
        hear: std::sync::RwLock::new(
            args.hear
                .as_deref()
                .map(audio::parse_hear)
                .unwrap_or_default(),
        ),
        hear_gen: std::sync::atomic::AtomicU64::new(0),
    });

    let (turn_tx, turn_rx) = unbounded();
    let (ui_tx, ui_rx) = unbounded();
    let references = references::References::new(
        ui_tx.clone(),
        Some(std::path::PathBuf::from(&args.references)),
    );
    references.import_folder();
    // The voice is deafened out of *endpoint* capture only, so the mute follows
    // --speak and not --hear: a process-loopback stream opts out itself (see
    // `app_feed` in audio.rs), and --hear still falls back to the endpoint mix
    // when the app cannot be opened — which without this would capture the
    // advice back as a THEM turn and coach on it.
    let mute = args.speak.then(|| Arc::new(AtomicBool::new(false)));
    let speaker = mute.as_ref().map(|m| speak::Speaker::new(m.clone()));

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
    // F8 works with a CLI *or* a provider now; only `--provider none` with no
    // CLI leaves it off, and then the keys are not registered at all.
    let research_enabled = agent_config.is_some() || provider.is_some();
    let research_prompt = std::fs::read_to_string(&args.research_prompt)
        .unwrap_or_else(|_| coach::RESEARCH_PROMPT.to_string());
    let online = provider.is_some();
    let agent = agent_config.map(|config| agent::Agent::new(config, turn_tx.clone()));
    let model = provider.as_ref().map(|p| p.model.clone());
    let coach = provider.map(|p| coach::Coach::new(p, prompt, turn_tx.clone()));
    {
        let (rx, tx) = (turn_rx, ui_tx.clone());
        let references = references.clone();
        let tune = tune.clone();
        std::thread::spawn(move || {
            route(
                rx,
                tx,
                coach,
                args.min_words,
                log,
                roster,
                ContextServices {
                    agent,
                    references,
                    research_prompt,
                    corpus,
                    persona,
                    tune,
                },
            )
        });
    }

    // YOU hears itself. THEM is the endpoint mix, or one app's process tree
    // with --hear; only the endpoint mix is deafened while the voice talks.
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
            mute.clone(),
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
            };
            if let Err(e) = audio::run(input, ctx, tx.clone(), tune) {
                let _ = tx.send(Msg::Sys(format!("{label} stopped: {e}")));
            }
        });
    }

    let _ = ui_tx.send(Msg::Sys(corpus_note));
    let glossary = tune
        .prompt
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    if !glossary.is_empty() {
        // Worth surfacing: a glossary that came out empty looks identical to one
        // that worked, right up until a name comes back wrong.
        let _ = ui_tx.send(Msg::Sys(format!(
            "primed {} terms: {}",
            glossary.matches(", ").count() + 1,
            glossary.trim_start_matches("Glossary: ")
        )));
    }
    if let Some(d) = &tune.dump {
        let _ = ui_tx.send(Msg::Sys(format!("dumping utterances to {}", d.display())));
    }
    let _ = ui_tx.send(Msg::Sys(log_note));
    let _ = ui_tx.send(Msg::Sys(names_note));
    let _ = ui_tx.send(Msg::Sys(coach_note));
    let _ = ui_tx.send(Msg::Sys(format!("YOU <- {mic_name}")));
    let _ = ui_tx.send(Msg::Sys(match &args.hear {
        Some(apps) => format!(
            "THEM <- {} (app loopback on {sys_name})",
            audio::parse_hear(apps).join(" + ")
        ),
        None => format!("THEM <- {sys_name} (loopback)"),
    }));
    if args.speak && args.hear.is_none() {
        let _ = ui_tx.send(Msg::Sys(
            "speak: reading advice mutes the call; add --hear <app> so it doesn't".into(),
        ));
    }
    hud::run(
        ui_rx,
        speaker,
        args.alpha,
        turn_tx,
        hud::Session {
            research_enabled,
            online,
            model,
            references,
            epoch: tune.epoch.clone(),
            preview: false,
        },
    )
}

#[cfg(test)]
mod tests {

    fn hearing_tune(apps: &[&str]) -> Arc<audio::Tune> {
        Arc::new(audio::Tune {
            prompt: std::sync::RwLock::new(String::new()),
            dump: None,
            voices: None,
            epoch: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            hear: std::sync::RwLock::new(apps.iter().map(|s| s.to_string()).collect()),
            hear_gen: std::sync::atomic::AtomicU64::new(0),
        })
    }
    fn hear(tune: &Arc<audio::Tune>, spec: &str) -> (Vec<String>, u64, String) {
        let (tx, rx) = crossbeam_channel::unbounded();
        set_hearing(tune, &tx, spec);
        let note = match rx.try_recv() {
            Ok(Msg::Sys(s)) => s,
            _ => panic!("expected exactly one Sys note"),
        };
        (
            tune.hear.read().unwrap().clone(),
            tune.hear_gen.load(std::sync::atomic::Ordering::SeqCst),
            note,
        )
    }

    /// Bare `/hear` is the one a user types to *check*. If it cleared the
    /// selection it would silently drop the far end mid-call.
    #[test]
    fn bare_hear_reports_and_changes_nothing() {
        let tune = hearing_tune(&["chrome"]);
        let (apps, generation, note) = hear(&tune, "");
        assert_eq!(apps, ["chrome"]);
        assert_eq!(generation, 0, "reporting must not retire the live streams");
        assert!(note.starts_with("hearing: chrome"), "{note}");
    }

    #[test]
    fn hear_sets_a_list_and_off_returns_to_the_mix() {
        let tune = hearing_tune(&[]);
        let (apps, generation, note) = hear(&tune, "chrome, zoom");
        assert_eq!(apps, ["chrome", "zoom"]);
        assert_eq!(generation, 1);
        assert!(note.contains("chrome + zoom"), "{note}");

        for (spec, pass) in [("off", 2), ("mix", 3), ("all", 4)] {
            *tune.hear.write().unwrap() = vec!["chrome".into()];
            let (apps, generation, note) = hear(&tune, spec);
            assert!(apps.is_empty(), "{spec} should clear the selection");
            assert_eq!(generation, pass, "every change retires the live streams");
            assert!(note.contains("the whole speaker mix"), "{note}");
        }
    }
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
