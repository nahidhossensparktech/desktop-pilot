//! Microphone capture with `cpal`, delivered as 16 kHz mono `f32` chunks.

use std::sync::mpsc::{Receiver, channel};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SizedSample};

use crate::VoiceError;
use crate::vad::Resampler;

/// An open microphone. Audio stops when it is dropped. `cpal` streams are not `Send`
/// on every platform, so keep it on the thread that opened it.
pub struct Microphone {
    _stream: cpal::Stream,
    /// 16 kHz mono chunks, about every 10 ms.
    pub chunks: Receiver<Vec<f32>>,
}

impl std::fmt::Debug for Microphone {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Microphone").finish_non_exhaustive()
    }
}

fn mic_err(e: impl std::fmt::Display) -> VoiceError {
    VoiceError::Microphone(e.to_string())
}

fn build<T>(device: &cpal::Device, config: &cpal::StreamConfig) -> Result<Microphone, VoiceError>
where
    T: SizedSample + Send + 'static,
    f32: FromSample<T>,
{
    let (tx, rx) = channel();
    let mut resampler = Resampler::new(config.sample_rate, usize::from(config.channels));
    let stream = device
        .build_input_stream::<T, _, _>(
            *config,
            move |data: &[T], _| {
                let samples: Vec<f32> = data.iter().map(|s| s.to_sample::<f32>()).collect();
                let _ = tx.send(resampler.process(&samples));
            },
            |e| tracing::warn!("microphone stream error: {e}"),
            None,
        )
        .map_err(mic_err)?;
    stream.play().map_err(mic_err)?;
    Ok(Microphone {
        _stream: stream,
        chunks: rx,
    })
}

/// Opens the default input device. On macOS the first call shows the Microphone
/// permission prompt (for the app that launched Desktop Pilot, e.g. Terminal).
pub fn open() -> Result<Microphone, VoiceError> {
    let host = cpal::default_host();
    let device = host
        .default_input_device()
        .ok_or_else(|| mic_err("no microphone found"))?;
    let supported = device.default_input_config().map_err(mic_err)?;
    let config = supported.config();
    match supported.sample_format() {
        cpal::SampleFormat::F32 => build::<f32>(&device, &config),
        cpal::SampleFormat::I16 => build::<i16>(&device, &config),
        cpal::SampleFormat::U16 => build::<u16>(&device, &config),
        cpal::SampleFormat::I32 => build::<i32>(&device, &config),
        other => Err(mic_err(format!("unsupported sample format {other:?}"))),
    }
}
