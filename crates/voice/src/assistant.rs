//! The voice assistant: a background thread that owns the Whisper model (and the
//! microphone while it is open) and turns speech into events.
//!
//! Two modes:
//! - **Tap / shortcut** (default): the microphone is closed until [`Assistant::listen_now`]
//!   (orb, mic button or keyboard shortcut); it opens for one command and closes again.
//! - **Always listening**: the microphone stays open to hear "Hey Mouse" at any time.
//!
//! In always-listening mode:
//! States: **sleeping** (each utterance gets a short Whisper pass that only checks for
//! "Hey Mouse") → **awaiting a command** (after a bare "Hey Mouse" or push-to-talk; the
//! next utterance is the command) → back to sleeping. "Hey Mouse, open Safari" in one
//! breath skips the waiting step.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, TryRecvError, channel};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::stt::{Language, Transcriber};
use crate::vad::{SAMPLE_RATE, Vad};
use crate::wake::{find_wake, is_meaningful_command, strip_wake};

/// The wake check only looks at the start of an utterance.
const WAKE_WINDOW: usize = SAMPLE_RATE * 5 / 2;
/// After a bare "Hey Mouse", wait this long for the command.
const COMMAND_WAIT: Duration = Duration::from_secs(8);
/// Level updates for the UI at most this often.
const LEVEL_EVERY: Duration = Duration::from_millis(60);

/// What the assistant reports.
#[derive(Clone, Debug, PartialEq)]
pub enum VoiceEvent {
    /// Model loaded and microphone open; listening for "Hey Mouse".
    Ready,
    /// Microphone level, 0.0–1.0.
    Level(f32),
    /// "Hey Mouse" (or push-to-talk): now listening for the command.
    WakeHeard,
    /// The command is being transcribed.
    Transcribing,
    /// A command. `lang` is "bn" or "en".
    Heard { text: String, lang: &'static str },
    /// Woken, but nothing usable was said.
    NotUnderstood,
    /// Stopped waiting for a command; back to listening for "Hey Mouse".
    Idle,
    /// The assistant stopped (bad model, no microphone, …).
    Error(String),
}

#[derive(Debug)]
enum Control {
    ListenNow,
    /// Stop waiting for a command (closes the microphone in tap mode).
    Cancel,
    Language(Language),
    Shutdown,
}

/// Settings the assistant starts with.
#[derive(Clone, Debug)]
pub struct AssistantConfig {
    pub model: PathBuf,
    pub language: Language,
    /// Keep the microphone open for "Hey Mouse"; `false` = open it only on request.
    pub always_listening: bool,
}

/// Handle to the background assistant. Dropping it stops the thread.
#[derive(Debug)]
pub struct Assistant {
    control: Sender<Control>,
    muted: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Assistant {
    /// Starts listening. `on_event` runs on the assistant thread; keep it cheap (send
    /// to a channel). Startup problems arrive as [`VoiceEvent::Error`].
    pub fn start(config: AssistantConfig, on_event: impl Fn(VoiceEvent) + Send + 'static) -> Self {
        let (control, rx) = channel();
        let muted = Arc::new(AtomicBool::new(false));
        let thread_muted = muted.clone();
        let thread = std::thread::Builder::new()
            .name("dp-voice".into())
            .spawn(move || run(config, &rx, &thread_muted, &on_event))
            .ok();
        Self {
            control,
            muted,
            thread,
        }
    }

    /// Push-to-talk: treat the next utterance as a command, without "Hey Mouse".
    pub fn listen_now(&self) {
        let _ = self.control.send(Control::ListenNow);
    }

    /// Stop listening for a command now (the conversation was closed).
    pub fn cancel(&self) {
        let _ = self.control.send(Control::Cancel);
    }

    /// Ignore the microphone (while the app itself is speaking).
    pub fn set_muted(&self, muted: bool) {
        self.muted.store(muted, Ordering::Relaxed);
    }

    pub fn set_language(&self, language: Language) {
        let _ = self.control.send(Control::Language(language));
    }
}

impl Drop for Assistant {
    fn drop(&mut self) {
        let _ = self.control.send(Control::Shutdown);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

enum State {
    Sleeping,
    Awaiting(Instant),
}

/// Speech detection, wake logic and Whisper, fed with 16 kHz mono audio. Separate
/// from the microphone so tests can feed it recorded audio.
pub(crate) struct Listener {
    stt: Transcriber,
    vad: Vad,
    state: State,
    pub(crate) language: Language,
    last_level: Instant,
}

impl Listener {
    pub(crate) fn new(stt: Transcriber, language: Language) -> Self {
        Self {
            stt,
            vad: Vad::default(),
            state: State::Sleeping,
            language,
            last_level: Instant::now(),
        }
    }

    /// Push-to-talk: the next utterance is a command.
    pub(crate) fn listen_now(&mut self, emit: &dyn Fn(VoiceEvent)) {
        self.vad.reset();
        self.state = State::Awaiting(Instant::now() + COMMAND_WAIT);
        emit(VoiceEvent::WakeHeard);
    }

    /// Drops what was heard so far (while muted).
    pub(crate) fn reset(&mut self) {
        self.vad.reset();
    }

    /// Stops waiting for a command.
    pub(crate) fn cancel(&mut self, emit: &dyn Fn(VoiceEvent)) {
        if !self.is_sleeping() {
            self.state = State::Sleeping;
            self.vad.reset();
            emit(VoiceEvent::Idle);
        }
    }

    /// Not waiting for a command (tap mode closes the microphone then).
    pub(crate) fn is_sleeping(&self) -> bool {
        matches!(self.state, State::Sleeping)
    }

    pub(crate) fn feed(&mut self, chunk: &[f32], emit: &dyn Fn(VoiceEvent)) {
        let utterances = self.vad.push(chunk);
        if self.last_level.elapsed() >= LEVEL_EVERY {
            self.last_level = Instant::now();
            emit(VoiceEvent::Level(self.vad.level()));
        }
        if let State::Awaiting(until) = self.state
            && Instant::now() > until
            && !self.vad.is_speaking()
        {
            self.state = State::Sleeping;
            emit(VoiceEvent::Idle);
        }
        for utterance in utterances {
            let state = std::mem::replace(&mut self.state, State::Sleeping);
            self.state = handle(&mut self.stt, &utterance, self.language, state, emit);
        }
    }
}

fn run(
    config: AssistantConfig,
    control: &Receiver<Control>,
    muted: &AtomicBool,
    emit: &dyn Fn(VoiceEvent),
) {
    let stt = match Transcriber::load(&config.model) {
        Ok(s) => s,
        Err(e) => return emit(VoiceEvent::Error(e.to_string())),
    };
    let mut listener = Listener::new(stt, config.language);
    if !config.always_listening {
        emit(VoiceEvent::Ready);
        return run_on_request(&mut listener, control, muted, emit);
    }
    let mic = match crate::audio::open() {
        Ok(m) => m,
        Err(e) => return emit(VoiceEvent::Error(e.to_string())),
    };
    emit(VoiceEvent::Ready);
    loop {
        match control.try_recv() {
            Ok(Control::ListenNow) => listener.listen_now(emit),
            Ok(Control::Cancel) => listener.cancel(emit),
            Ok(Control::Language(l)) => listener.language = l,
            Ok(Control::Shutdown) | Err(TryRecvError::Disconnected) => return,
            Err(TryRecvError::Empty) => {}
        }
        let chunk = match mic.chunks.recv_timeout(Duration::from_millis(50)) {
            Ok(c) => c,
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => {
                return emit(VoiceEvent::Error("the microphone stopped".into()));
            }
        };
        if muted.load(Ordering::Relaxed) {
            listener.reset();
            continue;
        }
        listener.feed(&chunk, emit);
    }
}

/// Tap / shortcut mode: the microphone is closed until asked for, then open for one
/// command (or until the command wait times out), then closed again.
fn run_on_request(
    listener: &mut Listener,
    control: &Receiver<Control>,
    muted: &AtomicBool,
    emit: &dyn Fn(VoiceEvent),
) {
    loop {
        match control.recv() {
            Ok(Control::ListenNow) => {}
            Ok(Control::Language(l)) => {
                listener.language = l;
                continue;
            }
            Ok(Control::Cancel) => continue,
            Ok(Control::Shutdown) | Err(_) => return,
        }
        let mic = match crate::audio::open() {
            Ok(m) => m,
            Err(e) => {
                emit(VoiceEvent::Error(e.to_string()));
                return;
            }
        };
        listener.listen_now(emit);
        while !listener.is_sleeping() {
            match control.try_recv() {
                Ok(Control::Language(l)) => listener.language = l,
                Ok(Control::Cancel) => listener.cancel(emit),
                Ok(Control::Shutdown) | Err(TryRecvError::Disconnected) => return,
                Ok(Control::ListenNow) | Err(TryRecvError::Empty) => {}
            }
            let chunk = match mic.chunks.recv_timeout(Duration::from_millis(50)) {
                Ok(c) => c,
                Err(RecvTimeoutError::Timeout) => continue,
                Err(RecvTimeoutError::Disconnected) => {
                    return emit(VoiceEvent::Error("the microphone stopped".into()));
                }
            };
            if muted.load(Ordering::Relaxed) {
                listener.reset();
                continue;
            }
            listener.feed(&chunk, emit);
        }
        // Dropping the stream closes the microphone (the orange dot goes away).
        drop(mic);
        emit(VoiceEvent::Level(0.0));
    }
}

/// Handles one finished utterance; returns the next state.
fn handle(
    stt: &mut Transcriber,
    utterance: &[f32],
    language: Language,
    state: State,
    emit: &dyn Fn(VoiceEvent),
) -> State {
    let command = |stt: &mut Transcriber, from_wake: bool| -> Option<(String, &'static str)> {
        emit(VoiceEvent::Transcribing);
        match stt.transcribe(utterance, language) {
            Ok(t) => {
                let text = if from_wake {
                    strip_wake(&t.text)
                } else {
                    // Some people say "Hey Mouse" again after push-to-talk.
                    find_wake(&t.text).unwrap_or(t.text)
                };
                Some((text, t.lang))
            }
            Err(e) => {
                emit(VoiceEvent::Error(e.to_string()));
                None
            }
        }
    };
    match state {
        State::Awaiting(_) => {
            match command(stt, false) {
                // Noise can come out as "Nothing." or "Thank you.": not a command.
                Some((text, lang)) if is_meaningful_command(&text) => {
                    emit(VoiceEvent::Heard { text, lang });
                }
                Some(_) => emit(VoiceEvent::NotUnderstood),
                None => {}
            }
            State::Sleeping
        }
        State::Sleeping => {
            let head = &utterance[..utterance.len().min(WAKE_WINDOW)];
            let Ok(wake) = stt.transcribe(head, Language::English) else {
                return State::Sleeping;
            };
            let Some(rest) = find_wake(&wake.text) else {
                return State::Sleeping;
            };
            tracing::debug!("wake phrase heard: {:?}", wake.text);
            let bare = rest.trim().is_empty() && utterance.len() <= WAKE_WINDOW;
            if bare {
                emit(VoiceEvent::WakeHeard);
                return State::Awaiting(Instant::now() + COMMAND_WAIT);
            }
            match command(stt, true) {
                Some((text, lang)) if is_meaningful_command(&text) => {
                    emit(VoiceEvent::Heard { text, lang });
                    State::Sleeping
                }
                Some(_) => {
                    emit(VoiceEvent::WakeHeard);
                    State::Awaiting(Instant::now() + COMMAND_WAIT)
                }
                None => State::Sleeping,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    //! Feeds recorded speech through the real listener. Needs a Whisper model and the
    //! test clips; run with
    //! `DP_VOICE_MODEL=… DP_VOICE_CLIPS=<dir with en1/en2/noise/bn1.wav> cargo test -p dp-voice -- --ignored`.

    use std::sync::Mutex;

    use super::*;
    use crate::vad::Resampler;

    fn wav(path: &std::path::Path) -> Vec<f32> {
        let data = std::fs::read(path).unwrap();
        let pos = data.windows(4).position(|w| w == b"data").unwrap();
        let rate = u32::from_le_bytes([data[24], data[25], data[26], data[27]]);
        let channels = usize::from(u16::from_le_bytes([data[22], data[23]]));
        let pcm: Vec<f32> = data[pos + 8..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| f32::from(i16::from_le_bytes(*b)) / 32768.0)
            .collect();
        Resampler::new(rate, channels).process(&pcm)
    }

    #[test]
    #[ignore = "needs a Whisper model and recorded clips"]
    fn recorded_speech_gives_the_expected_events() {
        let model = std::env::var("DP_VOICE_MODEL").unwrap();
        let clips = std::path::PathBuf::from(std::env::var("DP_VOICE_CLIPS").unwrap());
        let stt = Transcriber::load(std::path::Path::new(&model)).unwrap();
        let mut listener = Listener::new(stt, Language::Auto);
        let events = Mutex::new(Vec::new());
        let emit = |e: VoiceEvent| {
            if !matches!(e, VoiceEvent::Level(_) | VoiceEvent::Transcribing) {
                events.lock().unwrap().push(e);
            }
        };
        let silence = vec![0.0_f32; SAMPLE_RATE];
        // Play the clips with silence in between, in 10 ms chunks like the microphone.
        for name in ["en1", "noise", "en2", "en1", "bn1"] {
            let mut audio = wav(&clips.join(format!("{name}.wav")));
            audio.extend_from_slice(&silence);
            for chunk in audio.chunks(160) {
                listener.feed(chunk, &emit);
            }
        }
        let events = events.into_inner().unwrap();
        let heard: Vec<&VoiceEvent> = events.iter().collect();
        // en1: command in one breath.
        assert!(
            matches!(heard[0], VoiceEvent::Heard { text, lang: "en" } if text.to_lowercase().contains("open text")),
            "{events:?}"
        );
        // noise: nothing. en2: bare wake, then en1 is taken as the command (it still
        // starts with "Hey Mouse", which is dropped).
        assert_eq!(heard[1], &VoiceEvent::WakeHeard, "{events:?}");
        assert!(
            matches!(heard[2], VoiceEvent::Heard { text, .. } if text.to_lowercase().starts_with("open text")),
            "{events:?}"
        );
        // bn1: Bangla command in one breath.
        assert!(
            matches!(heard[3], VoiceEvent::Heard { lang: "bn", .. }),
            "{events:?}"
        );
        assert_eq!(heard.len(), 4, "{events:?}");
    }
}
