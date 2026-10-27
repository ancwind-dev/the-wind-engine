//! Микшер: банк готовых звуков, голоса с громкостью, панорамой и задержкой,
//! ограничитель на выходе. Сам микшер не знает про звуковую карту — его можно
//! гонять в тестах и выгружать в WAV.

use crate::synth::{self, Sfx};
use glam::Vec2;
use std::collections::HashMap;
use std::f32::consts::FRAC_PI_4;
use std::sync::Arc;

/// Сколько голосов звучит одновременно. Лишние — самые тихие — отбрасываем.
pub const MAX_VOICES: usize = 64;
/// Скорость звука, м/с: дальний выстрел слышно позже вспышки.
pub const SPEED_OF_SOUND: f32 = 343.0;
/// Потолок выхода: ограничитель не даёт сумме голосов уйти выше.
const CEILING: f32 = 0.9;

/// Все звуки со всеми вариантами, посчитанные на частоте вывода.
pub struct Bank {
    sounds: HashMap<Sfx, Vec<Arc<[f32]>>>,
    pub rate: u32,
}

impl Bank {
    pub fn build(rate: u32) -> Bank {
        let sounds = Sfx::ALL
            .iter()
            .map(|&sfx| (sfx, (0..sfx.variants()).map(|v| Arc::from(synth::render(sfx, v, rate))).collect()))
            .collect();
        Bank { sounds, rate }
    }

    pub fn get(&self, sfx: Sfx, variant: u32) -> Option<&Arc<[f32]>> {
        let all = self.sounds.get(&sfx)?;
        all.get(variant as usize % all.len().max(1))
    }
}

/// Что сыграть.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Play {
    pub sfx: Sfx,
    /// Вариант звука (берётся по модулю числа вариантов).
    pub variant: u32,
    pub gain: f32,
    /// −1 — слева, 0 — по центру, 1 — справа.
    pub pan: f32,
    /// Через сколько секунд начать.
    pub delay: f32,
}

impl Play {
    /// Звук «у уха»: интерфейс, свои шаги.
    pub fn flat(sfx: Sfx, variant: u32, gain: f32) -> Play {
        Play { sfx, variant, gain, pan: 0.0, delay: 0.0 }
    }
}

/// Где звук относительно слушателя: громкость, панорама, задержка. `range` —
/// на каком расстоянии звук становится тише вдвое; дальше `range * 8` — не слышно.
/// `None` — слишком далеко, играть незачем.
pub fn spatial(listener: Vec2, right: Vec2, source: Vec2, range: f32) -> Option<(f32, f32, f32)> {
    let to = source - listener;
    let d = to.length();
    if d > range * 8.0 {
        return None;
    }
    // Мягкое затухание: 1 вблизи, вдвое тише на `range`, плавно к нулю на краю.
    let edge = 1.0 - (d / (range * 8.0)).powi(2);
    let gain = edge / (1.0 + d / range);
    // Панорама: вблизи — почти по центру, иначе звук над головой «прыгает».
    let side = if d > 1e-3 { to.dot(right) / d } else { 0.0 };
    let pan = side * (d / 6.0).min(1.0) * 0.85;
    Some((gain, pan, d / SPEED_OF_SOUND))
}

struct Voice {
    samples: Arc<[f32]>,
    /// Позиция в отсчётах; отрицательная — ещё ждём задержку.
    pos: isize,
    left: f32,
    right: f32,
}

impl Voice {
    fn loudness(&self) -> f32 {
        self.left.max(self.right)
    }
}

pub struct Mixer {
    bank: Bank,
    voices: Vec<Voice>,
    pub volume: f32,
    /// Усиление ограничителя (≤ 1).
    limit: f32,
    release: f32,
}

impl Mixer {
    pub fn new(bank: Bank, volume: f32) -> Mixer {
        // Ограничитель отпускает за ~150 мс.
        let release = 1.0 - (-1.0 / (0.15 * bank.rate as f32)).exp();
        Mixer { bank, voices: Vec::with_capacity(MAX_VOICES), volume, limit: 1.0, release }
    }

    pub fn voices(&self) -> usize {
        self.voices.len()
    }

    pub fn play(&mut self, p: Play) {
        let Some(samples) = self.bank.get(p.sfx, p.variant) else { return };
        if p.gain.is_nan() || p.gain <= 1e-4 {
            return;
        }
        // Равная мощность: по центру каждый канал −3 дБ, сумма мощностей постоянна.
        let angle = (p.pan.clamp(-1.0, 1.0) + 1.0) * FRAC_PI_4;
        let voice = Voice {
            samples: samples.clone(),
            pos: -((p.delay.max(0.0) * self.bank.rate as f32) as isize),
            left: p.gain * angle.cos(),
            right: p.gain * angle.sin(),
        };
        if self.voices.len() < MAX_VOICES {
            self.voices.push(voice);
            return;
        }
        // Мест нет: заменяем самый тихий голос, если новый громче.
        let (i, quietest) = self
            .voices
            .iter()
            .enumerate()
            .min_by(|a, b| a.1.loudness().total_cmp(&b.1.loudness()))
            .map(|(i, v)| (i, v.loudness()))
            .unwrap();
        if voice.loudness() > quietest {
            self.voices[i] = voice;
        }
    }

    /// Смешивает в `out` (стерео, L R L R …), затирая прежнее.
    pub fn mix(&mut self, out: &mut [f32]) {
        out.iter_mut().for_each(|x| *x = 0.0);
        let frames = out.len() / 2;
        for v in &mut self.voices {
            let n = v.samples.len() as isize;
            for f in 0..frames {
                let i = v.pos + f as isize;
                if i < 0 {
                    continue;
                }
                if i >= n {
                    break;
                }
                let s = v.samples[i as usize];
                out[f * 2] += s * v.left;
                out[f * 2 + 1] += s * v.right;
            }
            v.pos += frames as isize;
        }
        self.voices.retain(|v| v.pos < v.samples.len() as isize);
        // Ограничитель: мгновенно прижимает пик к потолку и плавно отпускает.
        for frame in out.chunks_mut(2) {
            let peak = frame[0].abs().max(frame[1].abs()) * self.volume;
            self.limit += (1.0 - self.limit) * self.release;
            if peak * self.limit > CEILING {
                self.limit = CEILING / peak;
            }
            let g = self.volume * self.limit;
            for x in frame {
                *x = (*x * g).clamp(-1.0, 1.0);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::synth::Gun;

    const RATE: u32 = 8_000;

    fn mixer() -> Mixer {
        Mixer::new(Bank::build(RATE), 1.0)
    }

    fn render(m: &mut Mixer, seconds: f32) -> Vec<f32> {
        let mut out = vec![0.0; (seconds * RATE as f32) as usize * 2];
        // Кусками, как в настоящем потоке.
        for chunk in out.chunks_mut(512) {
            m.mix(chunk);
        }
        out
    }

    #[test]
    fn pan_puts_sound_on_one_side() {
        let mut m = mixer();
        m.play(Play { pan: 1.0, ..Play::flat(Sfx::Shot(Gun::Rifle), 0, 0.5) });
        let out = render(&mut m, 0.3);
        let left: f32 = out.iter().step_by(2).map(|x| x * x).sum();
        let right: f32 = out.iter().skip(1).step_by(2).map(|x| x * x).sum();
        assert!(right > 0.1 && left < right * 1e-6, "слева {left}, справа {right}");
    }

    #[test]
    fn delay_starts_later_and_voices_finish() {
        let mut m = mixer();
        m.play(Play { delay: 0.2, ..Play::flat(Sfx::Step, 0, 0.5) });
        let out = render(&mut m, 0.5);
        let start = out.iter().position(|x| x.abs() > 1e-4).unwrap() / 2;
        assert!((start as f32 / RATE as f32 - 0.2).abs() < 0.01, "начало {start}");
        // Шаг короткий: к концу голос освободился.
        assert_eq!(m.voices(), 0);
    }

    #[test]
    fn many_loud_sounds_do_not_clip() {
        let mut m = mixer();
        for i in 0..200 {
            m.play(Play::flat(Sfx::Explosion, i, 1.0));
        }
        assert_eq!(m.voices(), MAX_VOICES);
        let out = render(&mut m, 1.0);
        assert!(out.iter().all(|x| x.is_finite() && x.abs() <= CEILING + 1e-3));
    }

    #[test]
    fn quiet_voices_give_way_to_loud() {
        let mut m = mixer();
        for i in 0..MAX_VOICES as u32 {
            m.play(Play::flat(Sfx::Explosion, i, 0.01));
        }
        m.play(Play::flat(Sfx::Win, 0, 0.8));
        assert!(m.voices.iter().any(|v| v.loudness() > 0.5), "громкий звук должен вытеснить тихий");
        // А тихий не вытесняет громкий.
        let mut m = mixer();
        for i in 0..MAX_VOICES as u32 {
            m.play(Play::flat(Sfx::Explosion, i, 0.8));
        }
        m.play(Play::flat(Sfx::Step, 0, 0.01));
        assert!(m.voices.iter().all(|v| v.loudness() > 0.5));
    }

    #[test]
    fn far_sounds_are_quieter_later_and_to_the_side() {
        let me = Vec2::ZERO;
        let right = Vec2::X;
        let (near, _, _) = spatial(me, right, Vec2::new(0.0, 5.0), 20.0).unwrap();
        let (far, pan, delay) = spatial(me, right, Vec2::new(100.0, 0.0), 20.0).unwrap();
        assert!(near > far * 3.0);
        assert!(pan > 0.8, "справа: {pan}");
        assert!((delay - 100.0 / SPEED_OF_SOUND).abs() < 1e-4);
        assert!(spatial(me, right, Vec2::new(-200.0, 0.0), 20.0).is_none());
    }
}
