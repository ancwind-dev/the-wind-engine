//! Перевод интерфейса движка: русский (исходный) и английский.
//!
//! Строки в коде пишем по-русски и оборачиваем: `t("Файл")`, а с подстановками —
//! `f("Сервер запущен на порту {}", &[&port])`. Русский текст и есть ключ: английский
//! перевод лежит в `locale/en.ron` (карта «русский → английский»). Нет перевода —
//! показываем русский; тест `every_string_is_translated` не даёт об этом забыть.
//!
//! Подстановки в шаблоне: `{}` — следующий аргумент, `{:.N}` — с N знаками после запятой.

use std::collections::HashMap;
use std::fmt::Display;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, serde::Serialize, serde::Deserialize)]
pub enum Lang {
    #[default]
    Ru,
    En,
}

impl Lang {
    pub const ALL: [Lang; 2] = [Lang::Ru, Lang::En];

    /// Название языка на нём самом — так его найдёт и тот, кто не читает на текущем.
    pub fn native_name(self) -> &'static str {
        match self {
            Lang::Ru => "Русский",
            Lang::En => "English",
        }
    }
}

/// Язык один на всё приложение: меняется в настройках, читается при каждой отрисовке.
static LANG: AtomicU8 = AtomicU8::new(0);

pub fn set(lang: Lang) {
    LANG.store(lang as u8, Ordering::Relaxed);
}

pub fn lang() -> Lang {
    if LANG.load(Ordering::Relaxed) == Lang::En as u8 { Lang::En } else { Lang::Ru }
}

fn english() -> &'static HashMap<String, String> {
    static TABLE: OnceLock<HashMap<String, String>> = OnceLock::new();
    TABLE.get_or_init(|| {
        // Таблица вшита в программу: движок не зависит от папки, из которой его запустили.
        ron::from_str(include_str!("../locale/en.ron")).expect("locale/en.ron — карта строк")
    })
}

/// Строка на текущем языке.
pub fn t(ru: &'static str) -> &'static str {
    match lang() {
        Lang::Ru => ru,
        Lang::En => english().get(ru).map_or(ru, String::as_str),
    }
}

/// Строка с подстановками на текущем языке.
pub fn f(ru: &'static str, args: &[&dyn Display]) -> String {
    fill(t(ru), args)
}

fn fill(template: &str, args: &[&dyn Display]) -> String {
    let mut out = String::with_capacity(template.len() + 16);
    let mut args = args.iter();
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let tail = &rest[open + 1..];
        let Some(close) = tail.find('}') else {
            out.push_str(&rest[open..]);
            return out;
        };
        let spec = &tail[..close];
        match (spec, args.next()) {
            ("", Some(a)) => out.push_str(&a.to_string()),
            (s, Some(a)) if s.starts_with(":.") => match s[2..].parse::<usize>() {
                Ok(n) => out.push_str(&format!("{a:.n$}")),
                Err(_) => out.push_str(&a.to_string()),
            },
            _ => {}
        }
        rest = &tail[close + 1..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholders_are_filled_in_order_with_precision() {
        assert_eq!(fill("порт {}", &[&7777]), "порт 7777");
        assert_eq!(fill("{:.1} КБ/с · {} мс", &[&2.345_f32, &12]), "2.3 КБ/с · 12 мс");
        assert_eq!(fill("без аргументов", &[]), "без аргументов");
    }

    /// Все строки из `t("…")` и `f("…"` в коде лаборатории есть в английской таблице,
    /// а в таблице нет строк, которых в коде уже нет.
    #[test]
    fn every_string_is_translated() {
        let table = english();
        let mut used = std::collections::HashSet::new();
        let mut missing = Vec::new();
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/src");
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            let text = std::fs::read_to_string(&path).unwrap();
            for key in keys(&text) {
                if !table.contains_key(&key) {
                    missing.push(format!("{}: {key}", path.file_name().unwrap().to_string_lossy()));
                }
                used.insert(key);
            }
        }
        let stale: Vec<&String> = table.keys().filter(|k| !used.contains(*k)).collect();
        assert!(missing.is_empty(), "нет перевода в locale/en.ron:\n{}", missing.join("\n"));
        assert!(stale.is_empty(), "в locale/en.ron лишние строки: {stale:?}");
    }

    /// Русские строки из `t("…")` / `f("…"` в тексте программы (без escape-последовательностей,
    /// кроме `\"`).
    fn keys(text: &str) -> Vec<String> {
        let mut out = Vec::new();
        for opener in ["t(\"", "f(\""] {
            let mut rest = text;
            while let Some(i) = rest.find(opener) {
                // Только сами функции, а не `fmt(` или `assert(`: перед ними не буква и не `_`.
                let before = rest[..i].chars().last();
                rest = &rest[i + opener.len()..];
                if before.is_some_and(|c| c.is_alphanumeric() || c == '_') {
                    continue;
                }
                let mut key = String::new();
                let mut chars = rest.chars();
                while let Some(c) = chars.next() {
                    match c {
                        '"' => break,
                        '\\' => match chars.next() {
                            Some('n') => key.push('\n'),
                            Some('t') => key.push('\t'),
                            Some(n) => key.push(n),
                            None => {}
                        },
                        c => key.push(c),
                    }
                }
                if key.chars().any(|c| matches!(c, 'А'..='я' | 'Ё' | 'ё')) {
                    out.push(key);
                }
            }
        }
        out
    }
}
