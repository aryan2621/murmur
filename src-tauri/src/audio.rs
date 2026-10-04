//! Microphone capture. The stream lives on its own thread (cpal streams are not `Send`)
//! and is only open while the hotkey is held, so the OS mic indicator is off otherwise.
//!
//! On macOS, with echo cancellation on (Settings), the default microphone is opened through
//! Apple's voice processing (the echo cancellation FaceTime uses), so music or a video playing
//! from the Mac's own speakers is removed from what's heard. Otherwise, with a chosen microphone,
//! or if that can't be set up, plain capture.

use anyhow::{anyhow, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, Sample, SampleFormat, SizedSample};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

pub const TARGET_RATE: u32 = 16_000;
const LEVEL_INTERVAL: Duration = Duration::from_millis(33);

pub type LevelFn = Box<dyn Fn(f32) + Send + Sync + 'static>;

pub struct Recording {
    /// Mono, 16 kHz.
    pub samples: Vec<f32>,
    /// Loudest RMS seen in any callback chunk, used to skip silent clips.
    pub peak_rms: f32,
}

impl Recording {
    pub fn seconds(&self) -> f32 {
        self.samples.len() as f32 / TARGET_RATE as f32
    }
}

enum Cmd {
    Start(Option<String>, bool, LevelFn, mpsc::Sender<Result<()>>),
    Stop(mpsc::Sender<Result<Recording>>),
}

struct Buffer {
    samples: Vec<f32>,
    peak_rms: f32,
    window_peak: f32,
    last_emit: Instant,
}

pub struct Recorder {
    tx: mpsc::Sender<Cmd>,
}

impl Recorder {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel::<Cmd>();
        std::thread::Builder::new()
            .name("murmur-audio".into())
            .spawn(move || audio_thread(rx))
            .expect("spawn audio thread");
        Self { tx }
    }

    /// `device`: input device name, or `None` for the system default. `echo_cancellation`: remove
    /// the Mac's own playback from the input (system default microphone only).
    pub fn start(&self, device: Option<String>, echo_cancellation: bool, on_level: LevelFn) -> Result<()> {
        let (reply, wait) = mpsc::channel();
        self.tx.send(Cmd::Start(device, echo_cancellation, on_level, reply))?;
        wait.recv()?
    }

    pub fn stop(&self) -> Result<Recording> {
        let (reply, wait) = mpsc::channel();
        self.tx.send(Cmd::Stop(reply))?;
        wait.recv()?
    }
}

/// An open microphone; closing it is dropping it.
enum Stream {
    Plain(#[allow(dead_code)] cpal::Stream),
    #[cfg(target_os = "macos")]
    EchoFree(#[allow(dead_code)] echo_free::Capture),
}

fn audio_thread(rx: mpsc::Receiver<Cmd>) {
    let mut active: Option<(Stream, Arc<Mutex<Buffer>>, u32)> = None;
    for cmd in rx {
        match cmd {
            Cmd::Start(device, echo_cancellation, on_level, reply) => {
                active = None;
                let result = open_stream(device.as_deref(), echo_cancellation, on_level).map(|a| active = Some(a));
                let _ = reply.send(result);
            }
            Cmd::Stop(reply) => {
                let result = match active.take() {
                    Some((stream, buf, rate)) => {
                        drop(stream);
                        let buf = std::mem::replace(
                            &mut *buf.lock().unwrap(),
                            Buffer { samples: vec![], peak_rms: 0.0, window_peak: 0.0, last_emit: Instant::now() },
                        );
                        Ok(Recording { samples: resample(&buf.samples, rate, TARGET_RATE), peak_rms: buf.peak_rms })
                    }
                    None => Err(anyhow!("not recording")),
                };
                let _ = reply.send(result);
            }
        }
    }
}

/// Names of all available microphones.
pub fn input_device_names() -> Vec<String> {
    let Ok(devices) = cpal::default_host().input_devices() else { return vec![] };
    devices.filter_map(|d| d.description().ok().map(|desc| desc.name().to_string())).collect()
}

fn find_device(name: Option<&str>) -> Option<cpal::Device> {
    let host = cpal::default_host();
    if let Some(name) = name {
        let found = host
            .input_devices()
            .ok()
            .and_then(|mut it| it.find(|d| d.description().is_ok_and(|desc| desc.name() == name)));
        if found.is_some() {
            return found;
        }
        mlog!("microphone '{name}' not available, using system default");
    }
    host.default_input_device()
}

fn open_stream(device: Option<&str>, echo_cancellation: bool, on_level: LevelFn) -> Result<(Stream, Arc<Mutex<Buffer>>, u32)> {
    let on_level: Arc<LevelFn> = Arc::new(on_level);
    #[cfg(target_os = "macos")]
    if echo_cancellation && device.is_none() {
        let buf = Arc::new(Mutex::new(Buffer { samples: vec![], peak_rms: 0.0, window_peak: 0.0, last_emit: Instant::now() }));
        match echo_free::open(buf.clone(), on_level.clone()) {
            Ok((capture, rate)) => return Ok((Stream::EchoFree(capture), buf, rate)),
            Err(e) => mlog!("echo cancellation unavailable ({e}); recording without it"),
        }
    }
    let (stream, buf, rate) = open_plain(device, on_level)?;
    Ok((Stream::Plain(stream), buf, rate))
}

fn open_plain(device: Option<&str>, on_level: Arc<LevelFn>) -> Result<(cpal::Stream, Arc<Mutex<Buffer>>, u32)> {
    let device = find_device(device).ok_or_else(|| anyhow!("No microphone found"))?;
    let supported = device.default_input_config()?;
    let rate = supported.sample_rate();
    let channels = supported.channels() as usize;
    let config = supported.config();

    let buf = Arc::new(Mutex::new(Buffer {
        samples: Vec::with_capacity(rate as usize * 30),
        peak_rms: 0.0,
        window_peak: 0.0,
        last_emit: Instant::now(),
    }));
    let b = buf.clone();
    let err = |e| mlog!("audio stream error: {e}");

    let stream = match supported.sample_format() {
        SampleFormat::F32 => device.build_input_stream(config, move |d: &[f32], _: &_| push(d, channels, &b, &*on_level), err, None)?,
        SampleFormat::I16 => device.build_input_stream(config, move |d: &[i16], _: &_| push(d, channels, &b, &*on_level), err, None)?,
        SampleFormat::I32 => device.build_input_stream(config, move |d: &[i32], _: &_| push(d, channels, &b, &*on_level), err, None)?,
        SampleFormat::U16 => device.build_input_stream(config, move |d: &[u16], _: &_| push(d, channels, &b, &*on_level), err, None)?,
        other => return Err(anyhow!("Unsupported microphone sample format {other}")),
    };
    stream.play()?;
    Ok((stream, buf, rate))
}

fn push<T>(data: &[T], channels: usize, buf: &Mutex<Buffer>, on_level: &LevelFn)
where
    T: Sample + SizedSample,
    f32: FromSample<T>,
{
    let mut sum_sq = 0.0f32;
    let mut frames = 0usize;
    let mut b = buf.lock().unwrap();
    for frame in data.chunks(channels) {
        let mono = frame.iter().map(|s| f32::from_sample(*s)).sum::<f32>() / channels as f32;
        sum_sq += mono * mono;
        frames += 1;
        b.samples.push(mono);
    }
    if frames == 0 {
        return;
    }
    let rms = (sum_sq / frames as f32).sqrt();
    b.peak_rms = b.peak_rms.max(rms);
    b.window_peak = b.window_peak.max(rms);
    if b.last_emit.elapsed() >= LEVEL_INTERVAL {
        let level = b.window_peak;
        b.window_peak = 0.0;
        b.last_emit = Instant::now();
        drop(b);
        on_level(level);
    }
}

/// The default microphone through Apple's voice processing (AVAudioEngine): the Mac's own
/// playback is cancelled out of the input, and other audio isn't turned down while listening.
#[cfg(target_os = "macos")]
mod echo_free {
    use super::*;
    use block2::RcBlock;
    use objc2::rc::Retained;
    use objc2::runtime::Bool;
    use objc2::AllocAnyThread;
    use objc2_avf_audio::{
        AVAudioEngine, AVAudioFormat, AVAudioPCMBuffer, AVAudioTime, AVAudioVoiceProcessingOtherAudioDuckingConfiguration,
        AVAudioVoiceProcessingOtherAudioDuckingLevel,
    };
    use std::ptr::NonNull;

    /// Recording until dropped.
    pub struct Capture {
        engine: Retained<AVAudioEngine>,
    }

    impl Drop for Capture {
        fn drop(&mut self) {
            unsafe {
                self.engine.inputNode().removeTapOnBus(0);
                self.engine.stop();
            }
        }
    }

    pub fn open(buf: Arc<Mutex<Buffer>>, on_level: Arc<LevelFn>) -> Result<(Capture, u32)> {
        unsafe {
            let engine = AVAudioEngine::new();
            // Voice processing runs input and output as one unit: set the output side up first.
            let mixer = engine.mainMixerNode();
            let input = engine.inputNode();
            input.setVoiceProcessingEnabled_error(true).map_err(|e| anyhow!("turning it on: {}", e.localizedDescription()))?;
            // Voice processing turns other audio down by default; leave the user's music alone.
            input.setVoiceProcessingOtherAudioDuckingConfiguration(AVAudioVoiceProcessingOtherAudioDuckingConfiguration {
                enableAdvancedDucking: Bool::NO,
                duckingLevel: AVAudioVoiceProcessingOtherAudioDuckingLevel::Min,
            });
            let hardware = input.outputFormatForBus(0);
            let rate = hardware.sampleRate();
            // The processed voice, mono: the unit's input and output sides must agree on this.
            let format = AVAudioFormat::initStandardFormatWithSampleRate_channels(AVAudioFormat::alloc(), rate, 1)
                .ok_or_else(|| anyhow!("no mono format at {rate} Hz"))?;
            // It won't start without a path from input to output: send the mic there, silenced.
            engine.connect_to_format(&input, &mixer, Some(&format));
            mixer.setOutputVolume(0.0);
            let (rate, channels) = (rate as u32, 1usize);
            if rate == 0 {
                return Err(anyhow!("the microphone reported no audio format"));
            }
            let block = RcBlock::new(move |buffer: NonNull<AVAudioPCMBuffer>, _: NonNull<AVAudioTime>| {
                let buffer = buffer.as_ref();
                let frames = buffer.frameLength() as usize;
                let data = buffer.floatChannelData();
                if data.is_null() || frames == 0 {
                    return;
                }
                // One buffer per channel; mixed down to mono.
                let mut mono = vec![0f32; frames];
                for c in 0..channels {
                    let channel = std::slice::from_raw_parts((*data.add(c)).as_ptr(), frames);
                    for (m, s) in mono.iter_mut().zip(channel) {
                        *m += s / channels as f32;
                    }
                }
                push(&mono, 1, &buf, &*on_level);
            });
            input.installTapOnBus_bufferSize_format_block(0, 1024, Some(&format), RcBlock::as_ptr(&block));
            engine.prepare();
            if let Err(e) = engine.startAndReturnError() {
                input.removeTapOnBus(0);
                return Err(anyhow!("starting: {}", e.localizedDescription()));
            }
            Ok((Capture { engine }, rate))
        }
    }
}

/// Box-filter downsampling: averages each output sample's input window, which also
/// acts as a cheap anti-aliasing low-pass. Good enough for speech recognition.
pub fn resample(input: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to || input.is_empty() {
        return input.to_vec();
    }
    let ratio = from as f64 / to as f64;
    let out_len = (input.len() as f64 / ratio).floor() as usize;
    (0..out_len)
        .map(|i| {
            let start = (i as f64 * ratio) as usize;
            let end = (((i + 1) as f64 * ratio) as usize).clamp(start + 1, input.len());
            input[start..end].iter().sum::<f32>() / (end - start) as f32
        })
        .collect()
}
