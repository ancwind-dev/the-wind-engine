//! Оружие и классы солдат: `data/weapons.ron` (числа оружия) и `data/factions.ron`
//! (кто за какую команду, с какой моделью и оружием, как часто встречается).
//!
//! Набор солдата («класс») выбирается по его номеру — одинаково в симуляции и в
//! отрисовке, поэтому по сети его передавать не нужно. Сервер сверяет отпечаток
//! данных оружия, как отпечаток карты.

use serde::Deserialize;
use std::path::Path;

/// Числа оружия. Названия полей — как в `.weapon`-файлах RWR, где это возможно.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(default)]
pub struct WeaponSpec {
    /// Короткое имя: им оружие называют в `factions.ron`.
    pub name: String,
    /// Модель: путь от `data/models` без `.xml`. Симуляции не нужна.
    pub model: String,
    /// Звук выстрела: `rifle`, `carbine`, `smg`, `launcher` (см. `wind_audio::Gun`).
    /// Пусто — по виду оружия. Симуляции не нужен.
    pub sound: String,
    /// Пауза между выстрелами, с.
    pub retrigger_time: f32,
    pub magazine_size: u32,
    pub reload_time: f32,
    /// Скорость пули или снаряда, м/с.
    pub projectile_speed: f32,
    /// Дальность стрельбы, м.
    pub range: f32,
    /// Разброс в градусах — у первого выстрела очереди.
    pub spread_deg: f32,
    /// Насколько вырастает разброс с каждым выстрелом очереди (доля начального)
    /// и как быстро спадает, долей в секунду (как `sustained_fire_*` в RWR).
    pub sustained_fire_grow_step: f32,
    pub sustained_fire_diminish_rate: f32,
    /// Вероятность, что попадание убьёт сразу.
    pub kill_probability: f32,
    /// Урон, если попадание не убило (здоровье солдата = 1.0).
    pub wound_damage: f32,
    /// Гранатомёт: снаряд летит в точку прицела и взрывается.
    pub blast: Option<BlastSpec>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(default)]
pub struct BlastSpec {
    pub radius: f32,
    /// Вероятность убить в центре взрыва; к краю падает до нуля.
    pub kill_probability: f32,
    /// Урон по стенам в центре взрыва (прочность стены 1..254) и его радиус.
    pub wall_damage: f32,
    pub wall_damage_radius: f32,
}

/// Во сколько раз больше начального может стать разброс в длинной очереди.
pub const MAX_HEAT: f32 = 2.0;

impl WeaponSpec {
    /// Разброс в радианах при «нагреве» очереди `heat`.
    pub fn spread(&self, heat: f32) -> f32 {
        self.spread_deg.to_radians() * (1.0 + heat.clamp(0.0, MAX_HEAT))
    }
}

impl Default for WeaponSpec {
    fn default() -> Self {
        WeaponSpec {
            name: "rifle".into(),
            model: String::new(),
            sound: String::new(),
            retrigger_time: 0.15,
            magazine_size: 30,
            reload_time: 2.5,
            projectile_speed: 90.0,
            range: 50.0,
            spread_deg: 4.0,
            sustained_fire_grow_step: 0.0,
            sustained_fire_diminish_rate: 1.0,
            kill_probability: 0.6,
            wound_damage: 0.4,
            blast: None,
        }
    }
}

impl Default for BlastSpec {
    fn default() -> Self {
        BlastSpec { radius: 4.5, kill_probability: 0.85, wall_damage: 140.0, wall_damage_radius: 2.5 }
    }
}

/// Набор солдата: модель, оружие (номер в `Arsenal::weapons`) и как часто встречается.
#[derive(Clone, Debug, PartialEq)]
pub struct Kit {
    pub model: String,
    pub weapon: usize,
    pub weight: u32,
}

#[derive(Clone, Debug)]
pub struct Arsenal {
    pub weapons: Vec<WeaponSpec>,
    pub teams: [Vec<Kit>; 2],
    pub team_names: [String; 2],
}

#[derive(Deserialize)]
struct WeaponsFile {
    weapons: Vec<WeaponSpec>,
}

#[derive(Deserialize)]
struct FactionsFile {
    teams: Vec<TeamFile>,
}

#[derive(Deserialize)]
struct TeamFile {
    name: String,
    soldiers: Vec<SoldierFile>,
}

#[derive(Deserialize)]
struct SoldierFile {
    model: String,
    weapon: String,
    #[serde(default = "one")]
    weight: u32,
}

fn one() -> u32 {
    1
}

impl Arsenal {
    /// Одно оружие на всех — когда файлов с оружием нет (тесты, старые данные).
    pub fn single(weapon: WeaponSpec) -> Self {
        let kit = Kit { model: String::new(), weapon: 0, weight: 1 };
        Arsenal { weapons: vec![weapon], teams: [vec![kit.clone()], vec![kit]], team_names: ["Синие".into(), "Красные".into()] }
    }

    /// Есть ли в папке данных файлы оружия.
    pub fn exists(data_dir: &Path) -> bool {
        data_dir.join("weapons.ron").exists() && data_dir.join("factions.ron").exists()
    }

    /// Читает `weapons.ron` и `factions.ron` из папки данных.
    pub fn load(data_dir: &Path) -> Result<Self, String> {
        let read = |name: &str| {
            let path = data_dir.join(name);
            crate::config::read_text(&path).map(|t| (path, t))
        };
        let (path, text) = read("weapons.ron")?;
        let weapons: WeaponsFile = ron::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
        let (path, text) = read("factions.ron")?;
        let factions: FactionsFile = ron::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
        if factions.teams.len() != 2 {
            return Err(format!("{}: нужно ровно две команды", path.display()));
        }
        let mut teams: [Vec<Kit>; 2] = Default::default();
        for (team, t) in factions.teams.iter().enumerate() {
            for s in &t.soldiers {
                let weapon = weapons
                    .weapons
                    .iter()
                    .position(|w| w.name == s.weapon)
                    .ok_or_else(|| format!("{}: нет оружия «{}» в weapons.ron", path.display(), s.weapon))?;
                teams[team].push(Kit { model: s.model.clone(), weapon, weight: s.weight.max(1) });
            }
            if teams[team].is_empty() {
                return Err(format!("{}: у команды «{}» нет солдат", path.display(), t.name));
            }
        }
        Ok(Arsenal {
            weapons: weapons.weapons,
            teams,
            team_names: [factions.teams[0].name.clone(), factions.teams[1].name.clone()],
        })
    }

    /// Какой набор у солдата — по его номеру, с учётом весов.
    pub fn pick(&self, team: u8, id: u32) -> usize {
        let list = &self.teams[team as usize & 1];
        let total: u32 = list.iter().map(|k| k.weight).sum();
        let mut r = (id.wrapping_mul(2_654_435_761) >> 8) % total.max(1);
        for (i, k) in list.iter().enumerate() {
            if r < k.weight {
                return i;
            }
            r -= k.weight;
        }
        0
    }

    /// Оружие солдата.
    pub fn weapon_of(&self, team: u8, id: u32) -> &WeaponSpec {
        &self.weapons[self.teams[team as usize & 1][self.pick(team, id)].weapon]
    }

    /// Отпечаток того, что влияет на бой: числа оружия и кто с каким оружием.
    /// Модели и названия команд — только картинка, в отпечаток не входят.
    pub fn fingerprint(&self) -> u64 {
        let mut text = String::new();
        for w in &self.weapons {
            text += &format!("{:?}", WeaponSpec { model: String::new(), sound: String::new(), ..w.clone() });
        }
        for team in &self.teams {
            for k in team {
                text += &format!("|{}:{}", k.weapon, k.weight);
            }
        }
        // FNV-1a.
        text.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ b as u64).wrapping_mul(0x0100_0000_01b3))
    }
}

impl Default for Arsenal {
    fn default() -> Self {
        Arsenal::single(WeaponSpec::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data")
    }

    #[test]
    fn real_data_loads_and_classes_differ() {
        let a = Arsenal::load(&data()).unwrap();
        assert!(a.weapons.len() >= 4);
        // У каждой команды есть и автомат, и гранатомёт.
        for team in 0..2u8 {
            let kits: Vec<_> = (0..200).map(|id| a.weapon_of(team, id)).collect();
            assert!(kits.iter().any(|w| w.blast.is_none()), "у команды {team} нет стрелкового оружия");
            assert!(kits.iter().any(|w| w.blast.is_some()), "у команды {team} нет гранатомёта");
        }
    }

    #[test]
    fn fingerprint_ignores_models_but_not_numbers() {
        let a = Arsenal::load(&data()).unwrap();
        let mut b = a.clone();
        b.weapons[0].model = "другая/модель".into();
        b.weapons[0].sound = "smg".into();
        assert_eq!(a.fingerprint(), b.fingerprint());
        b.weapons[0].magazine_size += 1;
        assert_ne!(a.fingerprint(), b.fingerprint());
    }
}
