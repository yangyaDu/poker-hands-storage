/// CRC32C (Castagnoli) lookup table — computed at compile time.
///
/// Polynomial: 0x82F63B78 (Castagnoli, used by iSCSI / SCTP / SSE4.2)
///
/// Standard test vector: `"123456789"` → `0xE3069283`
const CRC32C_TABLE: [u32; 256] = {
    let mut table = [0u32; 256];
    let poly: u32 = 0x82F63B78;
    let mut i = 0u32;
    while i < 256 {
        let mut crc = i;
        let mut bit = 0;
        while bit < 8 {
            crc = if (crc & 1) != 0 {
                poly ^ (crc >> 1)
            } else {
                crc >> 1
            };
            bit += 1;
        }
        table[i as usize] = crc;
        i += 1;
    }
    table
};

/// Compute CRC32C checksum over `data`.
///
/// Returns the 32-bit unsigned checksum (matches JS crc32c implementation).
#[inline]
pub fn crc32c(data: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for &byte in data {
        let idx = ((crc ^ byte as u32) & 0xFF) as usize;
        crc = CRC32C_TABLE[idx] ^ (crc >> 8);
    }
    crc ^ 0xFFFF_FFFF
}

/// Verify that data's CRC32C matches the expected checksum.
///
/// Returns `Ok(())` on match, `Err(reason)` on mismatch.
#[inline]
pub fn assert_crc32c(data: &[u8], expected: u32) -> Result<(), String> {
    let actual = crc32c(data);
    if actual == expected {
        Ok(())
    } else {
        Err(format!(
            "CRC32C mismatch: expected {}, got {}",
            expected, actual
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty() {
        // CRC32C of empty data with initial 0xFFFFFFFF → final XOR
        assert_eq!(crc32c(b""), 0x0000_0000);
    }

    #[test]
    fn test_known_vector() {
        // Standard iSCSI test vector: "123456789" → 0xE3069283
        assert_eq!(crc32c(b"123456789"), 0xE306_9283);
    }

    #[test]
    fn test_assert_match() {
        let data = b"hello world";
        let checksum = crc32c(data);
        assert!(assert_crc32c(data, checksum).is_ok());
    }

    #[test]
    fn test_assert_mismatch() {
        let data = b"hello world";
        assert!(assert_crc32c(data, 0xDEAD_BEEF).is_err());
    }
}
