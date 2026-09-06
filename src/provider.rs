//! Provider presets.
//!
//! Two wire formats cover everything here. OpenAI's `/chat/completions` shape is
//! spoken by OpenAI, DeepSeek, Gemini *and* OpenRouter, so supporting all of them
//! is one extra code path, not four. Anthropic keeps its own only because
//! `speed:"fast"` + `effort:"low"` is what buys the ~1.2 s loop.
//!
//! OpenRouter is the cheapest way to reach every model from one key.

use anyhow::{Context, Result, bail};

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Wire {
    Anthropic,
    OpenAi,
}

#[derive(Clone)]
pub struct Provider {
    pub url: &'static str,
    pub model: String,
    pub key: String,
    pub wire: Wire,
}

impl std::fmt::Debug for Provider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Provider")
            .field("url", &self.url)
            .field("model", &self.model)
            .field("key", &"[redacted]")
            .field("wire", &self.wire)
            .finish()
    }
}

/// Preset lookup, kept pure so tests never have to touch the process
/// environment — `set_var` leaks across tests, which share one process.
fn preset(name: &str) -> Result<(&'static str, Option<&'static str>, &'static str, Wire)> {
    Ok(match name {
        "anthropic" => (
            "https://api.anthropic.com/v1/messages",
            Some("claude-opus-5"),
            "ANTHROPIC_API_KEY",
            Wire::Anthropic,
        ),
        "deepseek" => (
            "https://api.deepseek.com/chat/completions",
            Some("deepseek-chat"),
            "DEEPSEEK_API_KEY",
            Wire::OpenAi,
        ),
        "openai" => (
            "https://api.openai.com/v1/chat/completions",
            None,
            "OPENAI_API_KEY",
            Wire::OpenAi,
        ),
        "gemini" => (
            "https://generativelanguage.googleapis.com/v1beta/openai/chat/completions",
            None,
            "GEMINI_API_KEY",
            Wire::OpenAi,
        ),
        "openrouter" => (
            "https://openrouter.ai/api/v1/chat/completions",
            None,
            "OPENROUTER_API_KEY",
            Wire::OpenAi,
        ),
        other => bail!(
            "unknown provider {other:?} — use anthropic, openai, deepseek, gemini,              openrouter, or none for no coach at all"
        ),
    })
}

/// `model` is required where published model ids move fast enough that a
/// hardcoded default would silently 404. Better a clear error than a dead panel
/// mid-call.
pub fn resolve(name: &str, model: Option<&str>) -> Result<Provider> {
    let (url, default_model, env, wire) = preset(name)?;

    let model = match (model, default_model) {
        (Some(m), _) if !m.trim().is_empty() => m.trim().to_string(),
        (Some(_), _) => bail!("--model cannot be blank"),
        (None, Some(d)) => d.to_string(),
        (None, None) => bail!(
            "--model is required for {name} (ids change often). \
             OpenRouter example: --model anthropic/claude-opus-5"
        ),
    };

    // An empty `.env` line yields Ok("") rather than an error, which would sail
    // past this check and fail as a 401 mid-call. Treat blank as absent.
    let key = std::env::var(env)
        .ok()
        .map(|k| k.trim().to_string())
        .filter(|k| !k.is_empty())
        .with_context(|| format!("set {env} (in .env, or $env:{env} = '...')"))?;

    Ok(Provider {
        url,
        model,
        key,
        wire,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_map_to_the_right_wire() {
        // The four OpenAI-compatible vendors are one code path, not four.
        for name in ["openai", "deepseek", "gemini", "openrouter"] {
            assert_eq!(preset(name).unwrap().3, Wire::OpenAi, "{name}");
        }
        assert_eq!(preset("anthropic").unwrap().3, Wire::Anthropic);

        // Unknown provider is rejected rather than guessed at.
        assert!(preset("nope").is_err());

        // Only the providers with stable ids carry a default model.
        assert!(preset("anthropic").unwrap().1.is_some());
        assert!(preset("openrouter").unwrap().1.is_none());
    }

    #[test]
    fn missing_model_fails_loudly_rather_than_inventing_one() {
        let e = resolve("openrouter", None).unwrap_err().to_string();
        assert!(e.contains("--model is required"), "got: {e}");
    }

    #[test]
    fn blank_model_is_rejected_and_debug_redacts_credentials() {
        assert!(
            resolve("anthropic", Some("  "))
                .unwrap_err()
                .to_string()
                .contains("blank")
        );
        let p = Provider {
            url: "http://localhost",
            model: "model".into(),
            key: "secret-value".into(),
            wire: Wire::OpenAi,
        };
        assert!(!format!("{p:?}").contains("secret-value"));
    }

    #[test]
    fn blank_key_is_treated_as_absent() {
        // `KEY=` in .env gives Ok("") — without the filter that reaches the API
        // and comes back a 401 mid-call instead of failing at startup.
        assert!(blank(""), "empty");
        assert!(blank("   "), "whitespace");
        assert!(!blank("sk-ant-x"), "real key");
    }

    /// Mirrors the filter in `resolve`, without touching the process env.
    fn blank(k: &str) -> bool {
        Some(k.to_string())
            .map(|k| k.trim().to_string())
            .filter(|k| !k.is_empty())
            .is_none()
    }
}
