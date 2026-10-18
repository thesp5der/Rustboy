//! Sound-card output. The emulator pushes interleaved stereo f32 samples into a shared queue;
//! the audio callback pops them. The emulator also uses the queue length to pace itself, so
//! the sound card's clock (not a timer) sets the emulation speed and audio never drifts.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample, Stream};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

type Queue = Arc<Mutex<VecDeque<f32>>>;

pub struct Audio {
    _stream: Stream, // dropping it stops playback
    queue: Queue,
    pub rate: u32,
}

impl Audio {
    /// Open the default output device, or None if there isn't a usable one.
    pub fn new() -> Option<Self> {
        let dev = cpal::default_host().default_output_device()?;
        let cfg = dev.default_output_config().ok()?;
        let rate = cfg.sample_rate().0;
        let channels = cfg.channels() as usize;
        let format = cfg.sample_format();
        let conf: cpal::StreamConfig = cfg.into();
        let queue: Queue = Arc::new(Mutex::new(VecDeque::new()));
        let stream = match format {
            SampleFormat::F32 => build::<f32>(&dev, &conf, channels, &queue),
            SampleFormat::I16 => build::<i16>(&dev, &conf, channels, &queue),
            SampleFormat::U16 => build::<u16>(&dev, &conf, channels, &queue),
            _ => None,
        }?;
        stream.play().ok()?;
        Some(Self { _stream: stream, queue, rate })
    }

    pub fn push(&self, samples: &[f32]) {
        let mut q = self.queue.lock().unwrap();
        q.extend(samples);
        // Safety valve: never let latency exceed ~0.2 s. Drop an even count to keep L/R aligned.
        let max = self.rate as usize * 2 / 5;
        if q.len() > max {
            let excess = (q.len() - max) & !1;
            q.drain(..excess);
        }
    }

    /// Queued audio in seconds.
    pub fn queued_secs(&self) -> f32 {
        self.queue.lock().unwrap().len() as f32 / 2.0 / self.rate as f32
    }
}

fn build<T: SizedSample + FromSample<f32>>(
    dev: &cpal::Device,
    conf: &cpal::StreamConfig,
    channels: usize,
    queue: &Queue,
) -> Option<Stream> {
    let q = Arc::clone(queue);
    dev.build_output_stream(
        conf,
        move |data: &mut [T], _| {
            let mut q = q.lock().unwrap();
            for frame in data.chunks_mut(channels) {
                let l = q.pop_front().unwrap_or(0.0);
                let r = q.pop_front().unwrap_or(0.0);
                for (i, out) in frame.iter_mut().enumerate() {
                    *out = T::from_sample(match i {
                        0 => l,
                        1 => r,
                        _ => 0.0,
                    });
                }
            }
        },
        |e| eprintln!("audio error: {e}"),
        None,
    )
    .ok()
}
