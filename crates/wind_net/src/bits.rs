//! Запись и чтение по битам. Позиция солдата занимает 32 бита, а не 64,
//! флаги — по одному биту: каждый сэкономленный бит умножается на
//! число солдат, клиентов и тиков.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Truncated;

#[derive(Default, Clone)]
pub struct BitWriter {
    bytes: Vec<u8>,
    acc: u64,
    acc_bits: u32,
}

impl BitWriter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Младшие `bits` бит числа (до 32).
    pub fn write_bits(&mut self, value: u32, bits: u32) {
        debug_assert!(bits <= 32);
        let mask = if bits == 32 { u32::MAX } else { (1u32 << bits) - 1 };
        self.acc |= ((value & mask) as u64) << self.acc_bits;
        self.acc_bits += bits;
        while self.acc_bits >= 8 {
            self.bytes.push(self.acc as u8);
            self.acc >>= 8;
            self.acc_bits -= 8;
        }
    }

    pub fn write_bool(&mut self, value: bool) {
        self.write_bits(value as u32, 1);
    }

    /// Число переменной длины: по 7 бит + бит «дальше ещё есть».
    /// Маленькие числа (id, счётчики) занимают 8 бит, большие — больше.
    pub fn write_var(&mut self, mut value: u32) {
        loop {
            self.write_bits(value & 0x7f, 7);
            value >>= 7;
            self.write_bool(value != 0);
            if value == 0 {
                break;
            }
        }
    }

    /// Знаковое число в `bits` бит (дополнительный код).
    pub fn write_signed(&mut self, value: i32, bits: u32) {
        self.write_bits(value as u32, bits);
    }

    pub fn write_f32(&mut self, value: f32) {
        self.write_bits(value.to_bits(), 32);
    }

    pub fn write_u64(&mut self, value: u64) {
        self.write_bits(value as u32, 32);
        self.write_bits((value >> 32) as u32, 32);
    }

    pub fn write_str(&mut self, s: &str, max_bytes: usize) {
        let mut end = s.len().min(max_bytes);
        while !s.is_char_boundary(end) {
            end -= 1;
        }
        self.write_var(end as u32);
        for &b in &s.as_bytes()[..end] {
            self.write_bits(b as u32, 8);
        }
    }

    /// Дописывает биты другого писателя.
    pub fn append(&mut self, other: &BitWriter) {
        for &b in &other.bytes {
            self.write_bits(b as u32, 8);
        }
        if other.acc_bits > 0 {
            self.write_bits(other.acc as u32, other.acc_bits);
        }
    }

    pub fn len_bits(&self) -> usize {
        self.bytes.len() * 8 + self.acc_bits as usize
    }

    pub fn len_bytes(&self) -> usize {
        self.len_bits().div_ceil(8)
    }

    pub fn finish(mut self) -> Vec<u8> {
        if self.acc_bits > 0 {
            self.bytes.push(self.acc as u8);
        }
        self.bytes
    }
}

pub struct BitReader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> BitReader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        BitReader { data, pos: 0 }
    }

    pub fn read_bits(&mut self, bits: u32) -> Result<u32, Truncated> {
        if self.pos + bits as usize > self.data.len() * 8 {
            return Err(Truncated);
        }
        let mut value = 0u64;
        let mut got = 0;
        while got < bits {
            let byte = self.data[self.pos / 8] as u64;
            let offset = (self.pos % 8) as u32;
            let take = (8 - offset).min(bits - got);
            value |= ((byte >> offset) & ((1 << take) - 1)) << got;
            got += take;
            self.pos += take as usize;
        }
        Ok(value as u32)
    }

    pub fn read_bool(&mut self) -> Result<bool, Truncated> {
        Ok(self.read_bits(1)? != 0)
    }

    pub fn read_var(&mut self) -> Result<u32, Truncated> {
        let mut value = 0u32;
        let mut shift = 0;
        loop {
            let part = self.read_bits(7)?;
            if shift < 32 {
                value |= part << shift;
            }
            shift += 7;
            if !self.read_bool()? {
                return Ok(value);
            }
            if shift > 35 {
                return Err(Truncated); // испорченный пакет
            }
        }
    }

    pub fn read_signed(&mut self, bits: u32) -> Result<i32, Truncated> {
        let raw = self.read_bits(bits)?;
        let shift = 32 - bits;
        Ok(((raw << shift) as i32) >> shift)
    }

    pub fn read_f32(&mut self) -> Result<f32, Truncated> {
        Ok(f32::from_bits(self.read_bits(32)?))
    }

    pub fn read_u64(&mut self) -> Result<u64, Truncated> {
        let lo = self.read_bits(32)? as u64;
        let hi = self.read_bits(32)? as u64;
        Ok(lo | (hi << 32))
    }

    pub fn read_str(&mut self, max_bytes: usize) -> Result<String, Truncated> {
        let len = self.read_var()? as usize;
        if len > max_bytes {
            return Err(Truncated);
        }
        let bytes: Result<Vec<u8>, _> = (0..len).map(|_| self.read_bits(8).map(|b| b as u8)).collect();
        Ok(String::from_utf8_lossy(&bytes?).into_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let mut w = BitWriter::new();
        w.write_bits(5, 3);
        w.write_bool(true);
        w.write_var(300);
        w.write_signed(-7, 10);
        w.write_f32(1.5);
        w.write_u64(0xDEAD_BEEF_1234_5678);
        w.write_str("Ветер", 32);
        w.write_bits(u32::MAX, 32);
        let mut other = BitWriter::new();
        other.write_bits(3, 2);
        w.append(&other);
        let bytes = w.finish();

        let mut r = BitReader::new(&bytes);
        assert_eq!(r.read_bits(3), Ok(5));
        assert_eq!(r.read_bool(), Ok(true));
        assert_eq!(r.read_var(), Ok(300));
        assert_eq!(r.read_signed(10), Ok(-7));
        assert_eq!(r.read_f32(), Ok(1.5));
        assert_eq!(r.read_u64(), Ok(0xDEAD_BEEF_1234_5678));
        assert_eq!(r.read_str(32).as_deref(), Ok("Ветер"));
        assert_eq!(r.read_bits(32), Ok(u32::MAX));
        assert_eq!(r.read_bits(2), Ok(3));
        assert!(r.read_bits(8).is_err());
    }
}
