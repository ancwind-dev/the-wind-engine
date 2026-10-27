//! Штаб команды: раз в несколько секунд решает, какой отряд к какой точке идёт.
//!
//! Без штаба каждый отряд сам выбирает ближайшую не нашу точку — и половина
//! команды толпится у одной, пока другие пустуют. Штаб смотрит на все точки сразу:
//!
//! 1. **Нужда точки** — сколько бойцов ей нужно:
//!    - своя под угрозой (враги рядом) — подкрепления, в первую очередь;
//!    - ничья — захватить;
//!    - чужая — атаковать;
//!    - своя спокойная — небольшой гарнизон.
//!
//!    Чем больше врагов рядом, тем больше нужно своих.
//! 2. Точки по важности (а среди равных — ближе к своей базе первыми) получают
//!    ближайшие свободные отряды, пока нужда не закрыта. Атаку не по силам (у базы
//!    врага сотня бойцов) не начинают — отряды по одному там просто погибнут.
//! 3. Оставшиеся отряды усиливают начатую атаку, где бой жарче всего.
//!
//! Отряд, который уже идёт к точке, считается к ней ближе на `stickiness` метров,
//! а к другим — дальше на половину этого: штаб не гоняет его туда-сюда из-за пары
//! погибших и скорее пошлёт свободный.

use crate::world::{CapturePoint, NO_OBJECTIVE};
use glam::Vec2;
use serde::Deserialize;

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct CommandSpec {
    /// Штаб есть у обеих команд. `false` — отряды выбирают точки сами, как раньше.
    pub enabled: bool,
    /// Как часто штаб пересматривает приказы, с.
    pub interval: f32,
    /// Враги ближе этого к точке — угроза ей, м.
    pub threat_radius: f32,
    /// Сколько бойцов держать на своей спокойной точке.
    pub garrison: f32,
    /// Сколько бойцов сверх «по одному на врага» нужно для захвата и атаки.
    pub attack_extra: f32,
    /// Сколько своих нужно на каждого врага у точки.
    pub enemy_weight: f32,
    /// На столько метров ближе считается точка, к которой отряд уже идёт.
    pub stickiness: f32,
}

impl Default for CommandSpec {
    fn default() -> Self {
        CommandSpec { enabled: true, interval: 5.0, threat_radius: 40.0, garrison: 3.0, attack_extra: 12.0, enemy_weight: 2.5, stickiness: 60.0 }
    }
}

/// Как штаб видит точку — для лаборатории и тестов.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PointPlan {
    /// 3 — своя под угрозой, 2 — ничья, 1 — чужая, 0 — своя спокойная.
    pub priority: u8,
    /// Сколько бойцов нужно.
    pub need: f32,
    /// Своих и врагов рядом с точкой.
    pub ours: u32,
    pub enemies: u32,
    /// Сколько отрядов и бойцов в них штаб туда послал.
    pub squads: u32,
    pub sent: u32,
}

/// Что штаб знает об отряде.
#[derive(Clone, Debug)]
pub struct SquadInfo {
    /// Где командир.
    pub pos: Vec2,
    /// Сколько в отряде бойцов, способных драться.
    pub strength: u32,
    /// Куда отряд идёт сейчас.
    pub current: u32,
}

/// Приказы: точка для каждого отряда (в порядке `squads`) и как штаб видит точки.
/// `ours` и `enemies` — сколько своих и врагов у каждой точки.
pub fn plan(points: &[CapturePoint], team: u8, base: Vec2, squads: &[SquadInfo], ours: &[u32], enemies: &[u32], spec: &CommandSpec) -> (Vec<u32>, Vec<PointPlan>) {
    // Линия фронта — ближайшая к нашей базе не наша точка. Свои точки заметно ближе
    // к базе — тыл: гарнизон им не нужен, туда враг дойдёт не сразу.
    let front = points.iter().filter(|p| p.owner != Some(team)).map(|p| p.pos.distance(base)).fold(f32::INFINITY, f32::min);
    // Все точки наши — фронта нет, гарнизон нужен всем.
    let rear = |p: &CapturePoint| front.is_finite() && p.pos.distance(base) < front - spec.threat_radius;
    let mut plans: Vec<PointPlan> = points
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let e = enemies[i] as f32 * spec.enemy_weight;
            let (priority, need) = match p.owner {
                Some(t) if t == team && enemies[i] > 0 => (3, e + spec.garrison),
                Some(t) if t == team => (0, if rear(p) { 0.0 } else { spec.garrison }),
                None => (2, e + spec.attack_extra),
                Some(_) => (1, e + spec.attack_extra),
            };
            PointPlan { priority, need, ours: ours[i], enemies: enemies[i], squads: 0, sent: 0 }
        })
        .collect();

    let mut order: Vec<usize> = (0..points.len()).collect();
    order.sort_by(|&a, &b| {
        plans[b].priority.cmp(&plans[a].priority).then(points[a].pos.distance(base).total_cmp(&points[b].pos.distance(base)))
    });

    // Своя прежняя точка — ближе, чужая для занятого отряда — дальше: свободный отряд
    // послать проще, чем развернуть идущий.
    let cost = |s: &SquadInfo, point: usize| {
        let turn = match s.current {
            c if c == point as u32 => -spec.stickiness,
            NO_OBJECTIVE => 0.0,
            _ => spec.stickiness * 0.5,
        };
        s.pos.distance(points[point].pos) + turn
    };
    let mut orders = vec![NO_OBJECTIVE; squads.len()];
    let assign = |orders: &mut Vec<u32>, plans: &mut Vec<PointPlan>, s: usize, point: usize| {
        orders[s] = point as u32;
        plans[point].squads += 1;
        plans[point].sent += squads[s].strength;
    };

    // Нужды по важности: ближайшие свободные отряды, пока не хватит. Атаку, на которую
    // сил не хватит всё равно (у базы врага сотня бойцов), не начинаем: отряды,
    // посланные туда по одному, просто погибнут.
    for &point in &order {
        let free_strength: u32 = (0..squads.len()).filter(|&s| orders[s] == NO_OBJECTIVE).map(|s| squads[s].strength).sum();
        if plans[point].priority < 3 && plans[point].enemies as f32 * spec.enemy_weight > free_strength as f32 {
            continue;
        }
        while (plans[point].sent as f32) < plans[point].need {
            let free = (0..squads.len())
                .filter(|&s| orders[s] == NO_OBJECTIVE && squads[s].strength > 0)
                .min_by(|&a, &b| cost(&squads[a], point).total_cmp(&cost(&squads[b], point)));
            let Some(s) = free else { break };
            assign(&mut orders, &mut plans, s, point);
        }
    }

    // Остальные — усилить начатую атаку, где идёт настоящий бой (больше всего врагов;
    // непосильные уже отсеяны), среди равных — ближе. Пустой точке хватит и тех, кого
    // уже послали. Если брать нечего — к своим точкам поровну.
    let any_to_take = plans.iter().any(|p| p.priority > 0 && p.squads > 0);
    for s in 0..squads.len() {
        if orders[s] != NO_OBJECTIVE || squads[s].strength == 0 {
            continue;
        }
        let best = (0..points.len())
            .filter(|&i| if any_to_take { plans[i].priority > 0 && plans[i].squads > 0 } else { true })
            .min_by(|&a, &b| {
                let threat = |i: usize| if any_to_take { -(plans[i].enemies as f32) } else { plans[i].sent as f32 - plans[i].need };
                threat(a).total_cmp(&threat(b)).then(cost(&squads[s], a).total_cmp(&cost(&squads[s], b)))
            });
        if let Some(point) = best {
            assign(&mut orders, &mut plans, s, point);
        }
    }
    (orders, plans)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point(x: f32, owner: Option<u8>) -> CapturePoint {
        CapturePoint { pos: Vec2::new(x, 0.0), owner, progress: 0.0 }
    }

    fn squad(x: f32, current: u32) -> SquadInfo {
        SquadInfo { pos: Vec2::new(x, 0.0), strength: 6, current }
    }

    #[test]
    fn squads_spread_over_points() {
        // Пять ничьих точек, десять отрядов у базы: раньше все шли к ближайшей.
        let points: Vec<_> = (0..5).map(|i| point(100.0 + i as f32 * 50.0, None)).collect();
        let squads: Vec<_> = (0..10).map(|_| squad(10.0, NO_OBJECTIVE)).collect();
        let (orders, plans) = plan(&points, 0, Vec2::ZERO, &squads, &[0; 5], &[0; 5], &CommandSpec::default());
        assert!(orders.iter().all(|&o| o != NO_OBJECTIVE));
        // На каждую точку — по отряду (6 бойцов закрывают нужду 6), лишние — по нехватке.
        assert!(plans.iter().all(|p| p.squads >= 1), "{plans:?}");
        assert!(plans.iter().all(|p| p.squads <= 3), "{plans:?}");
    }

    #[test]
    fn threatened_point_gets_reinforcements_first() {
        // Наша точка B под атакой 6 врагов; ничья A ближе к отрядам.
        let points = [point(100.0, None), point(300.0, Some(0))];
        let squads: Vec<_> = (0..4).map(|i| squad(90.0 + i as f32, NO_OBJECTIVE)).collect();
        let (orders, plans) = plan(&points, 0, Vec2::ZERO, &squads, &[0, 2], &[0, 6], &CommandSpec::default());
        assert_eq!(plans[1].priority, 3);
        // 6 врагов × 2,5 + гарнизон 3 = 18 бойцов — три отряда по 6, четвёртый — на A.
        assert_eq!(orders.iter().filter(|&&o| o == 1).count(), 3, "{orders:?}");
        assert_eq!(orders.iter().filter(|&&o| o == 0).count(), 1);
    }

    #[test]
    fn squads_keep_their_orders() {
        // Две одинаковые ничьи точки, каждой хватит отряда: отряд, уже идущий к дальней,
        // не разворачивают — к ближней пойдёт свободный.
        let points = [point(100.0, None), point(130.0, None)];
        let squads = [squad(60.0, 1), squad(60.0, NO_OBJECTIVE)];
        let spec = CommandSpec { attack_extra: 6.0, ..Default::default() };
        let (orders, _) = plan(&points, 0, Vec2::ZERO, &squads, &[0, 0], &[0, 0], &spec);
        assert_eq!(orders, vec![1, 0]);
    }

    #[test]
    fn all_points_ours_garrison_and_guard_threats() {
        let points = [point(100.0, Some(1)), point(200.0, Some(1)), point(300.0, Some(1))];
        let squads: Vec<_> = (0..5).map(|i| squad(200.0 + i as f32, NO_OBJECTIVE)).collect();
        let (orders, plans) = plan(&points, 1, Vec2::new(400.0, 0.0), &squads, &[3, 3, 3], &[0, 0, 0], &CommandSpec::default());
        assert!(plans.iter().all(|p| p.priority == 0 && p.squads >= 1), "у каждой своей точки гарнизон: {plans:?}");
        assert!(orders.iter().all(|&o| o != NO_OBJECTIVE));
    }
}
