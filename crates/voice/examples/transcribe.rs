//! Runs a 16-bit PCM WAV file through the voice pipeline (speech detection, wake
//! phrase, Whisper) and prints what it heard. Developer tool for checking models.
//!
//! `cargo run -p dp-voice --example transcribe -- <model.bin> <file.wav> [auto|bn|en]`

#![allow(clippy::print_stdout)] // a command-line tool

use std::process::ExitCode;

use dp_voice::vad::{Resampler, Vad};
use dp_voice::{Language, Transcriber, find_wake, strip_wake};

/// Reads a PCM16 WAV: returns (samples as f32, sample rate, channels).
fn read_wav(path: &str) -> Result<(Vec<f32>, u32, usize), String> {
    let data = std::fs::read(path).map_err(|e| e.to_string())?;
    if data.len() < 12 || &data[..4] != b"RIFF" || &data[8..12] != b"WAVE" {
        return Err("not a WAV file".into());
    }
    let (mut rate, mut channels, mut bits) = (0_u32, 0_usize, 0_u16);
    let mut i = 12;
    while i + 8 <= data.len() {
        let id = &data[i..i + 4];
        let len = u32::from_le_bytes([data[i + 4], data[i + 5], data[i + 6], data[i + 7]]) as usize;
        let body = &data[i + 8..(i + 8 + len).min(data.len())];
        if id == b"fmt " && body.len() >= 16 {
            channels = u16::from_le_bytes([body[2], body[3]]) as usize;
            rate = u32::from_le_bytes([body[4], body[5], body[6], body[7]]);
            bits = u16::from_le_bytes([body[14], body[15]]);
        } else if id == b"data" {
            if bits != 16 {
                return Err(format!("{bits}-bit WAV; need 16-bit PCM"));
            }
            let samples = body
                .as_chunks::<2>()
                .0
                .iter()
                .map(|b| f32::from(i16::from_le_bytes(*b)) / 32768.0)
                .collect();
            return Ok((samples, rate, channels));
        }
        i += 8 + len + (len & 1);
    }
    Err("no audio data".into())
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("usage: transcribe <model.bin> <file.wav> [auto|bn|en]");
        return ExitCode::FAILURE;
    }
    let lang = Language::from_id(args.get(3).map_or("auto", String::as_str));
    let (raw, rate, channels) = match read_wav(&args[2]) {
        Ok(w) => w,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };
    let pcm = Resampler::new(rate, channels).process(&raw);
    let mut vad = Vad::default();
    let mut utterances = vad.push(&pcm);
    // Flush with trailing silence.
    utterances.extend(vad.push(&vec![0.0; 16_000 * 2]));
    let mut stt = match Transcriber::load(std::path::Path::new(&args[1])) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };
    println!("utterances: {}", utterances.len());
    for u in utterances {
        let start = std::time::Instant::now();
        let wake_pass = stt
            .transcribe(&u[..u.len().min(40_000)], Language::English)
            .map(|t| t.text)
            .unwrap_or_default();
        let full = stt.transcribe(&u, lang);
        println!(
            "{:.1}s audio, {} ms\n  wake pass: {wake_pass:?} -> {:?}\n  full: {:?}",
            u.len() as f32 / 16_000.0,
            start.elapsed().as_millis(),
            find_wake(&wake_pass),
            full.map(|t| (t.lang, t.text.clone(), strip_wake(&t.text)))
        );
    }
    ExitCode::SUCCESS
}
