//! Windows SAPI voice, on its own thread.
//!
//! It holds `mute` for as long as it is talking. Which capture paths honour that
//! flag is decided in `audio.rs`, not here: the endpoint pumps do, so a loopback
//! doesn't transcribe the AI's own voice straight back in as the other person.
//! The COM object is created inside the thread that uses it, which keeps it off
//! every other one.
//!
//! **The voices SAPI offers by default are the worst ones installed.** An
//! `ISpVoice` with no `SetVoice` chooses from the SAPI5 category, which on a
//! current Windows holds only the three *Desktop* voices — David, Zira, Hazel —
//! the oldest concatenative generation, and the whole reason the panel sounded
//! robotic. The OneCore voices (Mark, George, Susan, and non-Desktop
//! David/Hazel/Zira) sit in a category SAPI will not enumerate unless asked for
//! it by id, which is what `voices` does. Everything here degrades to the
//! default voice rather than to silence: a robotic voice still does the job,
//! and `--speak` failing closed would be the worse outcome.

use crate::Msg;
use crossbeam_channel::{Sender, unbounded};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use windows::Win32::Media::Speech::{
    ISpObjectToken, ISpObjectTokenCategory, ISpVoice, SPF_DEFAULT, SpObjectTokenCategory, SpVoice,
};
use windows::Win32::System::Com::{
    CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree,
};
use windows::core::PCWSTR;

/// Where the better voices are. The SAPI5 category holds the Desktop set; this
/// one holds what the modern speech stack installs.
const ONECORE: PCWSTR =
    windows::core::w!("HKEY_LOCAL_MACHINE\\SOFTWARE\\Microsoft\\Speech_OneCore\\Voices");

/// Every OneCore voice, as (token, description). Empty is an ordinary answer,
/// not a failure: a machine with none simply keeps the default voice.
///
/// # Safety
/// COM must already be initialised on this thread.
unsafe fn voices() -> Vec<(ISpObjectToken, String)> {
    unsafe {
        let Ok(category) =
            CoCreateInstance::<_, ISpObjectTokenCategory>(&SpObjectTokenCategory, None, CLSCTX_ALL)
        else {
            return Vec::new();
        };
        if category.SetId(ONECORE, false).is_err() {
            return Vec::new();
        }
        let Ok(tokens) = category.EnumTokens(PCWSTR::null(), PCWSTR::null()) else {
            return Vec::new();
        };
        let mut found = Vec::new();
        // One at a time: there are six on a stock machine, so a batched fetch
        // would be more code and no faster.
        loop {
            let mut slot: Option<ISpObjectToken> = None;
            let mut got = 0u32;
            if tokens.Next(1, &mut slot, Some(&mut got)).is_err() || got == 0 {
                return found;
            }
            let Some(token) = slot.take() else {
                return found;
            };
            // The default value of a voice token is its display name. SAPI
            // allocates it, so it has to be freed here.
            let description = match token.GetStringValue(PCWSTR::null()) {
                Ok(s) => {
                    let text = s.to_string().unwrap_or_default();
                    CoTaskMemFree(Some(s.0 as *const _));
                    text
                }
                Err(_) => String::new(),
            };
            found.push((token, description));
        }
    }
}

/// Which voice `wanted` names, matched the way `--hear` matches an app: any
/// part of the description, case-insensitively. `None` takes the first, which
/// is still not one of the Desktop voices SAPI would have picked.
///
/// A name that matches nothing returns `None` on purpose. Substituting some
/// other voice would let a typo look like a preference that worked, and the
/// caller reports the miss with the list of what is installed instead.
pub fn pick(descriptions: &[String], wanted: Option<&str>) -> Option<usize> {
    let Some(wanted) = wanted else {
        return (!descriptions.is_empty()).then_some(0);
    };
    let wanted = wanted.to_lowercase();
    descriptions
        .iter()
        .position(|d| d.to_lowercase().contains(&wanted))
}

/// Every OneCore voice description — what `--setup` prints, and what a missed
/// `--voice` is reported against.
pub fn available() -> Vec<String> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        voices().into_iter().map(|(_, d)| d).collect()
    }
}

pub struct Speaker {
    tx: Sender<String>,
}

impl Speaker {
    /// `mute` is held for the whole utterance. Who honours it is `audio.rs`'s
    /// decision: the endpoint pumps do, because that mix contains this voice; a
    /// process-loopback stream does not, because the voice is another process
    /// and deafening that stream would only lose the far end.
    pub fn new(mute: Arc<AtomicBool>, wanted: Option<String>, notice: Sender<Msg>) -> Self {
        let (tx, rx) = unbounded::<String>();
        std::thread::spawn(move || unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            let voice: ISpVoice = match CoCreateInstance(&SpVoice, None, CLSCTX_ALL) {
                Ok(v) => v,
                Err(_) => return, // no voice installed: stay silent, HUD still works
            };
            let found = voices();
            let descriptions: Vec<String> = found.iter().map(|(_, d)| d.clone()).collect();
            match pick(&descriptions, wanted.as_deref()) {
                Some(i) if voice.SetVoice(&found[i].0).is_ok() => {
                    let _ = notice.send(Msg::Sys(format!("speak: {}", descriptions[i])));
                }
                _ => {
                    let _ = notice.send(Msg::Sys(match (&wanted, descriptions.is_empty()) {
                        (Some(w), false) => format!(
                            "speak: no voice matches {w:?}; using the default. Installed: {}",
                            descriptions.join(", ")
                        ),
                        _ => "speak: using the default voice".into(),
                    }));
                }
            }
            while let Ok(text) = rx.recv() {
                let w: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
                mute.store(true, Ordering::Relaxed);
                // Synchronous on purpose: `mute` must stay set for the whole utterance.
                let _ = voice.Speak(PCWSTR(w.as_ptr()), SPF_DEFAULT.0 as u32, None);
                if rx.is_empty() {
                    mute.store(false, Ordering::Relaxed);
                }
            }
        });
        Self { tx }
    }

    pub fn say(&self, text: &str) {
        let _ = self.tx.send(text.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::pick;

    fn installed() -> Vec<String> {
        [
            "Microsoft George - English (United Kingdom)",
            "Microsoft Mark - English (United States)",
            "Microsoft Zira - English (United States)",
        ]
        .map(str::to_string)
        .to_vec()
    }

    #[test]
    fn voice_is_matched_by_any_part_of_its_name() {
        assert_eq!(pick(&installed(), Some("mark")), Some(1));
        assert_eq!(pick(&installed(), Some("United Kingdom")), Some(0));
        // No preference takes the first OneCore voice — still not a Desktop one.
        assert_eq!(pick(&installed(), None), Some(0));
    }

    /// Silently substituting another voice would let a typo look like a
    /// preference that worked; the caller reports the miss and the list instead.
    #[test]
    fn an_unmatched_name_is_a_miss_not_a_substitution() {
        assert_eq!(pick(&installed(), Some("ava")), None);
        assert_eq!(pick(&[], Some("mark")), None);
        assert_eq!(pick(&[], None), None);
    }
}
