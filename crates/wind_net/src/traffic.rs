//! Счётчик трафика: байты и пакеты в обе стороны, скорость за последнюю секунду.

use std::time::Instant;

#[derive(Clone, Debug)]
pub struct Traffic {
    pub sent_bytes: u64,
    pub recv_bytes: u64,
    pub sent_packets: u64,
    pub recv_packets: u64,
    /// Самые большие отправленный и полученный пакеты за последнюю секунду.
    pub max_sent_packet: usize,
    pub max_recv_packet: usize,
    /// Байт в секунду за последнюю полную секунду.
    pub sent_per_sec: f32,
    pub recv_per_sec: f32,
    window_start: Instant,
    window_sent: u64,
    window_recv: u64,
    window_max_sent: usize,
    window_max_recv: usize,
}

impl Default for Traffic {
    fn default() -> Self {
        Traffic {
            sent_bytes: 0,
            recv_bytes: 0,
            sent_packets: 0,
            recv_packets: 0,
            max_sent_packet: 0,
            max_recv_packet: 0,
            sent_per_sec: 0.0,
            recv_per_sec: 0.0,
            window_start: Instant::now(),
            window_sent: 0,
            window_recv: 0,
            window_max_sent: 0,
            window_max_recv: 0,
        }
    }
}

impl Traffic {
    pub fn on_send(&mut self, bytes: usize) {
        self.roll();
        self.sent_bytes += bytes as u64;
        self.sent_packets += 1;
        self.window_sent += bytes as u64;
        self.window_max_sent = self.window_max_sent.max(bytes);
    }

    pub fn on_recv(&mut self, bytes: usize) {
        self.roll();
        self.recv_bytes += bytes as u64;
        self.recv_packets += 1;
        self.window_recv += bytes as u64;
        self.window_max_recv = self.window_max_recv.max(bytes);
    }

    /// Раз в секунду переносит накопленное в скорости.
    pub fn roll(&mut self) {
        let elapsed = self.window_start.elapsed().as_secs_f32();
        if elapsed < 1.0 {
            return;
        }
        self.sent_per_sec = self.window_sent as f32 / elapsed;
        self.recv_per_sec = self.window_recv as f32 / elapsed;
        self.max_sent_packet = self.window_max_sent;
        self.max_recv_packet = self.window_max_recv;
        self.window_start = Instant::now();
        self.window_sent = 0;
        self.window_recv = 0;
        self.window_max_sent = 0;
        self.window_max_recv = 0;
    }
}
