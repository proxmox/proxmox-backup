use failure::{bail, Error};

pub fn digest_to_hex(digest: &[u8]) -> String {
    const HEX_CHARS: &'static [u8; 16] = b"0123456789abcdef";

    let mut buf = Vec::<u8>::with_capacity(digest.len() * 2);

    for i in 0..digest.len() {
        buf.push(HEX_CHARS[(digest[i] >> 4) as usize]);
        buf.push(HEX_CHARS[(digest[i] & 0xf) as usize]);
    }

    unsafe { String::from_utf8_unchecked(buf) }
}

pub unsafe fn swapped_data_to_buf<T>(data: &T) -> &[u8] {
    std::slice::from_raw_parts(data as *const T as *const u8, std::mem::size_of::<T>())
}

fn hex_nibble(c: u8) -> Result<u8, Error> {
    Ok(match c {
        b'0'..=b'9' => c - b'0',
        b'a'..=b'f' => c - b'a' + 0xa,
        b'A'..=b'F' => c - b'A' + 0xa,
        _ => bail!("not a hex digit: {}", c as char),
    })
}

#[inline]
pub fn parse_hex_digest<T: AsRef<[u8]>>(hex: T) -> Result<[u8; 32], Error> {
    let mut digest: [u8; 32] = unsafe { std::mem::uninitialized() };

    let hex = hex.as_ref();

    if hex.len() != 64 {
        bail!(
            "invalid hex digest ({} instead of 64 digits long)",
            hex.len()
        );
    }

    for i in 0..32 {
        digest[i] = (hex_nibble(hex[i * 2])? << 4) + hex_nibble(hex[i * 2 + 1])?;
    }

    Ok(digest)
}
