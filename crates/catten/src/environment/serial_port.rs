//! Allocation-free validation of the firmware's serial register contract.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UartKind {
    Pl011,
    Mmio16550,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SerialPort {
    pub kind: UartKind,
    pub base: usize,
}

impl SerialPort {
    /// Only memory-mapped interfaces implemented by the early console qualify.
    /// An unsupported or malformed SPCR must not select a QEMU fallback address.
    pub fn from_spcr(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < 80 || bytes.get(..4)? != b"SPCR" {
            return None;
        }
        let length = u32::from_le_bytes(bytes.get(4..8)?.try_into().ok()?) as usize;
        let table = bytes.get(..length)?;
        if length < 80 || table.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte)) != 0 {
            return None;
        }
        let revision = table[8];
        let interface = table[36];
        // GAS: SystemMemory, zero bit offset. Never interpret port I/O as MMIO.
        if table[40] != 0 || table[42] != 0 {
            return None;
        }
        let kind = match (revision, interface) {
            (2.., 3 | 14) if matches!(table[43], 0 | 3) => UartKind::Pl011,
            (1, 0 | 1) | (2.., 0 | 1 | 0x12) if table[41] == 8 && matches!(table[43], 0 | 1) => {
                UartKind::Mmio16550
            }
            _ => return None,
        };
        let base = usize::try_from(u64::from_le_bytes(table[44..52].try_into().ok()?)).ok()?;
        if base == 0 || (kind == UartKind::Pl011 && !base.is_multiple_of(4)) {
            return None;
        }
        base.checked_add(0x48)?;
        Some(Self {
            kind,
            base,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn checksum(table: &mut [u8]) {
        table[9] = 0;
        table[9] = 0u8.wrapping_sub(table.iter().fold(0u8, |s, b| s.wrapping_add(*b)));
    }

    fn table(kind: u8, width: u8, access: u8, base: u64) -> [u8; 80] {
        let mut table = [0u8; 80];
        table[..4].copy_from_slice(b"SPCR");
        table[4..8].copy_from_slice(&80u32.to_le_bytes());
        table[8] = 2;
        table[36] = kind;
        table[41] = width;
        table[43] = access;
        table[44..52].copy_from_slice(&base.to_le_bytes());
        checksum(&mut table);
        table
    }

    #[test]
    fn firmware_selects_register_family_and_width() {
        for kind in [3, 14] {
            assert_eq!(
                SerialPort::from_spcr(&table(kind, 32, 3, 0x0900_0000)),
                Some(SerialPort {
                    kind: UartKind::Pl011,
                    base: 0x0900_0000
                })
            );
        }
        assert_eq!(
            SerialPort::from_spcr(&table(0x12, 8, 1, 0x3fff_03f8)),
            Some(SerialPort {
                kind: UartKind::Mmio16550,
                base: 0x3fff_03f8
            })
        );
    }

    #[test]
    fn rejects_truncation_checksum_unknown_interface_and_unsafe_gas() {
        let original = table(0x12, 8, 1, 0x3fff_03f8);
        for length in 0..80 {
            assert_eq!(SerialPort::from_spcr(&original[..length]), None);
        }
        for (offset, value) in
            [(0, b'X'), (4, 81), (8, 0), (36, 0xff), (40, 1), (41, 32), (42, 1), (43, 3)]
        {
            let mut changed = original;
            changed[offset] = value;
            checksum(&mut changed);
            assert_eq!(SerialPort::from_spcr(&changed), None, "offset {offset}");
        }
        let mut corrupt = original;
        corrupt[9] ^= 1;
        assert_eq!(SerialPort::from_spcr(&corrupt), None);
        assert_eq!(SerialPort::from_spcr(&table(3, 32, 3, 0)), None);
        assert_eq!(SerialPort::from_spcr(&table(3, 32, 3, 1)), None);
        assert_eq!(SerialPort::from_spcr(&table(0x12, 8, 1, u64::MAX)), None);
    }
}
