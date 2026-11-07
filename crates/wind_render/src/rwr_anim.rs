//! ВРЕМЕННО, для разбора: чтение анимаций RWR (`soldier_animations.xml`) прямо из
//! установленной игры. В репозиторий и в сборку они не попадают — только смотрим в
//! студии и сверяем с нашими (см. заметку «Как устроены анимации RWR»). Свои клипы
//! делаем сами, по своим позам; из RWR берём числа (длительности, скорости) и устройство.
//!
//! Формат RWR: `<animation loop end speed [speed_spread] comment>`, внутри — кадры
//! `<frame time>` с 15 точками `<position x y z>` в порядке точек скелета модели
//! (голова, шея, плечи, локти, кисти, поясница, бёдра, колени, стопы) и метками
//! `<control key value>` (шаг коснулся земли, бросок, левая рука и т. п.).

use crate::rwr::VOXEL;
use glam::Vec3;
use std::path::{Path, PathBuf};

/// Имена точек в порядке RWR — тот же порядок, что у точек в файлах моделей.
pub const POINTS: [&str; 15] = [
    "head", "neck", "rightshoulder", "leftshoulder", "rightelbow", "leftelbow", "righthand", "lefthand", "midspine", "righthip", "lefthip",
    "rightknee", "leftknee", "rightfoot", "leftfoot",
];

/// Метка в кадре: `<control key="hit_ground" value="1" />`.
#[derive(Clone, Debug, PartialEq)]
pub struct Control {
    pub key: String,
    pub value: String,
}

#[derive(Clone, Debug)]
pub struct RwrFrame {
    /// Время кадра в единицах анимации (см. `RwrAnim::end`).
    pub time: f32,
    /// Точки в вокселях, уже в наших осях (x — вперёд, y — вверх, z — вправо).
    pub points: [Vec3; 15],
    pub controls: Vec<Control>,
}

#[derive(Clone, Debug)]
pub struct RwrAnim {
    /// Номер в файле: по нему на анимацию ссылается оружие (`<animation key="recoil" ref="12" />`).
    pub index: usize,
    /// Имя (`comment`): по нему ссылаются персонаж и оружие (`animation_key`).
    pub name: String,
    pub looped: bool,
    /// Конец анимации — время последнего кадра или (у круга) время, когда он
    /// возвращается к первому кадру.
    pub end: f32,
    /// Скорость проигрывания.
    pub speed: f32,
    /// Разброс скорости: у каждого солдата чуть своя.
    pub speed_spread: Option<f32>,
    pub frames: Vec<RwrFrame>,
}

/// Где лежит RWR: переменная `WIND_RWR` (папка игры) или Steam по умолчанию.
pub fn find_install() -> Option<PathBuf> {
    let candidates = std::env::var_os("WIND_RWR").map(PathBuf::from).into_iter().chain([
        PathBuf::from(r"C:\Program Files (x86)\Steam\steamapps\common\RunningWithRifles"),
        PathBuf::from(r"C:\Program Files\Steam\steamapps\common\RunningWithRifles"),
        PathBuf::from(r"D:\SteamLibrary\steamapps\common\RunningWithRifles"),
    ]);
    candidates.into_iter().find(|p| animations_file(p).exists())
}

pub fn animations_file(install: &Path) -> PathBuf {
    install.join("media/packages/vanilla/models/soldier_animations.xml")
}

/// Все анимации солдата из установленной RWR.
pub fn load_install() -> Result<Vec<RwrAnim>, String> {
    let install = find_install().ok_or("RWR не найдена: укажи папку игры в переменной WIND_RWR")?;
    let path = animations_file(&install);
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    parse(&text)
}

fn attrs(tag: &str) -> impl Iterator<Item = (&str, &str)> {
    let mut rest = tag;
    std::iter::from_fn(move || {
        let eq = rest.find("=\"")?;
        let key = rest[..eq].trim().rsplit(char::is_whitespace).next()?;
        let after = &rest[eq + 2..];
        let end = after.find('"')?;
        let value = &after[..end];
        rest = &after[end + 1..];
        Some((key, value))
    })
}

fn num(v: &str) -> Result<f32, String> {
    v.trim().parse().map_err(|_| format!("не число «{v}»"))
}

pub fn parse(text: &str) -> Result<Vec<RwrAnim>, String> {
    let mut out: Vec<RwrAnim> = Vec::new();
    let mut points: Vec<Vec3> = Vec::new();
    for piece in text.split('<').skip(1) {
        let tag = piece.split('>').next().unwrap_or("");
        let name = tag.split(|c: char| c.is_whitespace() || c == '/').next().unwrap_or("");
        match name {
            "animation" => {
                let mut a = RwrAnim { index: out.len(), name: String::new(), looped: false, end: 0.0, speed: 1.0, speed_spread: None, frames: Vec::new() };
                for (k, v) in attrs(tag) {
                    match k {
                        "loop" => a.looped = v.trim() == "1",
                        "end" => a.end = num(v)?,
                        "speed" => a.speed = num(v)?,
                        "speed_spread" => a.speed_spread = Some(num(v)?),
                        "comment" => a.name = v.to_owned(),
                        _ => {}
                    }
                }
                out.push(a);
            }
            "frame" => {
                let a = out.last_mut().ok_or("кадр вне анимации")?;
                let time = attrs(tag).find(|(k, _)| *k == "time").map(|(_, v)| num(v)).transpose()?.unwrap_or(0.0);
                points.clear();
                a.frames.push(RwrFrame { time, points: [Vec3::ZERO; 15], controls: Vec::new() });
            }
            "position" => {
                let (mut x, mut y, mut z) = (0.0, 0.0, 0.0);
                for (k, v) in attrs(tag) {
                    match k {
                        "x" => x = num(v)?,
                        "y" => y = num(v)?,
                        "z" => z = num(v)?,
                        _ => {}
                    }
                }
                // Оси RWR → наши, как у моделей: (x, y, z) → (z, y, −x).
                points.push(Vec3::new(z, y, -x));
                let f = out.last_mut().and_then(|a| a.frames.last_mut()).ok_or("точка вне кадра")?;
                if points.len() <= 15 {
                    f.points[points.len() - 1] = points[points.len() - 1];
                }
            }
            "control" => {
                let f = out.last_mut().and_then(|a| a.frames.last_mut()).ok_or("метка вне кадра")?;
                let mut c = Control { key: String::new(), value: String::new() };
                for (k, v) in attrs(tag) {
                    match k {
                        "key" => c.key = v.to_owned(),
                        "value" => c.value = v.to_owned(),
                        _ => {}
                    }
                }
                f.controls.push(c);
            }
            _ => {}
        }
    }
    if out.is_empty() {
        return Err("в файле нет анимаций".into());
    }
    Ok(out)
}

impl RwrAnim {
    /// Точки на времени `t` (в единицах анимации): между кадрами — по прямой, как
    /// в RWR; круг после последнего кадра возвращается к первому к `end`.
    pub fn sample(&self, t: f32) -> [Vec3; 15] {
        let f = &self.frames;
        if f.len() == 1 {
            return f[0].points;
        }
        let end = self.end.max(f[f.len() - 1].time);
        let t = if self.looped && end > 0.0 { t.rem_euclid(end) } else { t.clamp(0.0, end) };
        for w in f.windows(2) {
            if t <= w[1].time {
                let k = (t - w[0].time) / (w[1].time - w[0].time).max(1e-5);
                return std::array::from_fn(|i| w[0].points[i].lerp(w[1].points[i], k.clamp(0.0, 1.0)));
            }
        }
        let last = &f[f.len() - 1];
        if self.looped && end > last.time {
            let k = (t - last.time) / (end - last.time);
            return std::array::from_fn(|i| last.points[i].lerp(f[0].points[i], k.clamp(0.0, 1.0)));
        }
        last.points
    }

    /// Секунд в анимации: время делится на скорость проигрывания.
    pub fn seconds(&self) -> f32 {
        self.end / self.speed.max(1e-3)
    }

    /// Метки с временем кадра: (время, ключ, значение).
    pub fn controls(&self) -> Vec<(f32, &str, &str)> {
        self.frames.iter().flat_map(|f| f.controls.iter().map(move |c| (f.time, c.key.as_str(), c.value.as_str()))).collect()
    }

    /// Сводка для разбора: длина, кадры, высота таза, размах стоп и кистей.
    pub fn report(&self) -> String {
        let pt = |f: &RwrFrame, name: &str| f.points[POINTS.iter().position(|p| *p == name).unwrap_or(0)];
        let hips = |f: &RwrFrame| (pt(f, "righthip") + pt(f, "lefthip")) * 0.5;
        let (mut hip_lo, mut hip_hi) = (f32::MAX, f32::MIN);
        let (mut head_lo, mut head_hi) = (f32::MAX, f32::MIN);
        let (mut foot_x_lo, mut foot_x_hi, mut foot_up) = (f32::MAX, f32::MIN, 0.0_f32);
        for f in &self.frames {
            let h = hips(f);
            hip_lo = hip_lo.min(h.y);
            hip_hi = hip_hi.max(h.y);
            let head = pt(f, "head");
            head_lo = head_lo.min(head.y);
            head_hi = head_hi.max(head.y);
            for foot in ["rightfoot", "leftfoot"] {
                let p = pt(f, foot);
                foot_x_lo = foot_x_lo.min(p.x - h.x);
                foot_x_hi = foot_x_hi.max(p.x - h.x);
                foot_up = foot_up.max(p.y);
            }
        }
        let times: Vec<String> = self.frames.iter().map(|f| format!("{:.2}", f.time)).collect();
        let controls: Vec<String> = self.controls().iter().map(|(t, k, v)| format!("{t:.2} {k}={v}")).collect();
        format!(
            "#{} «{}» {} конец {:.2}, скорость {:.2}{} → {:.2} с; кадров {} [{}]\n    таз {:.0}…{:.0} вокс. ({:.2}…{:.2} м), голова {:.0}…{:.0}; стопы от таза по ходу {:.0}…{:.0}, выше всего {:.0}{}",
            self.index,
            self.name,
            if self.looped { "круг," } else { "раз," },
            self.end,
            self.speed,
            self.speed_spread.map_or(String::new(), |s| format!(" ±{s:.2}")),
            self.seconds(),
            self.frames.len(),
            times.join(" "),
            hip_lo,
            hip_hi,
            hip_lo * VOXEL,
            hip_hi * VOXEL,
            head_lo,
            head_hi,
            foot_x_lo,
            foot_x_hi,
            foot_up,
            if controls.is_empty() { String::new() } else { format!("\n    метки: {}", controls.join(", ")) },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_tiny_animation_and_loops_back() {
        let text = r#"<animations><animation loop="1" end="1.0" speed="2.0" comment="шаг">
            <frame time="0.0"><position x="0" y="10" z="0" /><control key="hit_ground" value="1" /></frame>
            <frame time="0.5"><position x="-2" y="20" z="4" /></frame>
            </animation></animations>"#;
        let a = &parse(text).unwrap()[0];
        assert!(a.looped && a.name == "шаг" && a.frames.len() == 2);
        // Оси: RWR (−2, 20, 4) → наши (4, 20, 2).
        assert_eq!(a.frames[1].points[0], Vec3::new(4.0, 20.0, 2.0));
        assert_eq!(a.controls(), vec![(0.0, "hit_ground", "1")]);
        // Посередине между последним кадром и концом — назад к первому.
        assert!((a.sample(0.75)[0].y - 15.0).abs() < 1e-4);
        assert!((a.seconds() - 0.5).abs() < 1e-6);
    }

    /// Если RWR установлена — файл читается целиком (на машине без игры тест молчит).
    #[test]
    fn reads_installed_rwr_if_present() {
        if find_install().is_none() {
            return;
        }
        let all = load_install().unwrap();
        assert!(all.len() > 500);
        let walking = &all[0];
        assert_eq!(walking.name, "walking");
        // Голова выше таза, стопы у земли.
        let p = walking.sample(0.0);
        assert!(p[0].y > p[9].y && p[13].y < 6.0);
    }
}
