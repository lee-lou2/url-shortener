use rand::distributions::{Alphanumeric, DistString};

const ALPHABET: &[u8; 62] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RandomKey {
    V1([u8; 4]),
    Legacy([u8; 2]),
}

impl RandomKey {
    pub fn from_stored(raw: &str) -> Option<Self> {
        let bytes = raw.as_bytes();
        if !bytes.iter().all(u8::is_ascii_alphanumeric) {
            return None;
        }
        match bytes {
            [a, b, c, d] => Some(Self::V1([*a, *b, *c, *d])),
            [a, b] => Some(Self::Legacy([*a, *b])),
            _ => None,
        }
    }
}

pub fn parse_short_key(key: &str) -> Option<(i64, RandomKey)> {
    let bytes = key.as_bytes();
    if !bytes.iter().all(u8::is_ascii_alphanumeric) {
        return None;
    }
    let (middle, random_key) = match bytes.len() {
        5..=15 => {
            let end = bytes.len() - 2;
            (
                &bytes[2..end],
                RandomKey::V1([bytes[0], bytes[1], bytes[end], bytes[end + 1]]),
            )
        }
        3..=4 => {
            let end = bytes.len() - 1;
            (&bytes[1..end], RandomKey::Legacy([bytes[0], bytes[end]]))
        }
        _ => return None,
    };
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
    Some((id, random_key))
}

pub fn encode_short_key(id: i64, random_key: &str) -> Option<String> {
    let key = RandomKey::from_stored(random_key)?;
    if id <= 0 {
        return None;
    }

    let mut value = u64::try_from(id).ok()?;
    let mut buf = [0_u8; 11];
    let mut start = buf.len();
    while value > 0 {
        start -= 1;
        let digit = usize::try_from(value % 62).ok()?;
        buf[start] = ALPHABET[digit];
        value /= 62;
    }
    let (prefix, suffix): (&[u8], &[u8]) = match &key {
        RandomKey::V1(bytes) => (&bytes[..2], &bytes[2..]),
        RandomKey::Legacy(bytes) => (&bytes[..1], &bytes[1..]),
    };
    let mut result = String::with_capacity(prefix.len() + suffix.len() + buf.len() - start);
    result.extend(prefix.iter().copied().map(char::from));
    result.extend(buf[start..].iter().copied().map(char::from));
    result.extend(suffix.iter().copied().map(char::from));
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
            assert_eq!(parse_short_key(key), Some((id, RandomKey::V1(*b"AbXy"))));
        }
        for (id, key) in [(1, "A1Z"), (61, "AzZ"), (132, "A28Z")] {
            assert_eq!(encode_short_key(id, "AZ").as_deref(), Some(key));
            assert_eq!(parse_short_key(key), Some((id, RandomKey::Legacy(*b"AZ"))));
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
            assert_eq!(
                parse_short_key(&expected),
                Some((id, RandomKey::V1(*b"AbXy")))
            );
        }
    }

    #[test]
    fn malformed_keys_never_decode() {
        for key in [
            "ab",
            "A0Z",
            "A01Z",
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
