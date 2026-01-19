//! Base62 encoding utilities used for slug generation.

const ALPHABET: &[u8; 62] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";

/// Returns the base62 alphabet as bytes.
pub fn alphabet() -> &'static [u8] {
    &ALPHABET[..]
}

/// Encode an unsigned 64-bit integer into a base62 string using the alphabet
/// 0-9, A-Z, a-z. Zero encodes to "0".
pub fn encode_u64(mut n: u64) -> String {
    if n == 0 {
        return "0".to_string();
    }
    // Worst-case length for u64 in base62 is 11 characters (since 62^11 > 2^64)
    let mut buf = [0u8; 22]; // spare room
    let mut i = buf.len();
    while n > 0 {
        let rem = (n % 62) as usize;
        i -= 1;
        buf[i] = ALPHABET[rem];
        n /= 62;
    }
    String::from_utf8(buf[i..].to_vec()).expect("valid ascii from alphabet")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alphabet_len() {
        assert_eq!(alphabet().len(), 62);
    }

    #[test]
    fn encodes_known_vectors() {
        assert_eq!(encode_u64(0), "0");
        assert_eq!(encode_u64(61), "z");
        assert_eq!(encode_u64(62), "10");
        assert_eq!(encode_u64(63), "11");
        assert_eq!(encode_u64(3843), "zz"); // 62*62-1
    }
}
