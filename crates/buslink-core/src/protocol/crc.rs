//! CRC-16/MODBUS: init 0xFFFF, reflected polynomial 0xA001, no final XOR,
//! transmitted low byte first.

pub fn crc16(data: &[u8]) -> u16 {
    let mut crc: u16 = 0xFFFF;
    for &byte in data {
        crc ^= u16::from(byte);
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xA001 } else { crc >> 1 };
        }
    }
    crc
}

/// Append the CRC of `frame` (low byte first).
pub fn append(frame: &mut Vec<u8>) {
    let crc = crc16(frame);
    frame.extend_from_slice(&crc.to_le_bytes());
}

/// Split an ADU into body and the CRC it carries; `None` if shorter than 3 bytes.
pub fn split(adu: &[u8]) -> Option<(&[u8], u16)> {
    if adu.len() < 3 {
        return None;
    }
    let (body, tail) = adu.split_at(adu.len() - 2);
    Some((body, u16::from_le_bytes([tail[0], tail[1]])))
}

/// True if the last two bytes are the correct CRC of the rest.
pub fn is_valid(adu: &[u8]) -> bool {
    split(adu).is_some_and(|(body, crc)| crc16(body) == crc)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_value() {
        assert_eq!(crc16(b"123456789"), 0x4B37);
    }

    #[test]
    fn known_read_request() {
        let mut f = vec![0x01, 0x03, 0x00, 0x00, 0x00, 0x08];
        append(&mut f);
        assert_eq!(f, vec![0x01, 0x03, 0x00, 0x00, 0x00, 0x08, 0x44, 0x0C]);
        assert!(is_valid(&f));
        f[7] ^= 1;
        assert!(!is_valid(&f));
        assert!(!is_valid(&[0x01, 0x02]));
    }
}
