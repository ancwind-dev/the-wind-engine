# The Wind Engine

Свой движок на Rust для тактического шутера в духе Running With Rifles.

- Начинай с `notes/Главная.md` и `notes/Как мы работаем.md` — там договорённости, вехи, решения и долги.
- Общение, комментарии и заметки — на русском.
- После работы допиши запись в `notes/Журнал/<дата>.md`, обнови заметку текущей вехи и `notes/Долги.md`.
- Проверка: `cargo clippy --release --all-targets`, `cargo test --release`, замер `cargo run --release -p wind_server -- --bench`.
- Сеть локально: `wind_server --host 127.0.0.1` (без окна брандмауэра), `wind_bots --count 24`, `wind_lab --connect 127.0.0.1:7777`.
