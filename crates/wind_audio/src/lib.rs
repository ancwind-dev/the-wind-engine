//! Звук игры: синтез звуков ([`synth`]), микшер голосов ([`mixer`]) и вывод
//! через cpal ([`Audio`]). Нет звуковой карты — игра идёт молча.

pub mod mixer;
pub mod synth;
pub mod wav;

pub use mixer::{Bank, Mixer, Play};
pub use synth::{Gun, Sfx};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SizedSample};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{Sender, channel};

/// Вывод звука. Команды уходят в поток звуковой карты через канал — игра
/// не ждёт и не блокируется.
pub struct Audio {
    tx: Option<Sender<Play>>,
    _stream: Option<cpal::Stream>,
    /// Частота вывода, Гц (для задержек по расстоянию).
    pub rate: u32,
    meter: Arc<Meter>,
}

/// Что сейчас в потоке звука — для отладки (F3): голоса и пик выхода.
#[derive(Default)]
struct Meter {
    voices: AtomicU32,
    /// Пик за последний кусок, f32 битами.
    peak: AtomicU32,
}

impl Audio {
    /// Открывает устройство по умолчанию; `volume` — общая громкость 0..1.
    pub fn open(volume: f32) -> Audio {
        match Self::try_open(volume) {
            Ok(a) => a,
            Err(e) => {
                eprintln!("звук отключён: {e}");
                Audio::silent()
            }
        }
    }

    /// Без звука (флаг `--mute` или нет устройства).
    pub fn silent() -> Audio {
        Audio { tx: None, _stream: None, rate: 48_000, meter: Arc::default() }
    }

    fn try_open(volume: f32) -> Result<Audio, String> {
        let host = cpal::default_host();
        let device = host.default_output_device().ok_or("нет устройства вывода")?;
        let config = device.default_output_config().map_err(|e| e.to_string())?;
        let format = config.sample_format();
        let config: cpal::StreamConfig = config.into();
        let rate = config.sample_rate;
        let channels = config.channels as usize;
        let mixer = Mixer::new(Bank::build(rate), volume);
        let (tx, rx) = channel();
        let meter = Arc::new(Meter::default());
        let m = meter.clone();
        let stream = match format {
            cpal::SampleFormat::F32 => build::<f32>(&device, &config, channels, mixer, rx, m),
            cpal::SampleFormat::I16 => build::<i16>(&device, &config, channels, mixer, rx, m),
            cpal::SampleFormat::U16 => build::<u16>(&device, &config, channels, mixer, rx, m),
            cpal::SampleFormat::I32 => build::<i32>(&device, &config, channels, mixer, rx, m),
            other => return Err(format!("формат {other} не поддержан")),
        }?;
        stream.play().map_err(|e| e.to_string())?;
        Ok(Audio { tx: Some(tx), _stream: Some(stream), rate, meter })
    }

    pub fn play(&self, play: Play) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(play);
        }
    }

    pub fn is_on(&self) -> bool {
        self.tx.is_some()
    }

    /// Сколько голосов звучит и пик выхода (0..1).
    pub fn meter(&self) -> (u32, f32) {
        (self.meter.voices.load(Ordering::Relaxed), f32::from_bits(self.meter.peak.load(Ordering::Relaxed)))
    }
}

fn build<T: SizedSample + FromSample<f32>>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    channels: usize,
    mut mixer: Mixer,
    rx: std::sync::mpsc::Receiver<Play>,
    meter: Arc<Meter>,
) -> Result<cpal::Stream, String> {
    let mut stereo = Vec::new();
    device
        .build_output_stream(
            *config,
            move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
                while let Ok(p) = rx.try_recv() {
                    mixer.play(p);
                }
                let frames = data.len() / channels.max(1);
                stereo.resize(frames * 2, 0.0);
                mixer.mix(&mut stereo);
                let peak = stereo.iter().fold(0.0f32, |m, x| m.max(x.abs()));
                meter.voices.store(mixer.voices() as u32, Ordering::Relaxed);
                meter.peak.store(peak.to_bits(), Ordering::Relaxed);
                for (frame, lr) in data.chunks_mut(channels.max(1)).zip(stereo.chunks(2)) {
                    for (c, out) in frame.iter_mut().enumerate() {
                        // Моно — среднее, стерео — как есть, остальные каналы — тишина.
                        let v = match (channels, c) {
                            (1, _) => (lr[0] + lr[1]) * 0.5,
                            (_, 0) => lr[0],
                            (_, 1) => lr[1],
                            _ => 0.0,
                        };
                        *out = T::from_sample(v);
                    }
                }
            },
            |e| eprintln!("звук: {e}"),
            None,
        )
        .map_err(|e| e.to_string())
}
