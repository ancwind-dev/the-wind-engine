//! Раунд. Бой идёт, пока одна команда не удержит все точки `hold_time` секунд
//! или не выйдет время — тогда побеждает та, у кого больше точек (поровну — ничья).
//! Потом перерыв с итогами и новый раунд на той же карте: стены, точки и счёт —
//! как в начале. Числа — в `sim.ron`, раздел `round`.

use serde::Deserialize;

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct RoundSpec {
    /// Сколько секунд нужно удерживать все точки, чтобы победить.
    pub hold_time: f32,
    /// Длина раунда, с. 0 — без ограничения.
    pub time_limit: f32,
    /// Перерыв с итогами между раундами, с.
    pub intermission: f32,
}

impl Default for RoundSpec {
    fn default() -> Self {
        RoundSpec { hold_time: 60.0, time_limit: 1200.0, intermission: 15.0 }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EndReason {
    /// Команда удержала все точки.
    AllPoints,
    /// Вышло время.
    TimeUp,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Playing,
    /// Раунд кончился: победитель (`None` — ничья) и почему. Идёт перерыв.
    Over { winner: Option<u8>, reason: EndReason },
}

#[derive(Clone, Debug)]
pub struct Round {
    /// Номер раунда, с 1.
    pub number: u32,
    pub phase: Phase,
    /// Сколько секунд идёт текущая фаза (бой или перерыв).
    pub time: f32,
    /// Сколько секунд каждая команда без перерыва держит все точки.
    pub hold: [f32; 2],
}

impl Default for Round {
    fn default() -> Self {
        Round { number: 1, phase: Phase::Playing, time: 0.0, hold: [0.0; 2] }
    }
}

impl Round {
    pub fn playing(&self) -> bool {
        self.phase == Phase::Playing
    }

    /// Кто победил, когда вышло время: у кого больше точек.
    pub fn time_up_winner(owned: [u32; 2]) -> Option<u8> {
        match owned[0].cmp(&owned[1]) {
            std::cmp::Ordering::Greater => Some(0),
            std::cmp::Ordering::Less => Some(1),
            std::cmp::Ordering::Equal => None,
        }
    }
}
