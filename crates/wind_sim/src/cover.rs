//! Укрытия. Стены у нас в полный рост: из-за стены не выстрелить, а сквозь неё
//! не попасть. Поэтому укрытие — это **угол**: место у стены, откуда врага видно
//! (`shoot`), и рядом, в шаг-другой, место, откуда не видно (`hide`). Солдат стреляет
//! из-за угла и прячется за него, пока перезаряжается.
//!
//! Ищется укрытие при «обдумывании» и не чаще раза в `CoverSpec::research`, а
//! найденное только проверяется двумя лучами — поиск дорогой, проверка дешёвая.

use crate::map::Map;
use glam::Vec2;
use serde::Deserialize;

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct CoverSpec {
    /// Где искать укрытие: в стольких метрах от себя (бойцу отряда — от своего
    /// места в строю). 0 — укрытий не ищут, как раньше.
    pub search_radius: f32,
    /// Сколько секунд держаться укрытия после того, как врага не видно и пули
    /// не летят рядом.
    pub hold: f32,
    /// Не искать новое укрытие чаще, с.
    pub research: f32,
    /// Пуля, пролетевшая ближе, — «по мне стреляют», м.
    pub near_miss: f32,
    /// Сколько секунд солдат помнит, откуда по нему стреляли.
    pub under_fire: f32,
}

impl Default for CoverSpec {
    fn default() -> Self {
        CoverSpec { search_radius: 12.0, hold: 4.0, research: 1.0, near_miss: 2.0, under_fire: 3.0 }
    }
}

/// Найденное укрытие.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cover {
    /// Отсюда враг виден — здесь стреляют.
    pub shoot: Vec2,
    /// Отсюда не виден — здесь прячутся.
    pub hide: Vec2,
    /// От кого укрываемся.
    pub threat: Vec2,
}

impl Cover {
    /// Всё ещё укрытие от `threat`: из `shoot` видно, из `hide` — нет.
    pub fn holds(&self, map: &Map, threat: Vec2) -> bool {
        map.line_of_sight(self.shoot, threat) && self.shelters(map, threat)
    }

    /// Спрятаться от `threat` здесь всё ещё можно.
    pub fn shelters(&self, map: &Map, threat: Vec2) -> bool {
        !map.line_of_sight(self.hide, threat)
    }
}

/// Что известно о стрелке и о себе при поиске.
pub struct Query {
    /// Где солдат сейчас.
    pub from: Vec2,
    /// Вокруг какой точки искать (свой место в строю или где стоит).
    pub around: Vec2,
    pub threat: Vec2,
    /// С какого и до какого расстояния до врага стрелять (дальность оружия;
    /// гранатомёту — не в упор).
    pub min_range: f32,
    pub max_range: f32,
    pub radius: f32,
    /// Радиус солдата: в укрытии он не должен задевать стену.
    pub body: f32,
}

/// Центр клетки.
fn center(x: i32, y: i32) -> Vec2 {
    Vec2::new(x as f32 + 0.5, y as f32 + 0.5)
}

/// Свободная клетка, в которой солдат помещается.
fn free(map: &Map, x: i32, y: i32, body: f32) -> bool {
    map.index(x, y).is_some() && !map.is_solid(x, y) && !map.circle_hits_solid(center(x, y), body)
}

fn next_to_wall(map: &Map, x: i32, y: i32) -> bool {
    (-1..=1).any(|dy| (-1..=1).any(|dx| (dx, dy) != (0, 0) && map.is_solid(x + dx, y + dy)))
}

/// Ищет угол: ближайшее к солдату место у стены, откуда враг виден на дистанции
/// огня, и рядом место, откуда он не виден. `taken(x, y)` — клетка уже чьё-то
/// укрытие (двое в одном углу толкаются и оба на виду).
pub fn find(map: &Map, q: &Query, taken: impl Fn(i32, i32) -> bool) -> Option<Cover> {
    let (cx, cy) = Map::cell_of(q.around);
    let r = q.radius.ceil() as i32;
    let mut best: Option<(f32, Cover)> = None;
    for y in cy - r..=cy + r {
        for x in cx - r..=cx + r {
            let shoot = center(x, y);
            if shoot.distance(q.around) > q.radius || !next_to_wall(map, x, y) || !free(map, x, y, q.body) || taken(x, y) {
                continue;
            }
            let range = shoot.distance(q.threat);
            if range < q.min_range || range > q.max_range {
                continue;
            }
            // Дешёвая оценка до лучей: дальше уже найденного — не смотрим.
            let score = shoot.distance(q.from);
            if best.is_some_and(|(b, _)| score >= b) {
                continue;
            }
            if !map.line_of_sight(shoot, q.threat) {
                continue;
            }
            // Где спрятаться: соседняя клетка (до двух шагов), не видная врагу,
            // и дойти до неё по прямой.
            let mut hide = None;
            'search: for step in 1..=2i32 {
                for dy in -step..=step {
                    for dx in -step..=step {
                        if dx.abs().max(dy.abs()) != step {
                            continue;
                        }
                        let (hx, hy) = (x + dx, y + dy);
                        let h = center(hx, hy);
                        if free(map, hx, hy, q.body) && map.line_of_sight(shoot, h) && !map.line_of_sight(h, q.threat) {
                            hide = Some(h);
                            break 'search;
                        }
                    }
                }
            }
            if let Some(hide) = hide {
                best = Some((score, Cover { shoot, hide, threat: q.threat }));
            }
        }
    }
    best.map(|(_, c)| c)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Стена с запада на восток по y = 20, от x = 0 до x = 29; враг севернее, солдат
    /// южнее, у середины стены (там врага не видно и угла нет).
    fn wall_map() -> Map {
        let mut map = Map::new(64, 64);
        for x in 0..30 {
            map.set(x, 20, 200);
        }
        map
    }

    fn query(from: Vec2, threat: Vec2) -> Query {
        Query { from, around: from, threat, min_range: 3.0, max_range: 50.0, radius: 8.0, body: 0.35 }
    }

    #[test]
    fn corner_lets_shoot_and_hide() {
        let map = wall_map();
        let threat = Vec2::new(36.0, 8.0);
        let c = find(&map, &query(Vec2::new(26.0, 23.0), threat), |_, _| false).expect("у конца стены есть угол");
        assert!(c.holds(&map, threat));
        // Угол — у конца стены (x ≈ 29), а не где-то посреди поля.
        assert!(c.shoot.x > 25.0 && c.shoot.x < 32.0 && (c.shoot.y - 20.5).abs() < 2.0, "{c:?}");
        assert!(c.shoot.distance(c.hide) < 3.0);
        assert!(!map.circle_hits_solid(c.shoot, 0.35) && !map.circle_hits_solid(c.hide, 0.35));
    }

    #[test]
    fn no_cover_in_open_field() {
        let map = Map::new(64, 64);
        assert!(find(&map, &query(Vec2::new(20.0, 30.0), Vec2::new(40.0, 10.0)), |_, _| false).is_none());
    }

    #[test]
    fn taken_corner_is_skipped_and_range_respected() {
        let map = wall_map();
        let threat = Vec2::new(36.0, 8.0);
        let q = query(Vec2::new(26.0, 23.0), threat);
        let first = find(&map, &q, |_, _| false).unwrap();
        let taken = Map::cell_of(first.shoot);
        let second = find(&map, &q, |x, y| (x, y) == taken).unwrap();
        assert_ne!(second.shoot, first.shoot);
        // Враг дальше дальности оружия — такое «укрытие» ни к чему.
        let short = Query { max_range: 5.0, ..q };
        assert!(find(&map, &short, |_, _| false).is_none());
    }
}
