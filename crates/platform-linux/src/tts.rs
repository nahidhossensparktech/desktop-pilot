//! Talk-back with Speech Dispatcher (`spd-say`) or `espeak-ng`, whichever is installed.

use std::path::Path;
use std::process::Command;

use dp_platform::Capability;
use dp_platform::speech::CommandSpeech;

fn find(program: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|paths| {
        std::env::split_paths(&paths).any(|dir| Path::new(&dir).join(program).is_file())
    })
}

pub(crate) fn speech() -> CommandSpeech {
    let spd = find("spd-say");
    let espeak = find("espeak-ng");
    let capability = if spd || espeak {
        Capability::Available
    } else {
        Capability::Unavailable {
            reason: "no speech program found".to_owned(),
            workaround: Some("Install speech-dispatcher or espeak-ng".to_owned()),
        }
    };
    CommandSpeech::new(
        Box::new(move |lang| {
            if spd {
                let mut cmd = Command::new("spd-say");
                cmd.args(["--wait", "--pipe-mode", "--language", lang]);
                Some(cmd)
            } else if espeak {
                let mut cmd = Command::new("espeak-ng");
                cmd.args(["--stdin", "-v", lang]);
                Some(cmd)
            } else {
                None
            }
        }),
        capability,
    )
}
