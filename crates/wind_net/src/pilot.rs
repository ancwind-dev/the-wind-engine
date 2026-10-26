//! Свой солдат на клиенте: двигается сразу по вводу, без ожидания сервера.
//!
//! Клиент шлёт, где его солдат (`PlayerInput::pos`), а сервер подтягивает солдата
//! туда не быстрее `CLAIM_CATCH_UP` скоростей и не сквозь стены. Если сервер
//! не согласен больше чем на `CLAIM_MAX_GAP` (возрождение, застряли, подделка),
//! солдат возвращается туда, где он по серверу. Подробнее — решение
//! «Управление — клиент ведёт своего солдата».

use wind_sim::map::Map;
use wind_sim::{CLAIM_MAX_GAP, Vec2, walk};

/// Сколько стоять на месте, прежде чем подтянуться к серверу, с. За это время
/// сервер точно догоняет заявку — если не догнал, значит упёрся во что-то.
const SETTLE_TIME: f32 = 1.0;

#[derive(Default)]
pub struct Pilot {
    pos: Option<Vec2>,
    /// Сколько секунд солдат стоит.
    still: f32,
    /// Сколько раз пришлось вернуть солдата на место сервера — для отладки.
    pub snaps: u32,
}

impl Pilot {
    /// Кадр. `server` — где солдат по серверу (последний снимок или своя симуляция);
    /// `None` — солдат мёртв или его не видно. Возвращает, где рисовать своего
    /// солдата и что заявить серверу.
    pub fn update(&mut self, map: &Map, server: Option<Vec2>, move_dir: Vec2, speed: f32, radius: f32, dt: f32) -> Option<Vec2> {
        let Some(server) = server else {
            self.pos = None;
            return None;
        };
        let mut pos = match self.pos {
            Some(p) if p.distance(server) < CLAIM_MAX_GAP => p,
            // Первый кадр, возрождение или сервер не согласен — встаём туда, где он.
            Some(_) => {
                self.snaps += 1;
                server
            }
            None => server,
        };
        let dir = move_dir.clamp_length_max(1.0);
        if dir == Vec2::ZERO {
            self.still += dt;
            if self.still > SETTLE_TIME {
                pos += (server - pos) * (1.0 - (-dt * 4.0).exp());
            }
        } else {
            self.still = 0.0;
            pos = walk(map, pos, dir * speed * dt, radius);
        }
        self.pos = Some(pos);
        Some(pos)
    }

    pub fn pos(&self) -> Option<Vec2> {
        self.pos
    }
}
