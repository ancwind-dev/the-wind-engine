//! Стандарт анимаций движка (с 2026-11-07) — по устройству как у RWR (см. заметку
//! «Как устроены анимации RWR»), но свои клипы и свой файл.
//!
//! Клип — файл `data/animations/<ключ>.ron`:
//! - **ключ** — имя, по которому клип берёт код: «running», «crouch still», «going prone»…
//!   (те же имена, что у RWR, — чтобы сверять один к одному);
//! - **длина** и **скорость проигрывания** (секунды = длина / скорость), круг или раз;
//! - **ключевые кадры**: время и 15 точек скелета по именам — в вокселях, в осях модели
//!   (x — вперёд, y — вверх от земли, z — вправо), на скелете покоя наших моделей (он тот
//!   же, что у RWR); у действий с оружием — ещё где оружие (`weapon`);
//! - **метки** в кадрах: шаг, выпуск гранаты, рука отпустила оружие, магазин, затвор, толчок.
//!
//! Своё сверх RWR: `body` — клип на всё тело или только на верх (ноги идут своим шагом,
//! пока руки перезаряжают), `hands` — руки ставит оружие или берутся из клипа, `stride` —
//! путь за круг (чтобы стопы не скользили), `repeat` — отрезок, который повторяется по
//! выстрелу (зарядить барабан по одному).
//!
//! Между ключами поза меняется плавно, отрезки скелета сохраняют длину: клип собирается на
//! каждой модели (см. `character::clips`).

use glam::Vec3;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

/// 15 точек скелета по именам (порядок — как у точек в файлах моделей и у RWR).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Points {
    pub head: [f32; 3],
    pub neck: [f32; 3],
    pub rightshoulder: [f32; 3],
    pub leftshoulder: [f32; 3],
    pub rightelbow: [f32; 3],
    pub leftelbow: [f32; 3],
    pub righthand: [f32; 3],
    pub lefthand: [f32; 3],
    pub midspine: [f32; 3],
    pub righthip: [f32; 3],
    pub lefthip: [f32; 3],
    pub rightknee: [f32; 3],
    pub leftknee: [f32; 3],
    pub rightfoot: [f32; 3],
    pub leftfoot: [f32; 3],
}

/// Имена точек по порядку `Points`.
pub const NAMES: [&str; 15] = crate::rwr_anim::POINTS;

impl Points {
    pub fn to_array(&self) -> [Vec3; 15] {
        [
            self.head,
            self.neck,
            self.rightshoulder,
            self.leftshoulder,
            self.rightelbow,
            self.leftelbow,
            self.righthand,
            self.lefthand,
            self.midspine,
            self.righthip,
            self.lefthip,
            self.rightknee,
            self.leftknee,
            self.rightfoot,
            self.leftfoot,
        ]
        .map(Vec3::from_array)
    }

    pub fn from_array(p: [Vec3; 15]) -> Points {
        // Две цифры после запятой — точнее вокселя в десятки раз, а файл читается глазами.
        let r = |v: Vec3| v.to_array().map(|x| (x * 100.0).round() / 100.0);
        Points {
            head: r(p[0]),
            neck: r(p[1]),
            rightshoulder: r(p[2]),
            leftshoulder: r(p[3]),
            rightelbow: r(p[4]),
            leftelbow: r(p[5]),
            righthand: r(p[6]),
            lefthand: r(p[7]),
            midspine: r(p[8]),
            righthip: r(p[9]),
            lefthip: r(p[10]),
            rightknee: r(p[11]),
            leftknee: r(p[12]),
            rightfoot: r(p[13]),
            leftfoot: r(p[14]),
        }
    }

    pub fn get(&self, name: &str) -> Option<Vec3> {
        NAMES.iter().position(|n| *n == name).map(|i| self.to_array()[i])
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Side {
    Right,
    Left,
}

/// Метка в кадре — событие в этот момент (у RWR — `<control key value>`).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum Event {
    /// Стопа коснулась земли (у RWR — `hit_ground`): звук шага.
    Step(Side),
    /// Граната уходит из руки (у RWR — `action`).
    Release,
    /// Левая рука держит оружие (`true`) или отпустила (у RWR — `weak_hand`).
    WeakHand(bool),
    /// Правая рука держит рукоять (`true`) или отпустила (`strong_hand`).
    StrongHand(bool),
    /// Магазин на месте (`true`) или снят (`magazine`).
    Magazine(bool),
    /// Затвор передёрнут (`cycle`).
    Cycle,
    /// Толчок ползком (`push`).
    Push,
}

/// Где оружие в ключе: рукоять, куда смотрит ствол, поворот вокруг ствола (плюс — низом
/// к левой руке). Без него оружие держат руки: рукоять — в правой, ствол — к левой.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct WeaponKey {
    pub grip: [f32; 3],
    pub aim: [f32; 3],
    pub roll: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Key {
    /// Время в единицах клипа (как `length`).
    pub time: f32,
    pub points: Points,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weapon: Option<WeaponKey>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<Event>,
}

/// На что клип.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Body {
    /// Всё тело: ходьба, присед, лечь.
    #[default]
    Full,
    /// Только верх (корпус выше поясницы, голова, руки): ноги идут своим шагом.
    Upper,
    /// Поверх любой позы: ключи — сдвиги от первого ключа (отдача от выстрела).
    Additive,
}

/// Кто ставит руки.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Hands {
    /// Руки держат оружие, как солдат целится (`CharModel::pose`), — клип задаёт тело.
    #[default]
    Weapon,
    /// Руки — из клипа (перезарядка, бросок, лечь).
    Clip,
}

fn one() -> f32 {
    1.0
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AnimClip {
    pub key: String,
    /// Что это, по-русски — для людей.
    #[serde(default)]
    pub about: String,
    /// Откуда клип: свой, из записи движений (с указанием записи) и т. п.
    #[serde(default)]
    pub source: String,
    pub looped: bool,
    /// Конец клипа в единицах времени ключей. У круга — когда он возвращается к первому ключу.
    pub length: f32,
    /// Скорость проигрывания: секунд в клипе — `length / speed`.
    #[serde(default = "one")]
    pub speed: f32,
    /// Разброс скорости между солдатами (доля), чтобы толпа не двигалась в ногу.
    #[serde(default)]
    pub speed_spread: f32,
    /// Путь за круг в длинах ноги (у движения; у остального — 0).
    #[serde(default)]
    pub stride: f32,
    #[serde(default)]
    pub body: Body,
    #[serde(default)]
    pub hands: Hands,
    /// Отрезок времени, который повторяется по выстрелу (зарядить по одному).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repeat: Option<(f32, f32)>,
    pub keys: Vec<Key>,
}

impl AnimClip {
    pub fn load(path: &Path) -> Result<AnimClip, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
        let clip: AnimClip = ron::from_str(text).map_err(|e| format!("{}: {e}", path.display()))?;
        clip.check().map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(clip)
    }

    /// Клип годится: есть ключи, время по возрастанию и не дальше конца.
    pub fn check(&self) -> Result<(), String> {
        if self.keys.is_empty() {
            return Err("в клипе нет ключей".into());
        }
        if self.keys.windows(2).any(|w| w[1].time < w[0].time) {
            return Err("время ключей должно расти".into());
        }
        if self.keys.last().is_some_and(|k| k.time > self.length + 1e-4) {
            return Err(format!("ключ после конца клипа ({})", self.length));
        }
        Ok(())
    }

    /// Секунд в клипе.
    pub fn seconds(&self) -> f32 {
        self.length / self.speed.max(1e-3)
    }

    /// Имя файла клипа: пробелы и запятые — в подчёркивания.
    pub fn file_name(key: &str) -> String {
        let mut s: String = key.chars().map(|c| if c.is_alphanumeric() { c } else { '_' }).collect();
        while s.contains("__") {
            s = s.replace("__", "_");
        }
        format!("{}.ron", s.trim_matches('_'))
    }

    /// Текст файла: заголовок-комментарий и сам клип.
    pub fn to_ron(&self) -> Result<String, String> {
        let config = ron::ser::PrettyConfig::new().depth_limit(3).indentor("    ".to_owned());
        let body = ron::ser::to_string_pretty(self, config).map_err(|e| e.to_string())?;
        Ok(format!(
            "// Клип «{}»{}. Стандарт анимаций движка: точки скелета в вокселях, x — вперёд,\n// y — вверх от земли, z — вправо (см. crates/wind_render/src/anim.rs).\n{body}\n",
            self.key,
            if self.about.is_empty() { String::new() } else { format!(" — {}", self.about) }
        ))
    }

    pub fn save(&self, dir: &Path) -> Result<std::path::PathBuf, String> {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let path = dir.join(Self::file_name(&self.key));
        std::fs::write(&path, self.to_ron()?).map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(path)
    }

    /// Точки на времени `t` (единицы клипа): по прямой между ключами, круг — через конец
    /// к первому ключу. Для листов кадров и сверки; в игре клип собирается на модели.
    pub fn sample(&self, t: f32) -> [Vec3; 15] {
        let k = &self.keys;
        if k.len() == 1 {
            return k[0].points.to_array();
        }
        let t = if self.looped && self.length > 0.0 { t.rem_euclid(self.length) } else { t.clamp(0.0, self.length) };
        let (a, b, u) = self.around(t);
        let (pa, pb) = (k[a].points.to_array(), k[b].points.to_array());
        std::array::from_fn(|i| pa[i].lerp(pb[i], u))
    }

    /// Соседние ключи на времени `t` и доля между ними.
    pub fn around(&self, t: f32) -> (usize, usize, f32) {
        let k = &self.keys;
        let n = k.len();
        if n == 1 || t <= k[0].time && !self.looped {
            return (0, 0, 0.0);
        }
        for i in 0..n - 1 {
            if t <= k[i + 1].time {
                if t < k[i].time {
                    break;
                }
                return (i, i + 1, (t - k[i].time) / (k[i + 1].time - k[i].time).max(1e-5));
            }
        }
        if self.looped {
            // После последнего ключа — к первому (к концу клипа), до первого — от последнего.
            let (last, first) = (k[n - 1].time, k[0].time + self.length);
            let t = if t < k[0].time { t + self.length } else { t };
            return (n - 1, 0, ((t - last) / (first - last).max(1e-5)).clamp(0.0, 1.0));
        }
        (n - 1, n - 1, 0.0)
    }

    /// Метки по времени: (время, метка).
    pub fn events(&self) -> Vec<(f32, Event)> {
        self.keys.iter().flat_map(|k| k.events.iter().map(move |&e| (k.time, e))).collect()
    }

    /// Ключи там, где без них поза уходит больше чем на `tolerance` вокселя: из плотной
    /// записи (кадр за кадром) — редкие ключи, как у RWR. Первый и последний остаются.
    pub fn simplify(&mut self, tolerance: f32) {
        if self.keys.len() < 3 {
            return;
        }
        let keys = std::mem::take(&mut self.keys);
        let mut keep = vec![false; keys.len()];
        keep[0] = true;
        keep[keys.len() - 1] = true;
        // Дуглас — Пекер по времени: ищем самый далёкий кадр от прямой между ключами.
        let mut stack = vec![(0, keys.len() - 1)];
        while let Some((a, b)) = stack.pop() {
            if b <= a + 1 {
                continue;
            }
            let (ta, tb) = (keys[a].time, keys[b].time);
            let (pa, pb) = (keys[a].points.to_array(), keys[b].points.to_array());
            let wa = keys[a].weapon;
            let mut worst = (0.0_f32, a);
            for (i, key) in keys.iter().enumerate().take(b).skip(a + 1) {
                let u = (key.time - ta) / (tb - ta).max(1e-5);
                let p = key.points.to_array();
                let mut d = (0..15).map(|j| p[j].distance(pa[j].lerp(pb[j], u))).fold(0.0, f32::max);
                if let (Some(w), Some(wa), Some(wb)) = (key.weapon, wa, keys[b].weapon) {
                    let g = Vec3::from_array(wa.grip).lerp(Vec3::from_array(wb.grip), u);
                    let aim = Vec3::from_array(wa.aim).lerp(Vec3::from_array(wb.aim), u).normalize_or(Vec3::X);
                    d = d.max(Vec3::from_array(w.grip).distance(g)).max(Vec3::from_array(w.aim).normalize_or(Vec3::X).distance(aim) * 10.0);
                }
                // Метки не теряем: кадр с меткой — всегда ключ.
                if !key.events.is_empty() {
                    d = f32::MAX;
                }
                if d > worst.0 {
                    worst = (d, i);
                }
            }
            if worst.0 > tolerance {
                keep[worst.1] = true;
                stack.push((a, worst.1));
                stack.push((worst.1, b));
            }
        }
        self.keys = keys.into_iter().zip(keep).filter(|(_, k)| *k).map(|(k, _)| k).collect();
    }
}

/// Все клипы из папки: ключ → клип. Файлы старого формата (записи движений до 2026-11-07)
/// пропускаются с пометкой в `notes`.
pub fn load_dir(dir: &Path) -> (BTreeMap<String, AnimClip>, Vec<String>) {
    let mut out = BTreeMap::new();
    let mut notes = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else { return (out, notes) };
    let mut paths: Vec<_> = entries.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "ron")).collect();
    paths.sort();
    for path in paths {
        match AnimClip::load(&path) {
            Ok(clip) => {
                out.insert(clip.key.clone(), clip);
            }
            Err(e) => notes.push(e),
        }
    }
    (out, notes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(time: f32, y: f32) -> Key {
        let mut p = [Vec3::ZERO; 15];
        p[0] = Vec3::new(0.0, y, 0.0);
        Key { time, points: Points::from_array(p), weapon: None, events: Vec::new() }
    }

    #[test]
    fn samples_between_keys_and_loops_through_the_end() {
        let clip = AnimClip {
            key: "проба".into(),
            about: String::new(),
            source: String::new(),
            looped: true,
            length: 1.0,
            speed: 2.0,
            speed_spread: 0.0,
            stride: 0.0,
            body: Body::Full,
            hands: Hands::Weapon,
            repeat: None,
            keys: vec![key(0.0, 10.0), key(0.5, 20.0)],
        };
        assert!((clip.sample(0.25)[0].y - 15.0).abs() < 1e-4);
        // После последнего ключа — обратно к первому к концу клипа.
        assert!((clip.sample(0.75)[0].y - 15.0).abs() < 1e-4);
        assert!((clip.seconds() - 0.5).abs() < 1e-6);
        // Файл читается обратно тем же клипом.
        let back: AnimClip = ron::from_str(&clip.to_ron().unwrap()).unwrap();
        assert_eq!(back, clip);
        assert_eq!(AnimClip::file_name("prone, moving forwards"), "prone_moving_forwards.ron");
    }

    #[test]
    fn simplify_keeps_corners_and_events() {
        let mut keys: Vec<Key> = (0..=20).map(|i| key(i as f32 * 0.05, if i <= 10 { i as f32 } else { 20.0 - i as f32 })).collect();
        keys[3].events.push(Event::Release);
        let mut clip = AnimClip {
            key: "углы".into(),
            about: String::new(),
            source: String::new(),
            looped: false,
            length: 1.0,
            speed: 1.0,
            speed_spread: 0.0,
            stride: 0.0,
            body: Body::Full,
            hands: Hands::Clip,
            repeat: None,
            keys,
        };
        clip.simplify(0.1);
        let times: Vec<f32> = clip.keys.iter().map(|k| k.time).collect();
        // Остались начало, излом посередине, конец и кадр с меткой.
        assert_eq!(clip.keys.len(), 4, "{times:?}");
        assert!(times.iter().any(|t| (t - 0.5).abs() < 1e-4) && times.iter().any(|t| (t - 0.15).abs() < 1e-4));
    }
}
