//! Синтез звуков: шум, синусы, простые фильтры и огибающие. Никаких файлов —
//! всё собирается при запуске за доли секунды. У каждого звука несколько
//! вариантов, чтобы очередь не звучала одной записью по кругу.

use std::f32::consts::TAU;

/// Чем стреляет оружие — «голос» выстрела (`sound` в `data/weapons.ron`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Gun {
    /// Автомат: сухой хлопок и длинный хвост.
    Rifle,
    /// Короткоствол: выше и резче.
    Carbine,
    /// Пистолет-пулемёт: тише и короче.
    Smg,
    /// Гранатомёт: глухое «бумп».
    Launcher,
}

impl Gun {
    pub fn from_name(name: &str) -> Gun {
        match name {
            "carbine" => Gun::Carbine,
            "smg" => Gun::Smg,
            "launcher" => Gun::Launcher,
            _ => Gun::Rifle,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Sfx {
    Shot(Gun),
    Explosion,
    /// Рушится стена.
    Crumble,
    /// Пуля пролетела рядом.
    Whiz,
    Step,
    Reload,
    /// Бросок гранаты.
    Throw,
    /// Точку взяли свои / потеряли.
    Captured,
    Lost,
    /// Конец раунда: победа / поражение.
    Win,
    Lose,
}

impl Sfx {
    /// Все звуки — для банка и для выгрузки в WAV.
    pub const ALL: [Sfx; 14] = [
        Sfx::Shot(Gun::Rifle),
        Sfx::Shot(Gun::Carbine),
        Sfx::Shot(Gun::Smg),
        Sfx::Shot(Gun::Launcher),
        Sfx::Explosion,
        Sfx::Crumble,
        Sfx::Whiz,
        Sfx::Step,
        Sfx::Reload,
        Sfx::Throw,
        Sfx::Captured,
        Sfx::Lost,
        Sfx::Win,
        Sfx::Lose,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Sfx::Shot(Gun::Rifle) => "выстрел-автомат",
            Sfx::Shot(Gun::Carbine) => "выстрел-карабин",
            Sfx::Shot(Gun::Smg) => "выстрел-пп",
            Sfx::Shot(Gun::Launcher) => "выстрел-гранатомёт",
            Sfx::Explosion => "взрыв",
            Sfx::Crumble => "обрушение",
            Sfx::Whiz => "свист-пули",
            Sfx::Step => "шаг",
            Sfx::Reload => "перезарядка",
            Sfx::Throw => "бросок",
            Sfx::Captured => "точка-наша",
            Sfx::Lost => "точка-потеряна",
            Sfx::Win => "победа",
            Sfx::Lose => "поражение",
        }
    }

    /// Сколько вариантов у звука.
    pub fn variants(self) -> u32 {
        match self {
            Sfx::Shot(_) | Sfx::Step => 4,
            Sfx::Explosion | Sfx::Crumble | Sfx::Whiz => 3,
            _ => 1,
        }
    }
}

/// Простой генератор шума (xorshift): у каждого варианта свой.
struct Noise(u32);

impl Noise {
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        (self.0 as f32 / u32::MAX as f32) * 2.0 - 1.0
    }
}

/// Однополюсный фильтр нижних частот: срез `hz`.
struct LowPass {
    a: f32,
    y: f32,
}

impl LowPass {
    fn new(hz: f32, rate: f32) -> Self {
        LowPass { a: 1.0 - (-TAU * hz / rate).exp(), y: 0.0 }
    }

    fn set(&mut self, hz: f32, rate: f32) {
        self.a = 1.0 - (-TAU * hz / rate).exp();
    }

    fn run(&mut self, x: f32) -> f32 {
        self.y += self.a * (x - self.y);
        self.y
    }
}

/// Полосовой фильтр (biquad) с центром `hz` и добротностью `q`.
struct BandPass {
    b0: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    x1: f32,
    x2: f32,
    y1: f32,
    y2: f32,
}

impl BandPass {
    fn new(hz: f32, q: f32, rate: f32) -> Self {
        let mut f = BandPass { b0: 0.0, b2: 0.0, a1: 0.0, a2: 0.0, x1: 0.0, x2: 0.0, y1: 0.0, y2: 0.0 };
        f.set(hz, q, rate);
        f
    }

    fn set(&mut self, hz: f32, q: f32, rate: f32) {
        let w = TAU * hz / rate;
        let alpha = w.sin() / (2.0 * q);
        let a0 = 1.0 + alpha;
        self.b0 = alpha / a0;
        self.b2 = -alpha / a0;
        self.a1 = -2.0 * w.cos() / a0;
        self.a2 = (1.0 - alpha) / a0;
    }

    fn run(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.b2 * self.x2 - self.a1 * self.y1 - self.a2 * self.y2;
        self.x2 = self.x1;
        self.x1 = x;
        self.y2 = self.y1;
        self.y1 = y;
        y
    }
}

/// Экспоненциальное затухание с постоянной `tau` секунд.
fn decay(t: f32, tau: f32) -> f32 {
    (-t / tau).exp()
}

/// Синтез звука `sfx`, вариант `variant`, на частоте дискретизации `rate`.
/// Звук начинается и кончается тишиной — без щелчков — и нормирован по громкости.
pub fn render(sfx: Sfx, variant: u32, rate: u32) -> Vec<f32> {
    let rate_f = rate as f32;
    let mut noise = Noise(0x9E37_79B9 ^ (variant.wrapping_mul(0x85EB_CA6B) + sfx_seed(sfx)));
    // Лёгкий разброс высоты между вариантами.
    let pitch = 1.0 + (variant as f32 - 1.5) * 0.04;
    let (seconds, level) = match sfx {
        Sfx::Shot(Gun::Rifle) => (0.55, 0.95),
        Sfx::Shot(Gun::Carbine) => (0.45, 0.9),
        Sfx::Shot(Gun::Smg) => (0.32, 0.75),
        Sfx::Shot(Gun::Launcher) => (0.5, 0.8),
        Sfx::Explosion => (2.2, 1.0),
        Sfx::Crumble => (1.3, 0.8),
        Sfx::Whiz => (0.32, 0.55),
        Sfx::Step => (0.14, 0.35),
        Sfx::Reload => (0.55, 0.45),
        Sfx::Throw => (0.3, 0.4),
        Sfx::Captured | Sfx::Lost => (0.5, 0.45),
        Sfx::Win | Sfx::Lose => (1.4, 0.5),
    };
    let n = (seconds * rate_f) as usize;
    let mut out = vec![0.0f32; n];
    match sfx {
        Sfx::Shot(gun) => shot(&mut out, gun, pitch, rate_f, &mut noise),
        Sfx::Explosion => explosion(&mut out, pitch, rate_f, &mut noise),
        Sfx::Crumble => crumble(&mut out, rate_f, &mut noise),
        Sfx::Whiz => whiz(&mut out, pitch, rate_f, &mut noise),
        Sfx::Step => step(&mut out, pitch, rate_f, &mut noise),
        Sfx::Reload => reload(&mut out, rate_f, &mut noise),
        Sfx::Throw => throw(&mut out, rate_f, &mut noise),
        Sfx::Captured => tones(&mut out, &[(0.0, 660.0), (0.16, 880.0)], 0.3, rate_f),
        Sfx::Lost => tones(&mut out, &[(0.0, 660.0), (0.16, 494.0)], 0.3, rate_f),
        Sfx::Win => tones(&mut out, &[(0.0, 523.0), (0.14, 659.0), (0.28, 784.0), (0.42, 1046.0)], 0.9, rate_f),
        Sfx::Lose => tones(&mut out, &[(0.0, 440.0), (0.18, 392.0), (0.36, 330.0)], 0.9, rate_f),
    }
    finish(&mut out, level, rate_f);
    out
}

fn sfx_seed(sfx: Sfx) -> u32 {
    Sfx::ALL.iter().position(|&s| s == sfx).unwrap_or(0) as u32 * 7919 + 1
}

/// Мягкие края (1 мс в начале, 20 мс в конце) и нормировка пика до `level`.
fn finish(out: &mut [f32], level: f32, rate: f32) {
    let fade_in = (rate * 0.001) as usize;
    let fade_out = (rate * 0.02) as usize;
    let n = out.len();
    for (i, x) in out.iter_mut().take(fade_in).enumerate() {
        *x *= i as f32 / fade_in as f32;
    }
    for i in 0..fade_out.min(n) {
        out[n - 1 - i] *= i as f32 / fade_out as f32;
    }
    let peak = out.iter().fold(0.0f32, |m, &x| m.max(x.abs()));
    if peak > 0.0 {
        let k = level / peak;
        out.iter_mut().for_each(|x| *x *= k);
    }
}

/// Выстрел: хлопок (высокий шум), «тело» (синус с падающей высотой), середина
/// и хвост — эхо, низкий шум, который тянется дольше.
fn shot(out: &mut [f32], gun: Gun, pitch: f32, rate: f32, noise: &mut Noise) {
    // (частота тела, затухание тела, срез середины, затухание хвоста, громкость хлопка)
    let (body_hz, body_tau, mid_hz, tail_tau, crack) = match gun {
        Gun::Rifle => (105.0, 0.045, 1800.0, 0.17, 1.0),
        Gun::Carbine => (135.0, 0.035, 2400.0, 0.13, 1.0),
        Gun::Smg => (170.0, 0.025, 2600.0, 0.08, 0.8),
        Gun::Launcher => (68.0, 0.11, 500.0, 0.12, 0.25),
    };
    let mut hp_prev = 0.0;
    let mut mid = LowPass::new(mid_hz, rate);
    let mut tail = LowPass::new(650.0, rate);
    let mut phase = 0.0f32;
    for (i, s) in out.iter_mut().enumerate() {
        let t = i as f32 / rate;
        let w = noise.next();
        // Хлопок: шум без низов (разность соседних значений), очень короткий.
        let hp = w - hp_prev;
        hp_prev = w;
        let crack_v = hp * decay(t, 0.006) * crack;
        // Тело: высота падает от 1,8 раза до своей за 30 мс.
        let f = body_hz * pitch * (1.0 + 0.8 * decay(t, 0.03));
        phase += TAU * f / rate;
        let body = phase.sin() * decay(t, body_tau) * 0.9;
        let mid_v = mid.run(w) * decay(t, 0.035) * 1.6;
        let tail_v = tail.run(w) * decay(t, tail_tau) * 1.2;
        *s = crack_v + body + mid_v + tail_v;
    }
}

/// Взрыв: треск, низкий удар с падающей высотой, рокот и осыпающиеся камешки.
fn explosion(out: &mut [f32], pitch: f32, rate: f32, noise: &mut Noise) {
    let mut rumble = LowPass::new(300.0, rate);
    let mut grit = BandPass::new(2200.0, 2.0, rate);
    let mut phase = 0.0f32;
    let mut hp_prev = 0.0;
    for (i, s) in out.iter_mut().enumerate() {
        let t = i as f32 / rate;
        let w = noise.next();
        let hp = w - hp_prev;
        hp_prev = w;
        let crack = hp * decay(t, 0.012);
        let f = 30.0 * pitch * (1.0 + 1.2 * decay(t, 0.12));
        phase += TAU * f / rate;
        let boom = phase.sin() * decay(t, 0.35) * 1.4;
        // Рокот темнеет со временем.
        rumble.set(300.0 * decay(t, 0.8) + 80.0, rate);
        let roar = rumble.run(w) * decay(t, 0.6) * 2.2;
        // Камешки: редкие щелчки через полосовой фильтр, с 0,3 до 1,4 с.
        let pebble = if t > 0.3 && noise.next() > 0.995 { noise.next() * 3.0 } else { 0.0 };
        let grit_v = grit.run(pebble) * decay(t - 0.3, 0.5).min(1.0) * 0.6;
        *s = crack + boom + roar + grit_v;
    }
}

/// Обрушение: шуршащий полосовой шум и много ударов кирпичей вразнобой.
fn crumble(out: &mut [f32], rate: f32, noise: &mut Noise) {
    let mut rustle = BandPass::new(450.0, 1.2, rate);
    let mut bricks = BandPass::new(900.0, 3.0, rate);
    let mut thud = LowPass::new(120.0, rate);
    for (i, s) in out.iter_mut().enumerate() {
        let t = i as f32 / rate;
        let w = noise.next();
        let rustle_v = rustle.run(w) * decay(t, 0.45) * 1.4;
        // Удары: чаще в начале.
        let p = 0.004 * decay(t, 0.5) + 0.0004;
        let hit = if noise.next().abs() < p { noise.next() * 6.0 } else { 0.0 };
        let brick = bricks.run(hit) + thud.run(hit) * 3.0;
        *s = rustle_v + brick * 0.5;
    }
}

/// Свист пули: полосовой шум, центр которого падает (эффект Доплера), огибающая —
/// горб посередине.
fn whiz(out: &mut [f32], pitch: f32, rate: f32, noise: &mut Noise) {
    let n = out.len() as f32;
    let mut band = BandPass::new(4000.0, 6.0, rate);
    for (i, s) in out.iter_mut().enumerate() {
        let k = i as f32 / n;
        band.set((4200.0 - 2800.0 * k) * pitch, 6.0, rate);
        let env = (std::f32::consts::PI * k).sin().powi(2);
        *s = band.run(noise.next()) * env;
    }
}

/// Шаг: глухой удар и немного хруста.
fn step(out: &mut [f32], pitch: f32, rate: f32, noise: &mut Noise) {
    let mut grit = LowPass::new(900.0 * pitch, rate);
    let mut phase = 0.0f32;
    for (i, s) in out.iter_mut().enumerate() {
        let t = i as f32 / rate;
        phase += TAU * 75.0 * pitch / rate;
        *s = phase.sin() * decay(t, 0.025) + grit.run(noise.next()) * decay(t, 0.02) * 1.5;
    }
}

/// Перезарядка: два металлических щелчка — магазин вынули и вставили.
fn reload(out: &mut [f32], rate: f32, noise: &mut Noise) {
    let mut hp_prev = 0.0;
    for (i, s) in out.iter_mut().enumerate() {
        let t = i as f32 / rate;
        let w = noise.next();
        let hp = w - hp_prev;
        hp_prev = w;
        let mut v = 0.0;
        for (at, ring) in [(0.0, 2300.0), (0.33, 2900.0)] {
            let dt = t - at;
            if dt >= 0.0 {
                v += hp * decay(dt, 0.004) + (TAU * ring * dt).sin() * decay(dt, 0.018) * 0.5;
            }
        }
        *s = v;
    }
}

/// Бросок: свист воздуха — полосовой шум с горбом.
fn throw(out: &mut [f32], rate: f32, noise: &mut Noise) {
    let n = out.len() as f32;
    let mut band = BandPass::new(900.0, 1.5, rate);
    for (i, s) in out.iter_mut().enumerate() {
        let k = i as f32 / n;
        band.set(1100.0 - 600.0 * k, 1.5, rate);
        *s = band.run(noise.next()) * (std::f32::consts::PI * k).sin();
    }
}

/// Сигнал: ноты (начало, частота) с мягкой атакой, вторая гармоника для теплоты.
fn tones(out: &mut [f32], notes: &[(f32, f32)], length: f32, rate: f32) {
    for (i, s) in out.iter_mut().enumerate() {
        let t = i as f32 / rate;
        let mut v = 0.0;
        for &(at, hz) in notes {
            let dt = t - at;
            if dt < 0.0 {
                continue;
            }
            let env = (dt / 0.01).min(1.0) * decay(dt, length * 0.35);
            v += ((TAU * hz * dt).sin() + 0.3 * (TAU * 2.0 * hz * dt).sin()) * env;
        }
        *s = v;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_sound_is_clean() {
        for sfx in Sfx::ALL {
            for v in 0..sfx.variants() {
                let s = render(sfx, v, 48_000);
                assert!(!s.is_empty(), "{sfx:?}: пусто");
                assert!(s.iter().all(|x| x.is_finite()), "{sfx:?}: NaN");
                let peak = s.iter().fold(0.0f32, |m, &x| m.max(x.abs()));
                assert!(peak <= 1.0 && peak > 0.2, "{sfx:?}: пик {peak}");
                // Без щелчка: первые и последние отсчёты — почти тишина.
                assert!(s[0].abs() < 0.05 && s[s.len() - 1].abs() < 0.01, "{sfx:?}: щелчок на краю");
            }
        }
    }

    #[test]
    fn variants_differ_and_repeat() {
        let a = render(Sfx::Shot(Gun::Rifle), 0, 48_000);
        let b = render(Sfx::Shot(Gun::Rifle), 1, 48_000);
        assert_ne!(a, b, "варианты должны различаться");
        assert_eq!(a, render(Sfx::Shot(Gun::Rifle), 0, 48_000), "один вариант — всегда один звук");
    }

    #[test]
    fn guns_sound_different() {
        // Хвост автомата длиннее, чем у пистолета-пулемёта.
        let energy_after = |s: &[f32], from: f32| s[(from * 48_000.0) as usize..].iter().map(|x| x * x).sum::<f32>();
        let rifle = render(Sfx::Shot(Gun::Rifle), 0, 48_000);
        let smg = render(Sfx::Shot(Gun::Smg), 0, 48_000);
        assert!(energy_after(&rifle, 0.15) > energy_after(&smg, 0.15) * 2.0);
    }
}
