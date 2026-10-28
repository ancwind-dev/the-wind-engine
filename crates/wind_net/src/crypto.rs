//! Шифрование пакетов после подключения.
//!
//! - **Обмен ключами — X25519.** Клиент и сервер на каждое подключение заводят
//!   свежую пару ключей и обмениваются открытыми: клиент — в «ответе», сервер — в
//!   «приветствии». Общий секрет из Диффи — Хеллмана знают только они двое, даже если
//!   кто-то видел весь обмен.
//! - **Ключи — через BLAKE3** (`derive_key`): свой для каждого направления, чтобы
//!   одинаковые номера пакетов в разные стороны не давали одинаковых nonce.
//! - **Шифр — ChaCha20-Poly1305.** Он и прячет содержимое, и проверяет, что пакет не
//!   подделан: изменённый или чужой пакет просто не расшифруется.
//!
//! Пакет: заголовок (4 байта, открыт, но защищён от подмены как «доп. данные»),
//! номер пакета (8 байт — он же nonce), шифротекст и 16 байт метки.
//! Отправитель увеличивает номер с каждым пакетом — nonce никогда не повторяется.
//! Повтор старого пакета ничего не даёт: ввод и снимки с устаревшим номером
//! отбрасываются выше (по `seq` и тику).
//!
//! Чего нет: сервер не доказывает, что он — тот самый сервер. Кто стоит посередине
//! и подменяет пакеты с самого подключения, может притвориться сервером (записано
//! в долгах).

use crate::protocol::HEADER;
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use x25519_dalek::{PublicKey, StaticSecret};

/// Сколько байт шифрование добавляет к пакету: номер и метка.
pub const OVERHEAD: usize = 8 + 16;

/// Пара ключей на одно подключение.
pub struct Keypair {
    secret: StaticSecret,
    pub public: [u8; 32],
}

impl Keypair {
    /// Свежая пара из случайности операционной системы.
    pub fn generate() -> Keypair {
        let mut bytes = [0u8; 32];
        getrandom::fill(&mut bytes).expect("случайность операционной системы");
        let secret = StaticSecret::from(bytes);
        let public = PublicKey::from(&secret).to_bytes();
        Keypair { secret, public }
    }
}

/// Ключи одной сессии: шифруем исходящие, расшифровываем входящие.
pub struct Session {
    send: ChaCha20Poly1305,
    recv: ChaCha20Poly1305,
    next: u64,
}

impl Session {
    /// Сессия клиента: своя пара и открытый ключ сервера. `None` — ключ сервера
    /// негодный (так бывает, только если его подменили).
    pub fn client(mine: &Keypair, server: [u8; 32]) -> Option<Session> {
        Self::derive(mine, server, mine.public, server, false)
    }

    pub fn server(mine: &Keypair, client: [u8; 32]) -> Option<Session> {
        Self::derive(mine, client, client, mine.public, true)
    }

    fn derive(mine: &Keypair, peer: [u8; 32], client: [u8; 32], server: [u8; 32], is_server: bool) -> Option<Session> {
        let shared = mine.secret.diffie_hellman(&PublicKey::from(peer));
        // Ключ из «плохой» точки кривой даёт всем один и тот же секрет — не принимаем.
        if !shared.was_contributory() {
            return None;
        }
        let mut material = Vec::with_capacity(96);
        material.extend_from_slice(shared.as_bytes());
        material.extend_from_slice(&client);
        material.extend_from_slice(&server);
        let key = |direction: &str| ChaCha20Poly1305::new(Key::from_slice(&blake3::derive_key(direction, &material)));
        let (c2s, s2c) = (key("The Wind Engine 2026 клиент → сервер"), key("The Wind Engine 2026 сервер → клиент"));
        let (send, recv) = if is_server { (s2c, c2s) } else { (c2s, s2c) };
        Some(Session { send, recv, next: 1 })
    }

    fn nonce(n: u64) -> [u8; 12] {
        let mut nonce = [0u8; 12];
        nonce[4..].copy_from_slice(&n.to_le_bytes());
        nonce
    }

    /// Зашифровать готовый пакет (заголовок остаётся открытым).
    pub fn seal(&mut self, packet: &[u8]) -> Vec<u8> {
        let n = self.next;
        self.next += 1;
        let (header, body) = packet.split_at(HEADER);
        let sealed = self.send.encrypt(Nonce::from_slice(&Self::nonce(n)), Payload { msg: body, aad: header }).expect("шифрование не ошибается");
        let mut out = Vec::with_capacity(packet.len() + OVERHEAD);
        out.extend_from_slice(header);
        out.extend_from_slice(&n.to_le_bytes());
        out.extend_from_slice(&sealed);
        out
    }

    /// Расшифровать пакет: заголовок и данные. `None` — пакет подделан, изменён
    /// или зашифрован не нашим ключом.
    pub fn open(&self, packet: &[u8]) -> Option<Vec<u8>> {
        if packet.len() < HEADER + OVERHEAD {
            return None;
        }
        let (header, rest) = packet.split_at(HEADER);
        let n = u64::from_le_bytes(rest[..8].try_into().ok()?);
        let body = self.recv.decrypt(Nonce::from_slice(&Self::nonce(n)), Payload { msg: &rest[8..], aad: header }).ok()?;
        let mut out = Vec::with_capacity(HEADER + body.len());
        out.extend_from_slice(header);
        out.extend_from_slice(&body);
        Some(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol;

    fn pair() -> (Session, Session) {
        let (c, s) = (Keypair::generate(), Keypair::generate());
        (Session::client(&c, s.public).unwrap(), Session::server(&s, c.public).unwrap())
    }

    #[test]
    fn both_sides_agree_and_text_is_hidden() {
        let (mut client, mut server) = pair();
        let packet = protocol::bye();
        let mut long = protocol::begin(protocol::Kind::Snapshot);
        long.write_str("секретное место: x = 123, y = 456", 64);
        let long = long.finish();
        let sealed = server.seal(&long);
        assert_eq!(sealed.len(), long.len() + OVERHEAD);
        assert!(!sealed.windows(6).any(|w| w == "секрет".as_bytes()), "содержимое не читается");
        assert_eq!(client.open(&sealed).unwrap(), long);
        assert_eq!(server.open(&client.seal(&packet)).unwrap(), packet);
        // Номера растут: два одинаковых пакета шифруются по-разному.
        assert_ne!(client.seal(&packet), client.seal(&packet));
    }

    #[test]
    fn tampered_or_foreign_packets_are_rejected() {
        let (mut client, server) = pair();
        let sealed = client.seal(&protocol::bye());
        // Изменён байт данных, номер или заголовок — не открывается.
        for i in [sealed.len() - 1, protocol::HEADER, 3] {
            let mut bad = sealed.clone();
            bad[i] ^= 1;
            assert!(server.open(&bad).is_none(), "изменён байт {i}");
        }
        // Чужая сессия не откроет; своё же направление — тоже (ключи разные).
        let (_, other) = pair();
        assert!(other.open(&sealed).is_none());
        assert!(client.open(&sealed).is_none());
        assert!(server.open(&sealed[..10]).is_none());
        // Негодный ключ сервера (нулевая точка) не принимаем.
        assert!(Session::client(&Keypair::generate(), [0; 32]).is_none());
    }
}
