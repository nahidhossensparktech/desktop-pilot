//! Energy-based voice activity detection on 16 kHz mono audio.
//!
//! Cuts the stream into utterances so Whisper only runs while someone speaks: speech
//! starts when the level rises clearly above an adaptive noise floor and ends after a
//! short silence. A little audio before the start is kept so first syllables survive.

use std::collections::VecDeque;

/// Sample rate the detector (and Whisper) works at.
pub const SAMPLE_RATE: usize = 16_000;
/// 30 ms analysis frames.
const FRAME: usize = SAMPLE_RATE * 30 / 1000;
/// Speech must last this many frames (90 ms) to start an utterance.
const START_FRAMES: usize = 3;
/// This many silent frames (≈1.2 s) end an utterance. Shorter cut commands at
/// natural pauses ("open notepad and … type hello" ended at "and" with 0.7 s).
const END_FRAMES: usize = 40;
/// Audio kept from before the detected start (300 ms).
const PRE_ROLL: usize = SAMPLE_RATE * 3 / 10;
/// Utterances are cut at 15 s.
const MAX_SAMPLES: usize = SAMPLE_RATE * 15;
/// Utterances with less than this much voiced audio (240 ms) are noise: clicks,
/// knocks, coughs.
const MIN_VOICED_FRAMES: usize = 8;
/// Speech must be this far above the noise floor…
const MARGIN_DB: f32 = 9.0;
/// …and at least this loud.
const MIN_SPEECH_DB: f32 = -48.0;

fn rms_db(frame: &[f32]) -> f32 {
    let mean = frame.iter().map(|s| s * s).sum::<f32>() / frame.len().max(1) as f32;
    10.0 * mean.max(1e-10).log10()
}

/// Streaming voice activity detector.
#[derive(Debug)]
pub struct Vad {
    pending: Vec<f32>,
    pre_roll: VecDeque<f32>,
    current: Vec<f32>,
    speaking: bool,
    speech_run: usize,
    silence_run: usize,
    /// Loud frames in the current utterance.
    voiced: usize,
    noise_db: f32,
    level_db: f32,
}

impl Default for Vad {
    fn default() -> Self {
        Self {
            pending: Vec::new(),
            pre_roll: VecDeque::with_capacity(PRE_ROLL),
            current: Vec::new(),
            speaking: false,
            speech_run: 0,
            silence_run: 0,
            voiced: 0,
            noise_db: -60.0,
            level_db: -90.0,
        }
    }
}

impl Vad {
    /// Feeds audio; returns any utterances that ended.
    pub fn push(&mut self, samples: &[f32]) -> Vec<Vec<f32>> {
        self.pending.extend_from_slice(samples);
        let mut done = Vec::new();
        let mut offset = 0;
        while self.pending.len() - offset >= FRAME {
            let frame: Vec<f32> = self.pending[offset..offset + FRAME].to_vec();
            offset += FRAME;
            if let Some(u) = self.frame(&frame) {
                done.push(u);
            }
        }
        self.pending.drain(..offset);
        done
    }

    fn frame(&mut self, frame: &[f32]) -> Option<Vec<f32>> {
        let db = rms_db(frame);
        self.level_db = db;
        let loud = db > self.noise_db + MARGIN_DB && db > MIN_SPEECH_DB;
        if self.speaking {
            self.current.extend_from_slice(frame);
            if loud {
                self.silence_run = 0;
                self.voiced += 1;
            } else {
                self.silence_run += 1;
            }
            if self.silence_run >= END_FRAMES || self.current.len() >= MAX_SAMPLES {
                return self.finish();
            }
            return None;
        }
        // Not speaking: track the noise floor (fast down, slow up).
        let alpha = if db < self.noise_db { 0.3 } else { 0.02 };
        self.noise_db += (db - self.noise_db) * alpha;
        if loud {
            self.speech_run += 1;
            if self.speech_run >= START_FRAMES {
                self.speaking = true;
                self.silence_run = 0;
                self.voiced = self.speech_run;
                self.current = self.pre_roll.drain(..).collect();
                self.current.extend_from_slice(frame);
                return None;
            }
        } else {
            self.speech_run = 0;
        }
        for &s in frame {
            if self.pre_roll.len() == PRE_ROLL {
                self.pre_roll.pop_front();
            }
            self.pre_roll.push_back(s);
        }
        None
    }

    fn finish(&mut self) -> Option<Vec<f32>> {
        self.speaking = false;
        self.speech_run = 0;
        self.silence_run = 0;
        let voiced = std::mem::take(&mut self.voiced);
        let utterance = std::mem::take(&mut self.current);
        (voiced >= MIN_VOICED_FRAMES).then_some(utterance)
    }

    /// Drops everything heard so far (used while the app is speaking).
    pub fn reset(&mut self) {
        let noise = self.noise_db;
        *self = Self::default();
        self.noise_db = noise;
    }

    /// Whether an utterance is in progress.
    pub fn is_speaking(&self) -> bool {
        self.speaking
    }

    /// Input level for meters and the listening orb, 0.0 (silence) to 1.0 (loud).
    pub fn level(&self) -> f32 {
        ((self.level_db + 60.0) / 50.0).clamp(0.0, 1.0)
    }
}

/// Converts interleaved samples at `rate` with `channels` to 16 kHz mono.
///
/// Averages channels, low-passes with a moving average over the decimation ratio, then
/// interpolates linearly. Good enough for speech recognition, and needs no extra crate.
#[derive(Debug)]
pub struct Resampler {
    rate: u32,
    channels: usize,
    /// Position of the next output sample, in input samples (fractional).
    pos: f64,
    last: f32,
    smooth: Vec<f32>,
}

impl Resampler {
    pub fn new(rate: u32, channels: usize) -> Self {
        Self {
            rate,
            channels: channels.max(1),
            pos: 0.0,
            last: 0.0,
            smooth: Vec::new(),
        }
    }

    pub fn process(&mut self, interleaved: &[f32]) -> Vec<f32> {
        let mono: Vec<f32> = interleaved
            .chunks(self.channels)
            .map(|c| c.iter().sum::<f32>() / c.len() as f32)
            .collect();
        if self.rate as usize == SAMPLE_RATE {
            return mono;
        }
        let step = f64::from(self.rate) / SAMPLE_RATE as f64;
        // Moving average over ≈ the decimation ratio removes most aliasing.
        let taps = step.round().max(1.0) as usize;
        let mut input = Vec::with_capacity(mono.len() + 1);
        input.push(self.last);
        for s in mono {
            self.smooth.push(s);
            if self.smooth.len() > taps {
                self.smooth.remove(0);
            }
            input.push(self.smooth.iter().sum::<f32>() / self.smooth.len() as f32);
        }
        let mut out = Vec::with_capacity((input.len() as f64 / step) as usize + 1);
        // `pos` is relative to input[0] (the previous chunk's last sample).
        while self.pos + 1.0 < input.len() as f64 {
            let i = self.pos.floor() as usize;
            let frac = (self.pos - i as f64) as f32;
            out.push(input[i] + (input[i + 1] - input[i]) * frac);
            self.pos += step;
        }
        self.pos -= (input.len() - 1) as f64;
        self.last = *input.last().unwrap_or(&0.0);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(seconds: f32, amp: f32) -> Vec<f32> {
        let n = (seconds * SAMPLE_RATE as f32) as usize;
        (0..n).map(|i| amp * (i as f32 * 0.07).sin()).collect()
    }

    fn noise(seconds: f32, amp: f32) -> Vec<f32> {
        let n = (seconds * SAMPLE_RATE as f32) as usize;
        let mut x = 12345_u32;
        (0..n)
            .map(|_| {
                x = x.wrapping_mul(1_103_515_245).wrapping_add(12345);
                amp * ((x >> 16) as f32 / 32768.0 - 1.0)
            })
            .collect()
    }

    #[test]
    fn finds_one_utterance_between_silences() {
        let mut vad = Vad::default();
        let mut audio = noise(1.0, 0.001);
        audio.extend(tone(1.2, 0.3));
        audio.extend(noise(1.5, 0.001));
        let utterances = vad.push(&audio);
        assert_eq!(utterances.len(), 1);
        let secs = utterances[0].len() as f32 / SAMPLE_RATE as f32;
        assert!((1.2..3.0).contains(&secs), "{secs}");
    }

    #[test]
    fn quiet_room_and_clicks_give_nothing() {
        let mut vad = Vad::default();
        let mut audio = noise(2.0, 0.002);
        audio.extend(tone(0.05, 0.5)); // a click
        audio.extend(noise(2.0, 0.002));
        assert!(vad.push(&audio).is_empty());
    }

    #[test]
    fn works_in_small_chunks() {
        let mut vad = Vad::default();
        let mut audio = noise(1.0, 0.001);
        audio.extend(tone(1.0, 0.3));
        audio.extend(noise(1.5, 0.001));
        let mut found = 0;
        for chunk in audio.chunks(160) {
            found += vad.push(chunk).len();
        }
        assert_eq!(found, 1);
    }

    #[test]
    fn long_speech_is_cut_at_the_cap() {
        let mut vad = Vad::default();
        let mut audio = noise(0.5, 0.001);
        audio.extend(tone(16.0, 0.3));
        let utterances = vad.push(&audio);
        assert_eq!(utterances.len(), 1);
        assert!(utterances[0].len() <= MAX_SAMPLES + FRAME);
    }

    #[test]
    fn resamples_48k_stereo_to_16k_mono() {
        let mut r = Resampler::new(48_000, 2);
        let input: Vec<f32> = (0..48_000 * 2)
            .map(|i| if i % 2 == 0 { 0.5 } else { 0.1 })
            .collect();
        let mut out = Vec::new();
        for chunk in input.chunks(960) {
            out.extend(r.process(chunk));
        }
        assert!((15_990..=16_010).contains(&out.len()), "{}", out.len());
        assert!(out[100..].iter().all(|s| (s - 0.3).abs() < 1e-4));
    }

    #[test]
    fn sixteen_k_mono_passes_through() {
        let mut r = Resampler::new(16_000, 1);
        assert_eq!(r.process(&[0.1, 0.2, 0.3]), vec![0.1, 0.2, 0.3]);
    }
}
