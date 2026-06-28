/// Compute CRC32C checksum over `data` using hardware-accelerated instructions
/// (SSE4.2 on x86, ARM CRC on aarch64) with automatic software fallback.
#[inline]
pub fn crc32c(data: &[u8]) -> u32 {
    crc32c::crc32c(data)
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
