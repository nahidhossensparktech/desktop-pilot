//! Runs the always-listening assistant on the real microphone and prints its events.
//! Developer tool. Say "Hey Mouse, …" (or play it through the speakers).
//!
//! `cargo run -p dp-voice --example listen -- <model.bin> [seconds] [auto|bn|en]`

#![allow(clippy::print_stdout)] // a command-line tool

use std::time::Duration;

use dp_voice::{Assistant, AssistantConfig, Language, VoiceEvent};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let Some(model) = args.get(1) else {
        println!("usage: listen <model.bin> [seconds] [auto|bn|en]");
        return;
    };
    let seconds = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(30);
    let language = Language::from_id(args.get(3).map_or("auto", String::as_str));
    let start = std::time::Instant::now();
    let peak = std::sync::Arc::new(std::sync::Mutex::new((0.0_f32, start)));
    let _assistant = Assistant::start(
        AssistantConfig {
            model: model.into(),
            language,
            always_listening: true,
        },
        move |e| match e {
            VoiceEvent::Level(l) => {
                // Print the loudest level every 2 s, to check the microphone works.
                let mut p = peak
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                p.0 = p.0.max(l);
                if p.1.elapsed() >= Duration::from_secs(2) {
                    println!(
                        "{:>5.1}s mic level peak {:.2}",
                        start.elapsed().as_secs_f32(),
                        p.0
                    );
                    *p = (0.0, std::time::Instant::now());
                }
            }
            other => println!("{:>5.1}s {other:?}", start.elapsed().as_secs_f32()),
        },
    );
    std::thread::sleep(Duration::from_secs(seconds));
}
