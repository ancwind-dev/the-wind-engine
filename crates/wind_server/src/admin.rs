//! Управление сервером из консоли: игроки, кик, бан, пароль, имя. И список банов
//! в простом текстовом файле — его можно поправить и руками.

use std::net::IpAddr;
use std::path::{Path, PathBuf};
use wind_net::NetServer;
use wind_sim::World;

/// Баны: по адресу на строку, после `#` — кто и за что.
///
/// ```text
/// 203.0.113.7   # Читер — стрелял сквозь стены
/// ```
pub struct Bans {
    path: PathBuf,
    pub list: Vec<(IpAddr, String)>,
}

impl Bans {
    /// Читает файл банов; нет файла — пустой список.
    pub fn load(path: &Path) -> Result<Bans, String> {
        let mut list = Vec::new();
        if path.exists() {
            let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
            for (n, line) in text.lines().enumerate() {
                let (addr, note) = line.split_once('#').unwrap_or((line, ""));
                let addr = addr.trim();
                if addr.is_empty() {
                    continue;
                }
                let ip = addr.parse().map_err(|_| format!("{}, строка {}: «{addr}» — не адрес", path.display(), n + 1))?;
                list.push((ip, note.trim().to_owned()));
            }
        }
        Ok(Bans { path: path.to_owned(), list })
    }

    pub fn save(&self) -> Result<(), String> {
        let mut text = String::from("# Баны сервера: адрес и после # — кто и за что. Правится и руками.\n");
        for (ip, note) in &self.list {
            text += &format!("{ip}   # {note}\n");
        }
        std::fs::write(&self.path, text).map_err(|e| format!("{}: {e}", self.path.display()))
    }

    pub fn ips(&self) -> Vec<IpAddr> {
        self.list.iter().map(|(ip, _)| *ip).collect()
    }
}

/// Что сделала команда: ответ для консоли и не пора ли остановить сервер.
pub struct Outcome {
    pub lines: Vec<String>,
    pub quit: bool,
}

const HELP: &str = "Команды:\n  игроки — кто на сервере\n  кик <номер> [причина] — выкинуть\n  бан <номер> [причина] — выкинуть и больше не пускать с этого адреса\n  разбан <адрес> — снять бан\n  баны — список банов\n  пароль [новый] — поставить пароль (без слова — снять)\n  имя <новое> — переименовать сервер\n  стоп — остановить сервер\nПо-английски тоже можно: players, kick, ban, unban, bans, password, name, quit.";

/// Выполнить команду хозяина сервера.
pub fn command(line: &str, net: &mut NetServer, world: &mut World, bans: &mut Bans) -> Outcome {
    let mut out = Outcome { lines: Vec::new(), quit: false };
    let line = line.trim();
    let (word, rest) = line.split_once(char::is_whitespace).map_or((line, ""), |(w, r)| (w, r.trim()));
    // Номер игрока и причина: «кик 3 мешает играть».
    let target = |rest: &str| -> Option<(u16, String)> {
        let (id, reason) = rest.split_once(char::is_whitespace).map_or((rest, ""), |(a, b)| (a, b.trim()));
        Some((id.parse().ok()?, if reason.is_empty() { "без причины".into() } else { reason.to_owned() }))
    };
    match word.to_lowercase().as_str() {
        "" => {}
        "помощь" | "help" | "?" => out.lines.extend(HELP.lines().map(str::to_owned)),
        "игроки" | "players" => {
            if net.clients.is_empty() {
                out.lines.push("На сервере никого".into());
            }
            for c in &net.clients {
                let s = &world.soldiers[c.replica.soldier as usize];
                out.lines.push(format!("  {:>3}  {:<24} {}  {}  убийств {}", c.id, c.name, ["синие", "красные"][c.team as usize & 1], c.addr, s.kills));
            }
        }
        "кик" | "kick" => match target(rest) {
            Some((id, reason)) if net.kick(id, &reason, world) => out.lines.push(format!("Игрок {id} выкинут: {reason}")),
            _ => out.lines.push("Кого? Например: кик 3 мешает играть (номера — в «игроки»)".into()),
        },
        "бан" | "ban" => match target(rest) {
            Some((id, reason)) => {
                let name = net.clients.iter().find(|c| c.id == id).map(|c| c.name.clone());
                match (net.ban(id, &reason, world), name) {
                    (Some(addr), Some(name)) => {
                        bans.list.retain(|(ip, _)| *ip != addr.ip());
                        bans.list.push((addr.ip(), format!("{name} — {reason}")));
                        out.lines.push(format!("{name} ({}) забанен: {reason}", addr.ip()));
                        if let Err(e) = bans.save() {
                            out.lines.push(format!("Список банов не сохранился: {e}"));
                        }
                    }
                    _ => out.lines.push(format!("Игрока {id} нет — номера в «игроки»")),
                }
            }
            None => out.lines.push("Кого? Например: бан 3 читы".into()),
        },
        "разбан" | "unban" => match rest.parse::<IpAddr>() {
            Ok(ip) if bans.list.iter().any(|(b, _)| *b == ip) => {
                bans.list.retain(|(b, _)| *b != ip);
                net.banned = bans.ips();
                out.lines.push(format!("{ip} разбанен"));
                if let Err(e) = bans.save() {
                    out.lines.push(format!("Список банов не сохранился: {e}"));
                }
            }
            Ok(ip) => out.lines.push(format!("{ip} не в банах")),
            Err(_) => out.lines.push("Какой адрес? Например: разбан 203.0.113.7 (список — «баны»)".into()),
        },
        "баны" | "bans" => {
            if bans.list.is_empty() {
                out.lines.push("Банов нет".into());
            }
            for (ip, note) in &bans.list {
                out.lines.push(format!("  {ip}   {note}"));
            }
        }
        "пароль" | "password" => {
            net.password = rest.to_owned();
            out.lines.push(if rest.is_empty() { "Пароль снят".into() } else { "Пароль поставлен — новым игрокам нужен он".into() });
        }
        "имя" | "name" if !rest.is_empty() => {
            net.name = rest.chars().take(wind_net::protocol::MAX_SERVER_NAME).collect();
            out.lines.push(format!("Сервер теперь называется «{}» (увидят новые игроки)", net.name));
        }
        "стоп" | "quit" | "exit" => {
            out.lines.push("Останавливаю сервер".into());
            out.quit = true;
        }
        other => out.lines.push(format!("Не знаю команду «{other}». «помощь» — список команд")),
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use wind_net::ReplicationConfig;
    use wind_sim::SimConfig;

    #[test]
    fn bans_file_round_trip() {
        let dir = std::env::temp_dir().join(format!("wind_bans_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("bans.txt");
        assert!(Bans::load(&path).unwrap().list.is_empty());
        std::fs::write(&path, "# комментарий\n203.0.113.7  # Читер — сквозь стены\n\n::1\n").unwrap();
        let mut bans = Bans::load(&path).unwrap();
        assert_eq!(bans.list.len(), 2);
        assert_eq!(bans.list[0].1, "Читер — сквозь стены");
        bans.list.push(("10.0.0.1".parse().unwrap(), "ещё".into()));
        bans.save().unwrap();
        assert_eq!(Bans::load(&path).unwrap().list, bans.list);
        std::fs::write(&path, "не-адрес\n").unwrap();
        assert!(Bans::load(&path).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn console_commands() {
        let dir = std::env::temp_dir().join(format!("wind_admin_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut bans = Bans::load(&dir.join("bans.txt")).unwrap();
        let mut net = NetServer::bind("127.0.0.1:0", ReplicationConfig::default()).unwrap();
        let mut world = World::new(SimConfig { map_size: 128, soldiers_per_team: 2, capture_points: 1, ..Default::default() }, 1);
        let mut run = |line: &str, net: &mut NetServer| command(line, net, &mut world, &mut bans);
        assert!(run("игроки", &mut net).lines[0].contains("никого"));
        assert!(run("кик 5", &mut net).lines[0].contains("Кого?"));
        run("пароль тайна", &mut net);
        assert_eq!(net.password, "тайна");
        run("password", &mut net);
        assert!(net.password.is_empty());
        run("имя Сервер Артёма", &mut net);
        assert_eq!(net.name, "Сервер Артёма");
        assert!(run("разбан 1.2.3.4", &mut net).lines[0].contains("не в банах"));
        assert!(run("абракадабра", &mut net).lines[0].contains("Не знаю"));
        assert!(run("стоп", &mut net).quit);
        std::fs::remove_dir_all(&dir).ok();
    }
}
