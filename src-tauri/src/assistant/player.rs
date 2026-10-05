//! Plays the natural voice's sound: 24 kHz clips, one per sentence, one after another on the
//! default output.

use anyhow::{anyhow, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, Sample, SampleFormat, SizedSample};
use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

pub const SAMPLE_RATE: u32 = 24_000;

#[derive(Default)]
struct Queue {
    /// Mono, 24 kHz.
    clips: VecDeque<Vec<f32>>,
    /// Position in the first clip, in 24 kHz samples (fractional, for resampling).
    pos: f64,
    /// Something has played since the queue was last cleared.
    audible: bool,
}

impl Queue {
    /// Fills `out` (mono, at the device's rate) with what's queued, `step` clip samples apart.
    fn fill(&mut self, out: &mut [f32], step: f64) {
        let mut i = 0;
        while i < out.len() {
            let Some(samples) = self.clips.front() else { break };
            self.audible = true;
            while i < out.len() {
                let at = self.pos as usize;
                if at + 1 >= samples.len() {
                    break;
                }
                let frac = (self.pos - at as f64) as f32;
                out[i] = samples[at] * (1.0 - frac) + samples[at + 1] * frac;
                self.pos += step;
                i += 1;
            }
            if i < out.len() {
                self.clips.pop_front();
                self.pos = 0.0;
            }
        }
        out[i..].fill(0.0);
    }
}

/// The output is only open while there's something to play (and briefly after), on its own
/// thread since cpal streams are not `Send`.
pub struct Player {
    queue: Arc<(Mutex<Queue>, Condvar)>,
}

/// Keep the output open this long after the last clip, so the next sentence doesn't wait for it.
const LINGER: Duration = Duration::from_secs(3);

impl Player {
    pub fn new() -> Self {
        let queue = Arc::new((Mutex::new(Queue::default()), Condvar::new()));
        let q = queue.clone();
        std::thread::Builder::new().name("murmur-voice-out".into()).spawn(move || player_thread(&q)).expect("spawn voice output thread");
        Self { queue }
    }

    /// Queues a clip to play after the ones before it.
    pub fn add(&self, samples: Vec<f32>) {
        let (queue, wake) = &*self.queue;
        queue.lock().unwrap().clips.push_back(samples);
        wake.notify_all();
    }

    /// Cuts off whatever is playing and drops what's waiting.
    pub fn clear(&self) {
        *self.queue.0.lock().unwrap() = Queue::default();
    }

    /// Sound is playing or waiting to.
    pub fn busy(&self) -> bool {
        !self.queue.0.lock().unwrap().clips.is_empty()
    }

    /// Something has started playing since the last `clear`.
    pub fn audible(&self) -> bool {
        self.queue.0.lock().unwrap().audible
    }
}

fn player_thread(queue: &Arc<(Mutex<Queue>, Condvar)>) {
    let (lock, wake) = &**queue;
    loop {
        {
            let mut q = lock.lock().unwrap();
            while q.clips.is_empty() {
                q = wake.wait(q).unwrap();
            }
        }
        let stream = match open_output(queue.clone()) {
            Ok(s) => s,
            Err(e) => {
                mlog!("voice: no sound output ({e})");
                lock.lock().unwrap().clips.clear();
                continue;
            }
        };
        let mut idle_since = None;
        loop {
            std::thread::sleep(Duration::from_millis(50));
            if !lock.lock().unwrap().clips.is_empty() {
                idle_since = None;
            } else if idle_since.get_or_insert_with(Instant::now).elapsed() >= LINGER {
                break;
            }
        }
        drop(stream);
    }
}

fn open_output(queue: Arc<(Mutex<Queue>, Condvar)>) -> Result<cpal::Stream> {
    let device = cpal::default_host().default_output_device().ok_or_else(|| anyhow!("no output device"))?;
    let supported = device.default_output_config()?;
    let channels = supported.channels() as usize;
    let step = SAMPLE_RATE as f64 / supported.sample_rate() as f64;
    let config = supported.config();
    let err = |e| mlog!("voice output error: {e}");
    let stream = match supported.sample_format() {
        SampleFormat::F32 => device.build_output_stream(config, move |d: &mut [f32], _: &_| fill(d, channels, step, &queue), err, None)?,
        SampleFormat::I16 => device.build_output_stream(config, move |d: &mut [i16], _: &_| fill(d, channels, step, &queue), err, None)?,
        SampleFormat::I32 => device.build_output_stream(config, move |d: &mut [i32], _: &_| fill(d, channels, step, &queue), err, None)?,
        SampleFormat::U16 => device.build_output_stream(config, move |d: &mut [u16], _: &_| fill(d, channels, step, &queue), err, None)?,
        other => return Err(anyhow!("unsupported output sample format {other}")),
    };
    stream.play()?;
    Ok(stream)
}

fn fill<T>(out: &mut [T], channels: usize, step: f64, queue: &(Mutex<Queue>, Condvar))
where
    T: Sample + SizedSample + FromSample<f32>,
{
    let mut mono = vec![0.0f32; out.len() / channels.max(1)];
    queue.0.lock().unwrap().fill(&mut mono, step);
    for (frame, v) in out.chunks_mut(channels.max(1)).zip(mono) {
        frame.fill(T::from_sample(v));
    }
}
