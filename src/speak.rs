//! Windows SAPI voice, on its own thread.
//!
//! It holds `mute` while talking so the loopback stream doesn't transcribe the
//! AI's own voice straight back in as the other person. The COM object is
//! created inside the thread that uses it, which keeps it off every other one.

use crossbeam_channel::{Sender, unbounded};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use windows::Win32::Media::Speech::{ISpVoice, SPF_DEFAULT, SpVoice};
use windows::Win32::System::Com::{
    CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx,
};
use windows::core::PCWSTR;

pub struct Speaker {
    tx: Sender<String>,
}

impl Speaker {
    pub fn new(mute: Arc<AtomicBool>) -> Self {
        let (tx, rx) = unbounded::<String>();
        std::thread::spawn(move || unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            let voice: ISpVoice = match CoCreateInstance(&SpVoice, None, CLSCTX_ALL) {
                Ok(v) => v,
                Err(_) => return, // no voice installed: stay silent, HUD still works
            };
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
