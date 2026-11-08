//! Bounded parsing of FFmpeg's serialized AVEncryptionInfo, without content keys.
// Format reference: FFmpeg libavutil/encryption_info.c.

#[derive(Debug, PartialEq, Eq)]
pub struct PacketEncryption<'a> {
    pub key_id: &'a [u8],
    pub iv: &'a [u8],
    pub subsamples: Vec<(u32, u32)>,
}

impl<'a> PacketEncryption<'a> {
    pub fn parse(bytes: &'a [u8], packet_size: usize) -> Result<Self, &'static str> {
        const INVALID: &str = "invalid or unsupported CENC packet metadata";
        if bytes.len() < 24 || packet_size == 0 || packet_size > 1024 * 1024 {
            return Err(INVALID);
        }
        let word = |offset: usize| {
            u32::from_be_bytes(
                bytes[offset..offset + 4]
                    .try_into()
                    .expect("checked CENC header"),
            )
        };
        if &bytes[..4] != b"cenc" || word(4) != 0 || word(8) != 0 {
            return Err(INVALID);
        }
        let key_size = word(12) as usize;
        let iv_size = word(16) as usize;
        let count = word(20) as usize;
        if key_size != 16 || !matches!(iv_size, 8 | 16) || count > 256 {
            return Err(INVALID);
        }
        let offset = 24 + key_size + iv_size;
        if bytes.len() != offset + count * 8 {
            return Err(INVALID);
        }
        let mut subsamples = Vec::with_capacity(count);
        let mut total = 0u64;
        for entry in bytes[offset..].as_chunks::<8>().0 {
            let clear = u32::from_be_bytes(entry[..4].try_into().expect("CENC entry"));
            let cipher = u32::from_be_bytes(entry[4..].try_into().expect("CENC entry"));
            total += u64::from(clear) + u64::from(cipher);
            subsamples.push((clear, cipher));
        }
        if count != 0 && total != packet_size as u64 {
            return Err(INVALID);
        }
        Ok(Self {
            key_id: &bytes[24..40],
            iv: &bytes[40..offset],
            subsamples,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(subs: &[(u32, u32)]) -> Vec<u8> {
        let mut bytes = b"cenc".to_vec();
        for value in [0, 0, 16, 16, subs.len() as u32] {
            bytes.extend_from_slice(&value.to_be_bytes());
        }
        bytes.extend_from_slice(&[7; 16]);
        bytes.extend_from_slice(&[9; 16]);
        for (clear, cipher) in subs {
            bytes.extend_from_slice(&clear.to_be_bytes());
            bytes.extend_from_slice(&cipher.to_be_bytes());
        }
        bytes
    }

    #[test]
    fn accepts_full_sample_and_subsample_encryption() {
        let whole = fixture(&[]);
        let parsed = PacketEncryption::parse(&whole, 100).unwrap();
        assert_eq!(parsed.key_id, &[7; 16]);
        assert_eq!(parsed.iv, &[9; 16]);
        assert!(parsed.subsamples.is_empty());
        let split = fixture(&[(4, 96)]);
        assert_eq!(
            PacketEncryption::parse(&split, 100).unwrap().subsamples,
            [(4, 96)]
        );
    }

    #[test]
    fn rejects_truncation_and_packet_size_mismatch() {
        let bytes = fixture(&[(4, 96)]);
        for end in 0..bytes.len() {
            assert!(PacketEncryption::parse(&bytes[..end], 100).is_err());
        }
        assert!(PacketEncryption::parse(&bytes, 99).is_err());
        assert!(PacketEncryption::parse(&bytes, 101).is_err());
        assert!(PacketEncryption::parse(&fixture(&[]), 1024 * 1024 + 1).is_err());
    }

    #[test]
    fn refuses_other_schemes_patterns_and_oversized_metadata() {
        let mut bytes = fixture(&[]);
        bytes[..4].copy_from_slice(b"cbcs");
        assert!(PacketEncryption::parse(&bytes, 100).is_err());
        bytes[..4].copy_from_slice(b"cenc");
        bytes[7] = 1;
        assert!(PacketEncryption::parse(&bytes, 100).is_err());
        let mut bytes = fixture(&[]);
        bytes[20..24].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(PacketEncryption::parse(&bytes, 100).is_err());
    }
}
