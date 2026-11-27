use rand::distributions::{Alphanumeric, DistString};

const ALPHABET: &[u8; 62] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";

pub fn parse_short_key(key: &str) -> Option<(i64, [u8; 4])> {
    let bytes = key.as_bytes();
    if !(5..=15).contains(&bytes.len()) || !bytes.iter().all(u8::is_ascii_alphanumeric) {
        return None;
    }
    let end = bytes.len() - 2;
    let middle = &bytes[2..end];
    if middle.first() == Some(&b'0') {
        return None;
    }

    let mut id = 0_i64;
    for &byte in middle {
        let digit = match byte {
            b'0'..=b'9' => byte - b'0',
            b'A'..=b'Z' => byte - b'A' + 10,
            _ => byte - b'a' + 36,
        };
        id = id.checked_mul(62)?.checked_add(i64::from(digit))?;
    }
    Some((id, [bytes[0], bytes[1], bytes[end], bytes[end + 1]]))
}

pub fn encode_short_key(id: i64, random_key: &str) -> Option<String> {
    let key: &[u8; 4] = random_key.as_bytes().try_into().ok()?;
    if id <= 0 || !key.iter().all(u8::is_ascii_alphanumeric) {
        return None;
    }

    let mut id = u64::try_from(id).ok()?;
    let mut buf = [0_u8; 11];
    let mut start = buf.len();
    while id > 0 {
        start -= 1;
        let digit = usize::try_from(id % 62).ok()?;
        buf[start] = ALPHABET[digit];
        id /= 62;
    }
    let mut result = String::with_capacity(4 + buf.len() - start);
    result.extend(key[..2].iter().copied().map(char::from));
    result.extend(buf[start..].iter().copied().map(char::from));
    result.extend(key[2..].iter().copied().map(char::from));
    Some(result)
}

pub(crate) fn generate_random_key() -> String {
    Alphanumeric.sample_string(&mut rand::thread_rng(), 4)
}

#[cfg(test)]
mod tests {
    use rand::{rngs::StdRng, Rng, SeedableRng};

    use super::*;

    #[test]
    fn issued_keys_match_golden_vectors() {
        for (id, key) in [
            (62, "Ab10Xy"),
            (12_345, "Ab3D7Xy"),
            (i64::MAX, "AbAzL8n0Y58m7Xy"),
        ] {
            assert_eq!(encode_short_key(id, "AbXy").as_deref(), Some(key));
            assert_eq!(parse_short_key(key), Some((id, *b"AbXy")));
        }
    }

    #[test]
    fn matches_existing_base62_alphabet() {
        let mut rng = StdRng::seed_from_u64(42);
        let random_ids = (0..10_000).map(|_| rng.gen_range(1..=i64::MAX));
        for id in [1, 61, 62, 63, 3843, 3844, i64::MAX]
            .into_iter()
            .chain(random_ids)
        {
            let expected = format!("Ab{}Xy", base62::encode(u64::try_from(id).unwrap()));
            assert_eq!(
                encode_short_key(id, "AbXy").as_deref(),
                Some(expected.as_str())
            );
            assert_eq!(parse_short_key(&expected), Some((id, *b"AbXy")));
        }
    }

    #[test]
    fn malformed_keys_never_decode() {
        for key in [
            "abc",
            "Ab0Xy",
            "Ab0001Xy",
            "AbAzL8n0Y58m8Xy",
            "AbAAAAAAAAAAAAXy",
            "é1Xy",
            "Ab1-Xy",
        ] {
            assert_eq!(parse_short_key(key), None, "{key}");
        }
        assert_eq!(encode_short_key(0, "AbXy"), None);
        assert_eq!(encode_short_key(1, "éxy"), None);
    }
}
