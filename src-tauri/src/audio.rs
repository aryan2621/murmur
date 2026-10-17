//! Microphone capture. The stream lives on its own thread (cpal streams are not `Send`)
//! and is only open while the hotkey is held, so the OS mic indicator is off otherwise.

use anyhow::{anyhow, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, Sample, SampleFormat, SizedSample};
use std::any::Any;
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use nnnoiseless::{DenoiseState, FRAME_SIZE};

/// RNNoise works on 48 kHz mono, 10 ms (480 samples) at a time.
const DENOISE_RATE: u32 = 48_000;
pub const TARGET_RATE: u32 = 16_000;
const LEVEL_INTERVAL: Duration = Duration::from_millis(33);

pub type LevelFn = Box<dyn Fn(f32) + Send + Sync + 'static>;

/// How the mic audio is cleaned up before anyone hears it.
#[derive(Debug, Clone, Copy)]
pub struct Processing {
    /// Remove noise that isn't speech (RNNoise).
    pub denoise: bool,
    /// Remove what the Mac itself is playing (macOS voice processing; system default mic only).
    pub echo: bool,
}

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
    Start(Option<String>, Processing, LevelFn, mpsc::Sender<Result<()>>),
    Stop(mpsc::Sender<Result<Recording>>),
    Peek(Duration, mpsc::Sender<Option<Tail>>),
}

/// The newest audio of a recording that is still running.
pub struct Tail {
    /// Mono, 16 kHz.
    pub samples: Vec<f32>,
    /// How much has been recorded so far, in 16 kHz samples; tells callers what is new.
    pub total: usize,
}

struct Buffer {
    samples: Vec<f32>,
    peak_rms: f32,
    window_peak: f32,
    last_emit: Instant,
    /// Strips everything but speech from the mic audio as it arrives (`None`: audio kept as is).
    denoiser: Option<Denoiser>,
}

impl Buffer {
    fn new(capacity: usize, denoiser: Option<Denoiser>) -> Self {
        Self { samples: Vec::with_capacity(capacity), peak_rms: 0.0, window_peak: 0.0, last_emit: Instant::now(), denoiser }
    }
}

/// Streaming noise suppression (RNNoise, the same idea as Teams' "noise cancellation"): a small
/// neural network that keeps voices and removes fans, hum, keyboards and the like. It needs 48 kHz,
/// so the mic audio is resampled up first and the clean audio is stored at 48 kHz.
struct Denoiser {
    state: Box<DenoiseState<'static>>,
    /// Mic samples per 48 kHz sample.
    step: f64,
    /// Where the next 48 kHz sample falls between `prev` (0) and the next mic sample (1).
    pos: f64,
    prev: f32,
    /// 48 kHz audio waiting for a whole frame.
    pending: Vec<f32>,
    /// RNNoise's first output frame is a fade-in artifact; it is dropped.
    first: bool,
}

impl Denoiser {
    fn new(mic_rate: u32) -> Self {
        Self { state: DenoiseState::new(), step: mic_rate as f64 / DENOISE_RATE as f64, pos: 0.0, prev: 0.0, pending: Vec::with_capacity(FRAME_SIZE * 2), first: true }
    }

    /// Feeds mono mic samples and appends the cleaned 48 kHz audio to `out`.
    fn process(&mut self, mono: &[f32], out: &mut Vec<f32>) {
        for &x in mono {
            while self.pos < 1.0 {
                self.pending.push((self.prev + (x - self.prev) * self.pos as f32) * 32768.0);
                self.pos += self.step;
            }
            self.pos -= 1.0;
            self.prev = x;
        }
        let mut clean = [0.0f32; FRAME_SIZE];
        let whole = self.pending.len() / FRAME_SIZE * FRAME_SIZE;
        for frame in self.pending[..whole].chunks(FRAME_SIZE) {
            self.state.process_frame(&mut clean, frame);
            if self.first {
                self.first = false;
                continue;
            }
            out.extend(clean.iter().map(|s| s / 32768.0));
        }
        self.pending.drain(..whole);
    }
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

    /// Starts recording from `device` (input device name, or `None` for the system default) and
    /// waits until the mic is open.
    pub fn start(&self, device: Option<String>, processing: Processing, on_level: LevelFn) -> Result<()> {
        self.start_async(device, processing, on_level).recv()?
    }

    /// Like `start`, but returns at once with where the outcome will arrive. Opening the mic takes
    /// a moment; the hotkey's own thread must not wait for it, or a quick tap's release is only
    /// seen after that and taken for a hold. Commands are handled in order, so a `stop` sent
    /// meanwhile still comes after this start.
    pub fn start_async(&self, device: Option<String>, processing: Processing, on_level: LevelFn) -> mpsc::Receiver<Result<()>> {
        let (reply, wait) = mpsc::channel();
        if let Err(e) = self.tx.send(Cmd::Start(device, processing, on_level, reply.clone())) {
            let _ = reply.send(Err(anyhow!("the audio thread has stopped: {e}")));
        }
        wait
    }

    /// The last `span` of audio while recording (`None` when not recording).
    pub fn peek(&self, span: Duration) -> Option<Tail> {
        let (reply, wait) = mpsc::channel();
        self.tx.send(Cmd::Peek(span, reply)).ok()?;
        wait.recv().ok()?
    }

    pub fn stop(&self) -> Result<Recording> {
        let (reply, wait) = mpsc::channel();
        self.tx.send(Cmd::Stop(reply))?;
        wait.recv()?
    }
}

fn audio_thread(rx: mpsc::Receiver<Cmd>) {
    // The open mic (a cpal stream or the voice-processing engine; dropping it closes the mic),
    // its buffer and the buffer's sample rate.
    let mut active: Option<(Box<dyn Any>, Arc<Mutex<Buffer>>, u32)> = None;
    for cmd in rx {
        match cmd {
            Cmd::Start(device, processing, on_level, reply) => {
                active = None;
                let result = open_mic(device.as_deref(), processing, Arc::new(on_level)).map(|a| active = Some(a));
                let _ = reply.send(result);
            }
            Cmd::Stop(reply) => {
                let result = match active.take() {
                    Some((stream, buf, rate)) => {
                        drop(stream);
                        let buf = std::mem::replace(
                            &mut *buf.lock().unwrap(),
                            Buffer::new(0, None),
                        );
                        Ok(Recording { samples: resample(&buf.samples, rate, TARGET_RATE), peak_rms: buf.peak_rms })
                    }
                    None => Err(anyhow!("not recording")),
                };
                let _ = reply.send(result);
            }
            Cmd::Peek(span, reply) => {
                let tail = active.as_ref().map(|(_, buf, rate)| {
                    // Copy and let go at once: the mic callback waits on this lock.
                    let (raw, len) = {
                        let b = buf.lock().unwrap();
                        let want = (span.as_secs_f64() * *rate as f64) as usize;
                        (b.samples[b.samples.len().saturating_sub(want)..].to_vec(), b.samples.len())
                    };
                    Tail { samples: resample(&raw, *rate, TARGET_RATE), total: (len as u64 * TARGET_RATE as u64 / *rate as u64) as usize }
                });
                let _ = reply.send(tail);
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

fn open_mic(device: Option<&str>, processing: Processing, on_level: Arc<LevelFn>) -> Result<(Box<dyn Any>, Arc<Mutex<Buffer>>, u32)> {
    #[cfg(target_os = "macos")]
    if processing.echo {
        // Voice processing always uses the system default mic.
        if device.is_some() {
            mlog!("echo cancellation needs the system default microphone; recording without it");
        } else {
            match echo::open(processing.denoise, on_level.clone()) {
                Ok((engine, buf, rate)) => return Ok((Box::new(engine), buf, rate)),
                Err(e) => mlog!("echo cancellation unavailable ({e}); recording without it"),
            }
        }
    }
    let (stream, buf, rate) = open_stream(device, processing.denoise, on_level)?;
    Ok((Box::new(stream), buf, rate))
}

fn open_stream(device: Option<&str>, denoise: bool, on_level: Arc<LevelFn>) -> Result<(cpal::Stream, Arc<Mutex<Buffer>>, u32)> {
    let device = find_device(device).ok_or_else(|| anyhow!("No microphone found"))?;
    let supported = device.default_input_config()?;
    let rate = supported.sample_rate();
    let channels = supported.channels() as usize;
    let config = supported.config();

    // With noise suppression the buffer holds 48 kHz audio, otherwise the mic's own rate.
    let stored_rate = if denoise { DENOISE_RATE } else { rate };
    let denoiser = denoise.then(|| Denoiser::new(rate));
    let buf = Arc::new(Mutex::new(Buffer::new(stored_rate as usize * 30, denoiser)));
    let b = buf.clone();
    let err = |e| mlog!("audio stream error: {e}");

    let stream = match supported.sample_format() {
        SampleFormat::F32 => device.build_input_stream(config, move |d: &[f32], _: &_| push(d, channels, &b, &**on_level), err, None)?,
        SampleFormat::I16 => device.build_input_stream(config, move |d: &[i16], _: &_| push(d, channels, &b, &**on_level), err, None)?,
        SampleFormat::I32 => device.build_input_stream(config, move |d: &[i32], _: &_| push(d, channels, &b, &**on_level), err, None)?,
        SampleFormat::U16 => device.build_input_stream(config, move |d: &[u16], _: &_| push(d, channels, &b, &**on_level), err, None)?,
        other => return Err(anyhow!("Unsupported microphone sample format {other}")),
    };
    stream.play()?;
    Ok((stream, buf, stored_rate))
}

fn push<T>(data: &[T], channels: usize, buf: &Mutex<Buffer>, on_level: &dyn Fn(f32))
where
    T: Sample + SizedSample,
    f32: FromSample<T>,
{
    let mono: Vec<f32> = data.chunks(channels).map(|frame| frame.iter().map(|s| f32::from_sample(*s)).sum::<f32>() / channels as f32).collect();
    if mono.is_empty() {
        return;
    }
    let mut b = buf.lock().unwrap();
    let start = b.samples.len();
    match b.denoiser.take() {
        Some(mut d) => {
            let mut clean = Vec::with_capacity(mono.len() * 2);
            d.process(&mono, &mut clean);
            b.samples.extend_from_slice(&clean);
            b.denoiser = Some(d);
        }
        None => b.samples.extend_from_slice(&mono),
    }
    let added = &b.samples[start..];
    if added.is_empty() {
        return;
    }
    let rms = (added.iter().map(|s| s * s).sum::<f32>() / added.len() as f32).sqrt();
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

/// Echo cancellation: macOS's voice processing (what FaceTime uses) subtracts whatever the Mac is
/// playing from the mic, so Jarvis's own voice, music and videos don't end up in what you said.
#[cfg(target_os = "macos")]
mod echo {
    use super::{push, Buffer, Denoiser, LevelFn, DENOISE_RATE};
    use anyhow::{anyhow, Result};
    use objc2::exception::catch;
    use objc2::rc::Retained;
    use objc2::runtime::Bool;
    use objc2::sel;
    use objc2::AnyThread;
    use objc2_avf_audio::{AVAudioEngine, AVAudioSinkNode, AVAudioVoiceProcessingOtherAudioDuckingConfiguration, AVAudioVoiceProcessingOtherAudioDuckingLevel};
    use objc2_core_audio_types::{AudioBufferList, AudioTimeStamp};
    use objc2_foundation::NSObjectProtocol;
    use std::panic::AssertUnwindSafe;
    use std::ptr::NonNull;
    use std::sync::{Arc, Mutex};

    /// The running engine; dropping it closes the mic.
    pub struct Engine(Retained<AVAudioEngine>);

    impl Drop for Engine {
        fn drop(&mut self) {
            unsafe { self.0.stop() }
        }
    }

    pub fn open(denoise: bool, on_level: Arc<LevelFn>) -> Result<(Engine, Arc<Mutex<Buffer>>, u32)> {
        unsafe {
            let engine = AVAudioEngine::new();
            let input = engine.inputNode();
            input.setVoiceProcessingEnabled_error(true).map_err(|e| anyhow!("{}", e.localizedDescription()))?;
            // Voice processing turns other apps down while the mic is open; keep that to a minimum
            // (macOS 14+; older versions duck at their default level).
            if input.respondsToSelector(sel!(setVoiceProcessingOtherAudioDuckingConfiguration:)) {
                input.setVoiceProcessingOtherAudioDuckingConfiguration(AVAudioVoiceProcessingOtherAudioDuckingConfiguration {
                    enableAdvancedDucking: Bool::NO,
                    duckingLevel: AVAudioVoiceProcessingOtherAudioDuckingLevel::Min,
                });
            }
            let format = input.outputFormatForBus(0);
            let rate = format.sampleRate() as u32;
            if rate == 0 || format.channelCount() == 0 {
                return Err(anyhow!("the voice-processing mic has no audio format"));
            }
            let stored_rate = if denoise { DENOISE_RATE } else { rate };
            let buf = Arc::new(Mutex::new(Buffer::new(stored_rate as usize * 30, denoise.then(|| Denoiser::new(rate)))));
            let b = buf.clone();
            // A sink node gets the mic every render cycle (~10 ms), so the level meter moves smoothly;
            // a tap would deliver it in 100 ms+ batches.
            let receive = block2::RcBlock::new(move |_: NonNull<AudioTimeStamp>, frames: u32, list: NonNull<AudioBufferList>| -> i32 {
                let list = list.as_ref();
                let first = &list.mBuffers[0];
                if list.mNumberBuffers == 0 || first.mData.is_null() {
                    return 0;
                }
                // Non-interleaved float: the first buffer is the processed mic's first channel.
                let len = (frames as usize).min(first.mDataByteSize as usize / 4 / first.mNumberChannels.max(1) as usize);
                let samples = std::slice::from_raw_parts(first.mData as *const f32, len * first.mNumberChannels.max(1) as usize);
                push(samples, first.mNumberChannels.max(1) as usize, &b, &**on_level);
                0
            });
            let sink = AVAudioSinkNode::initWithReceiverBlock(AVAudioSinkNode::alloc(), &*receive as *const _ as *mut _);
            // AVAudioEngine reports misuse by raising Objective-C exceptions; turn them into errors.
            catch(AssertUnwindSafe(|| {
                engine.attachNode(&sink);
                engine.connect_to_format(&input, &sink, Some(&format));
            }))
            .map_err(|e| anyhow!("could not listen: {e:?}"))?;
            let started = catch(AssertUnwindSafe(|| engine.startAndReturnError()));
            if !matches!(started, Ok(Ok(()))) {
                return Err(match started {
                    Ok(Err(e)) => anyhow!("could not start: {}", e.localizedDescription()),
                    Err(e) => anyhow!("could not start: {e:?}"),
                    Ok(Ok(())) => unreachable!(),
                });
            }
            Ok((Engine(engine), buf, stored_rate))
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
