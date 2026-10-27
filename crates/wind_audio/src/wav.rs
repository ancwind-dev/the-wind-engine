//! Запись звука в WAV (16 бит, моно) — чтобы послушать синтез без игры.

use std::io::{self, Write};

pub fn write(w: &mut impl Write, samples: &[f32], rate: u32) -> io::Result<()> {
    let data = (samples.len() * 2) as u32;
    w.write_all(b"RIFF")?;
    w.write_all(&(36 + data).to_le_bytes())?;
    w.write_all(b"WAVEfmt ")?;
    w.write_all(&16u32.to_le_bytes())?;
    // PCM, один канал, частота, байт в секунду, байт на отсчёт, бит на отсчёт.
    w.write_all(&1u16.to_le_bytes())?;
    w.write_all(&1u16.to_le_bytes())?;
    w.write_all(&rate.to_le_bytes())?;
    w.write_all(&(rate * 2).to_le_bytes())?;
    w.write_all(&2u16.to_le_bytes())?;
    w.write_all(&16u16.to_le_bytes())?;
    w.write_all(b"data")?;
    w.write_all(&data.to_le_bytes())?;
    for &s in samples {
        w.write_all(&((s.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn header_has_right_sizes() {
        let mut buf = Vec::new();
        super::write(&mut buf, &[0.0, 0.5, -0.5], 48_000).unwrap();
        assert_eq!(buf.len(), 44 + 6);
        assert_eq!(&buf[0..4], b"RIFF");
        assert_eq!(u32::from_le_bytes(buf[4..8].try_into().unwrap()), 36 + 6);
        assert_eq!(i16::from_le_bytes(buf[46..48].try_into().unwrap()), 16383);
    }
}
