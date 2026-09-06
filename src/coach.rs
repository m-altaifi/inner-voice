//! The fast lane, streamed over SSE. A newer turn cancels the request in flight.
//!
//! Two wire formats, selected in `provider.rs`. Everything else here is shared.

use crate::Msg;
use crate::provider::{Provider, Wire};
use anyhow::{Result, bail};
use crossbeam_channel::{Receiver, Sender, bounded};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Read};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

const BETAS: &str = "fast-mode-2026-02-01,server-side-fallback-2026-07-01";
const MAX_TOKENS: u32 = 400;

/// Every turn goes to the same host, so the connection should be opened once.
///
/// `ureq::post` is documented as running on a *use-once* agent: it paid a fresh
/// DNS lookup, TCP connect and TLS handshake before the model saw a single byte,
/// on every turn, inside a ~1.2 s budget. One pooled agent amortises that over
/// the call. The pool's 15 s default idle age is shorter than a thoughtful pause
/// in a real conversation, which is exactly when the panel must not stall, so it
/// is raised past any gap that is still the same call.
fn pooled_agent() -> ureq::Agent {
    ureq::config::Config::builder()
        .max_idle_age(Duration::from_secs(600))
        .timeout_global(Some(Duration::from_secs(60)))
        // Bounds the drain in `stream`: a provider that sent its terminator and
        // then held the socket open would otherwise hang the coach thread. Well
        // past any real MAX_TOKENS stream, short enough to not outlive a call.
        .timeout_recv_body(Some(Duration::from_secs(60)))
        .build()
        .new_agent()
}

pub struct Coach {
    tx: Sender<Msg>,
    seq: Arc<AtomicU64>,
    jobs: Sender<(u64, String)>,
    pending: Receiver<(u64, String)>,
}

impl Coach {
    /// Retire the live generation.
    ///
    /// Unlike `ask`, which supersedes by announcing a *newer* `AdviceStart`,
    /// a cancel has no successor — so the generation it retires has to be
    /// closed out here or the panel sits on "preparing advice" for a request
    /// nothing will ever finish. `fetch_add` returns the id being retired, and
    /// this lands behind any `AdviceStart` already queued ahead of it, which is
    /// what makes the ordering come out right. Consumers ignore a repeat.
    pub fn cancel(&self) {
        let retired = self.seq.fetch_add(1, Ordering::SeqCst);
        let _ = self.pending.try_recv();
        let _ = self.tx.send(Msg::AdviceEnd(retired));
    }
    pub fn new(provider: Provider, prompt: String, tx: Sender<Msg>) -> Self {
        let (jobs, pending) = bounded::<(u64, String)>(1);
        let seq = Arc::new(AtomicU64::new(0));
        let (rx, live, output) = (pending.clone(), seq.clone(), tx.clone());
        std::thread::spawn(move || {
            let agent = pooled_agent();
            while let Ok((generation, transcript)) = rx.recv() {
                if live.load(Ordering::SeqCst) != generation {
                    continue;
                }
                if let Err(e) = stream(
                    &agent,
                    &provider,
                    &prompt,
                    &transcript,
                    generation,
                    &live,
                    &output,
                ) && live.load(Ordering::SeqCst) == generation
                {
                    let _ = output.send(Msg::Sys(format!("coach: {e}")));
                    let _ = output.send(Msg::AdviceEnd(generation));
                }
            }
        });
        Self {
            tx,
            seq,
            jobs,
            pending,
        }
    }

    /// Fire and forget. The HUD renders whatever streams back, and drops
    /// anything tagged with a superseded generation.
    pub fn ask(&self, transcript: String) {
        let seq = self.seq.fetch_add(1, Ordering::SeqCst) + 1;
        // Announced before the request goes out, not when the first byte lands,
        // so the panel can show the whole ~1-2 s wait instead of sitting mute.
        let _ = self.tx.send(Msg::AdviceStart(seq));
        // Keep one active request and only the newest pending turn. A slow or
        // unreachable provider must not create a thread per utterance.
        let _ = self.pending.try_recv();
        let _ = self.jobs.try_send((seq, transcript));
    }
}

impl Drop for Coach {
    fn drop(&mut self) {
        self.seq.fetch_add(1, Ordering::SeqCst);
    }
}

/// Request body per wire. Anthropic gets the latency knobs; the OpenAI shape is
/// deliberately plain, because every vendor speaking it supports a slightly
/// different set of extras and none are worth the breakage.
fn body(p: &Provider, prompt: &str, transcript: &str) -> Value {
    match p.wire {
        // Two system blocks: persona plus taught corpus are identical every turn,
        // so they sit behind a cache breakpoint and are not resent. The
        // transcript changes every turn and stays in `messages` after it.
        Wire::Anthropic => json!({
            "model": p.model,
            "max_tokens": MAX_TOKENS,
            "stream": true,
            "speed": "fast",
            "fallbacks": "default",
            "output_config": {"effort": "low"},
            "system": [{"type": "text", "text": prompt,
                        "cache_control": {"type": "ephemeral"}}],
            "messages": [{"role": "user", "content": transcript}],
        }),
        Wire::OpenAi => json!({
            "model": p.model,
            "max_tokens": MAX_TOKENS,
            "stream": true,
            "messages": [
                {"role": "system", "content": prompt},
                {"role": "user", "content": transcript},
            ],
        }),
    }
}

/// Pull the text delta out of one SSE event. `None` means "nothing to render".
fn delta(wire: Wire, v: &Value) -> Option<String> {
    match wire {
        Wire::Anthropic => match v["type"].as_str()? {
            "content_block_delta" => v["delta"]["text"].as_str().map(str::to_string),
            "message_delta" if v["delta"]["stop_reason"] == "refusal" => {
                Some("\n[declined]".into())
            }
            _ => None,
        },
        Wire::OpenAi => v["choices"][0]["delta"]["content"]
            .as_str()
            .map(str::to_string),
    }
}

fn stream(
    agent: &ureq::Agent,
    p: &Provider,
    prompt: &str,
    transcript: &str,
    seq: u64,
    live: &AtomicU64,
    tx: &Sender<Msg>,
) -> Result<()> {
    let req = agent.post(p.url).header("content-type", "application/json");
    let req = match p.wire {
        Wire::Anthropic => req
            .header("x-api-key", &p.key)
            .header("anthropic-version", "2023-06-01")
            .header("anthropic-beta", BETAS),
        Wire::OpenAi => req.header("authorization", &format!("Bearer {}", p.key)),
    };
    let resp = req.send_json(body(p, prompt, transcript))?;
    let reader = BufReader::new(resp.into_body().into_reader());
    consume(reader, p.wire, seq, live, tx)
}

fn consume(
    mut reader: impl BufRead,
    wire: Wire,
    seq: u64,
    live: &AtomicU64,
    tx: &Sender<Msg>,
) -> Result<()> {
    // `done` rather than `break`: ureq only returns a connection to the pool
    // when the body is read to its end, and re-probes the socket to be sure. So
    // breaking on the terminator left the trailing bytes unread and silently
    // threw the connection away — every turn re-handshaked TLS and the pooled
    // agent above did nothing. The tail after a terminator is a couple of bytes.
    let mut done = false;
    let mut event = String::new();
    let mut output_bytes = 0;
    loop {
        if live.load(Ordering::SeqCst) != seq {
            // They said something newer. Abandon the socket rather than draining
            // a stream that is still generating — this one is worth closing.
            return Ok(());
        }
        let mut line = String::new();
        let bytes = reader.by_ref().take(65_537).read_line(&mut line)?;
        if bytes == 0 {
            break;
        }
        if bytes > 65_536 {
            bail!("provider SSE line exceeds 64 KB");
        }
        if done {
            continue;
        }
        let line = line.trim_end_matches(['\r', '\n']);
        if let Some(data) = line.strip_prefix("data:") {
            if !event.is_empty() {
                event.push('\n');
            }
            event.push_str(data.strip_prefix(' ').unwrap_or(data));
            if event.len() > 65_536 {
                bail!("provider SSE event exceeds 64 KB");
            }
            continue;
        }
        if !line.is_empty() || event.is_empty() {
            continue;
        }
        let data = std::mem::take(&mut event);
        if data.trim() == "[DONE]" {
            done = true; // OpenAI wire terminator
            continue;
        }
        let v: Value = serde_json::from_str(&data)?;
        if let Some(err) = v.get("error").and_then(|e| e["message"].as_str()) {
            bail!("{err}");
        }
        if let Some(text) = delta(wire, &v) {
            output_bytes += text.len();
            if output_bytes > 65_536 {
                bail!("provider response exceeds 64 KB");
            }
            tx.send(Msg::Advice(seq, text))?;
        }
        if v["type"] == "message_stop" {
            done = true; // Anthropic wire terminator
        }
    }
    if !done {
        bail!("provider stream ended before its completion marker");
    }
    if output_bytes == 0 {
        bail!("provider returned no advice");
    }
    tx.send(Msg::AdviceEnd(seq))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read_sse(data: &str) -> (Result<()>, Vec<Msg>) {
        let (tx, rx) = crossbeam_channel::unbounded();
        let result = consume(
            std::io::Cursor::new(data),
            Wire::OpenAi,
            1,
            &AtomicU64::new(1),
            &tx,
        );
        (result, rx.try_iter().collect())
    }

    #[test]
    fn sse_accepts_optional_spaces_crlf_and_multiline_events() {
        let (result, messages) = read_sse(
            ": ping\r\ndata:{\"choices\":\r\ndata: [{\"delta\":{\"content\":\"hello\"}}]}\r\n\r\ndata: [DONE]\r\n\r\n",
        );
        result.unwrap();
        assert!(
            messages
                .iter()
                .any(|m| matches!(m, Msg::Advice(1, s) if s == "hello"))
        );
        assert!(matches!(messages.last(), Some(Msg::AdviceEnd(1))));
    }

    #[test]
    fn truncated_empty_error_and_oversized_streams_fail() {
        assert!(
            read_sse("data: {\"choices\":[{\"delta\":{\"content\":\"partial\"}}]}\n\n")
                .0
                .is_err()
        );
        assert!(read_sse("data: [DONE]\n\n").0.is_err());
        assert!(
            read_sse("data: {\"error\":{\"message\":\"quota\"}}\n\n")
                .0
                .unwrap_err()
                .to_string()
                .contains("quota")
        );
        assert!(
            read_sse(&format!("data: {}\n\n", "x".repeat(70_000)))
                .0
                .is_err()
        );
    }

    #[test]
    fn cancelling_closes_the_generation_it_retired() {
        let (tx, rx) = crossbeam_channel::unbounded();
        let coach = Coach::new(
            Provider {
                // Refused immediately; the worker's fate is not what is under
                // test, only that the panel is told the turn it saw is over.
                url: "http://127.0.0.1:1",
                model: "test".into(),
                key: "test".into(),
                wire: Wire::OpenAi,
            },
            String::new(),
            tx,
        );
        coach.ask("a turn".into());
        assert!(matches!(rx.recv().unwrap(), Msg::AdviceStart(1)));
        coach.cancel();
        assert!(
            rx.try_iter().any(|m| matches!(m, Msg::AdviceEnd(1))),
            "a cancelled generation must still be closed, or the panel waits forever"
        );
    }

    #[test]
    fn superseded_stream_does_not_emit_advice() {
        let (tx, rx) = crossbeam_channel::unbounded();
        consume(
            std::io::Cursor::new("data: [DONE]\n\n"),
            Wire::OpenAi,
            1,
            &AtomicU64::new(2),
            &tx,
        )
        .unwrap();
        assert!(rx.is_empty());
    }

    fn p(wire: Wire) -> Provider {
        Provider {
            url: "http://x",
            model: "m".into(),
            key: "k".into(),
            wire,
        }
    }

    #[test]
    fn each_wire_parses_its_own_delta_shape() {
        let a: Value =
            serde_json::from_str(r#"{"type":"content_block_delta","delta":{"text":"hi"}}"#)
                .unwrap();
        let o: Value = serde_json::from_str(r#"{"choices":[{"delta":{"content":"hi"}}]}"#).unwrap();

        assert_eq!(delta(Wire::Anthropic, &a).as_deref(), Some("hi"));
        assert_eq!(delta(Wire::OpenAi, &o).as_deref(), Some("hi"));
        // Cross-parsing must yield nothing rather than garbage.
        assert_eq!(delta(Wire::OpenAi, &a), None);
        assert_eq!(delta(Wire::Anthropic, &o), None);
    }

    /// Real call against the configured provider. Opt-in: costs money and adds
    /// seconds, so it stays out of the normal run.
    ///   $env:IV_LIVE_TEST=1; .\build.ps1 test --release
    ///
    /// Three turns on one `Coach`, because that is the shape of a call and the
    /// shape the connection pool is for: turn 1 pays DNS, TCP and TLS, turns 2
    /// and 3 should not. A regression to a use-once agent reads as three
    /// identical times rather than one slow turn and a fast tail.
    #[test]
    fn live_provider_returns_advice() {
        let _ = dotenvy::dotenv();
        if std::env::var("IV_LIVE_TEST").is_err() {
            eprintln!("skipping live test (set IV_LIVE_TEST=1)");
            return;
        }
        let name = std::env::var("IV_PROVIDER").unwrap_or("anthropic".into());
        let provider =
            crate::provider::resolve(&name, std::env::var("IV_MODEL").ok().as_deref()).unwrap();

        let (tx, rx) = crossbeam_channel::unbounded();
        let coach = Coach::new(
            provider,
            "You are a terse interview coach. Reply with 2 short lines.".into(),
            tx,
        );

        for turn in 1..=3 {
            let started = std::time::Instant::now();
            coach.ask(format!(
                "THEM: We had a major outage in quarter {turn} but handled it well."
            ));
            let (mut text, mut ttft) = (String::new(), None);
            while let Ok(m) = rx.recv_timeout(std::time::Duration::from_secs(60)) {
                match m {
                    Msg::Advice(_, t) => {
                        ttft.get_or_insert_with(|| started.elapsed());
                        text.push_str(&t);
                    }
                    Msg::AdviceEnd(_) => break,
                    Msg::Sys(e) => panic!("provider error: {e}"),
                    _ => {}
                }
            }
            eprintln!(
                "{name} turn {turn}: first token {:>7.0?}, total {:>7.0?}",
                ttft.expect("no tokens streamed"),
                started.elapsed()
            );
            assert!(!text.trim().is_empty(), "no text on turn {turn}");
        }
    }

    #[test]
    fn openai_body_omits_anthropic_only_knobs() {
        let b = body(&p(Wire::OpenAi), "sys", "hello");
        assert!(b["speed"].is_null() && b["output_config"].is_null());
        assert_eq!(b["messages"][0]["role"], "system");

        let b = body(&p(Wire::Anthropic), "sys", "hello");
        assert_eq!(b["speed"], "fast");
        assert_eq!(b["system"][0]["cache_control"]["type"], "ephemeral");
    }
}
