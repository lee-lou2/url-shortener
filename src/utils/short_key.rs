//! 단축키 인코딩/디코딩 모듈.

/// Length of the random prefix (first part of short key).
pub const RAND_PREFIX_LEN: usize = 2;

/// Length of the random suffix (last part of short key).
pub const RAND_SUFFIX_LEN: usize = 2;

/// Total length of random key stored in database.
pub const RANDOM_KEY_LEN: usize = RAND_PREFIX_LEN + RAND_SUFFIX_LEN;

/// Minimum length of a valid short key (prefix + at least 1 char for ID + suffix).
pub const SHORT_KEY_MIN_LEN: usize = RAND_PREFIX_LEN + 1 + RAND_SUFFIX_LEN;

/// Merges a random key with a Base62-encoded ID.
///
/// The random key is split into prefix and suffix, wrapping the encoded ID.
///
/// # Arguments
///
/// * `rand_key` - 4-character random key (first 2 chars as prefix, last 2 as suffix)
/// * `id` - Numeric ID to encode
///
/// # Returns
///
/// A short key string (e.g., "`Ab3D7Xy`" where "Ab" is prefix, "3D7" is encoded ID, "Xy" is suffix)
#[must_use]
pub fn merge_short_key(rand_key: &str, id: u64) -> String {
    let encoded = base62::encode(id);

    // Split random key into prefix (first 2) and suffix (last 2)
    let prefix = if rand_key.len() >= RAND_PREFIX_LEN {
        &rand_key[..RAND_PREFIX_LEN]
    } else {
        rand_key
    };

    let suffix = if rand_key.len() >= RANDOM_KEY_LEN {
        &rand_key[RAND_PREFIX_LEN..RANDOM_KEY_LEN]
    } else if rand_key.len() > RAND_PREFIX_LEN {
        &rand_key[RAND_PREFIX_LEN..]
    } else {
        ""
    };

    format!("{prefix}{encoded}{suffix}")
}

/// Splits a short key into its random key and decoded ID.
///
/// Extracts prefix (first 2 chars) and suffix (last 2 chars), then decodes the middle part.
///
/// # Arguments
///
/// * `short_key` - The short key to decode
///
/// # Returns
///
/// A tuple of (`decoded_id`, `random_key`). Returns (0, "") if decoding fails.
/// The `random_key` is reconstructed as prefix + suffix (4 chars total).
#[must_use]
pub fn split_short_key(short_key: &str) -> (u64, String) {
    // Check minimum length and ensure all characters are ASCII
    if short_key.len() < SHORT_KEY_MIN_LEN || !short_key.is_ascii() {
        return (0, String::new());
    }

    let prefix = &short_key[..RAND_PREFIX_LEN];
    let suffix = &short_key[short_key.len() - RAND_SUFFIX_LEN..];
    let encoded_id = &short_key[RAND_PREFIX_LEN..short_key.len() - RAND_SUFFIX_LEN];

    base62::decode(encoded_id).map_or_else(
        |_| (0, String::new()),
        |id| {
            // base62 returns u128, convert to u64 safely
            let id_u64 = u64::try_from(id).unwrap_or(0);
            // Reconstruct random_key as prefix + suffix
            let rand_key = format!("{prefix}{suffix}");
            (id_u64, rand_key)
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    // ============ 상수 테스트 ============

    #[test]
    fn test_constants() {
        assert_eq!(RAND_PREFIX_LEN, 2);
        assert_eq!(RAND_SUFFIX_LEN, 2);
        assert_eq!(RANDOM_KEY_LEN, 4);
        assert_eq!(SHORT_KEY_MIN_LEN, 5);
    }

    // ============ merge_short_key 테스트 ============

    #[test]
    fn test_merge_short_key_basic() {
        let result = merge_short_key("AbXy", 12345);
        assert!(result.starts_with("Ab"));
        assert!(result.ends_with("Xy"));
        assert!(result.len() >= SHORT_KEY_MIN_LEN);
    }

    #[test]
    fn test_merge_short_key_id_zero() {
        let result = merge_short_key("AbXy", 0);
        assert_eq!(result, "Ab0Xy");
    }

    #[test]
    fn test_merge_short_key_id_one() {
        let result = merge_short_key("ZzAa", 1);
        assert_eq!(result, "Zz1Aa");
    }

    #[test]
    fn test_merge_short_key_large_id() {
        let result = merge_short_key("AbCd", u64::MAX);
        assert!(result.starts_with("Ab"));
        assert!(result.ends_with("Cd"));
        assert!(result.len() > RANDOM_KEY_LEN);
    }

    #[test]
    fn test_merge_short_key_structure() {
        // ID 62 encodes to "10" in base62
        let result = merge_short_key("PrSf", 62);
        assert_eq!(result, "Pr10Sf");
    }

    // ============ split_short_key 테스트 ============

    #[test]
    fn test_split_short_key_valid() {
        let rand_key = "AbXy";
        let id: u64 = 999;
        let short_key = merge_short_key(rand_key, id);

        let (decoded_id, decoded_rand) = split_short_key(&short_key);
        assert_eq!(decoded_id, id);
        assert_eq!(decoded_rand, rand_key);
    }

    #[test]
    fn test_split_short_key_too_short() {
        // Less than 5 chars (minimum: 2 prefix + 1 id + 2 suffix)
        let (id, rand_key) = split_short_key("abcd");
        assert_eq!(id, 0);
        assert!(rand_key.is_empty());
    }

    #[test]
    fn test_split_short_key_empty() {
        let (id, rand_key) = split_short_key("");
        assert_eq!(id, 0);
        assert!(rand_key.is_empty());
    }

    #[test]
    fn test_split_short_key_exactly_minimum() {
        // "Ab0Xy" -> prefix="Ab", encoded_id="0", suffix="Xy" -> id=0, rand_key="AbXy"
        let (id, rand_key) = split_short_key("Ab0Xy");
        assert_eq!(id, 0);
        assert_eq!(rand_key, "AbXy");
    }

    #[test]
    fn test_split_short_key_invalid_base62() {
        // 유효하지 않은 Base62 문자 포함
        let (id, rand_key) = split_short_key("Ab!@#Xy");
        assert_eq!(id, 0);
        assert!(rand_key.is_empty());
    }

    #[test]
    fn test_roundtrip_various_ids() {
        for id in [1, 10, 100, 1000, 10000, 100_000, 1_000_000, 10_000_000] {
            let rand_key = "ZzAa";
            let short_key = merge_short_key(rand_key, id);
            let (decoded_id, decoded_rand) = split_short_key(&short_key);
            assert_eq!(decoded_id, id, "ID mismatch for {id}");
            assert_eq!(decoded_rand, rand_key);
        }
    }

    #[test]
    fn test_roundtrip_edge_cases() {
        for id in [0, 1, u64::MAX / 2, u64::MAX - 1] {
            let rand_key = "AaBb";
            let short_key = merge_short_key(rand_key, id);
            let (decoded_id, decoded_rand) = split_short_key(&short_key);
            assert_eq!(decoded_id, id, "ID mismatch for {id}");
            assert_eq!(decoded_rand, rand_key);
        }
    }

    #[test]
    fn test_different_rand_keys() {
        let test_cases = [
            ("0011", 1),
            ("AaBb", 100),
            ("ZzYy", 1000),
            ("aZbY", 10000),
            ("zAxB", 100_000),
            ("9900", 1_000_000),
        ];

        for (rand_key, id) in test_cases {
            let short_key = merge_short_key(rand_key, id);
            let (decoded_id, decoded_rand) = split_short_key(&short_key);
            assert_eq!(decoded_id, id);
            assert_eq!(decoded_rand, rand_key);
        }
    }

    #[test]
    fn test_short_key_length_growth() {
        // ID가 커질수록 short_key 길이가 증가하는지 확인
        let len_1 = merge_short_key("AbCd", 1).len();
        let len_1000 = merge_short_key("AbCd", 1000).len();
        let len_1m = merge_short_key("AbCd", 1_000_000).len();
        let len_1b = merge_short_key("AbCd", 1_000_000_000).len();

        assert!(len_1 <= len_1000);
        assert!(len_1000 <= len_1m);
        assert!(len_1m <= len_1b);
    }

    #[test]
    fn test_unicode_rand_key_split() {
        // 유니코드 문자는 안전하게 (0, "") 반환
        let (id, rand_key) = split_short_key("한글abcXy");
        assert_eq!(id, 0);
        assert!(rand_key.is_empty());
    }

    #[test]
    fn test_non_ascii_input_handled() {
        // ASCII가 아닌 문자 포함 시 (0, "") 반환
        let (id, rand_key) = split_short_key("Aé12Xy");
        assert_eq!(id, 0);
        assert!(rand_key.is_empty());
    }

    #[test]
    fn test_emoji_input_handled() {
        // 이모지 포함 시 (0, "") 반환
        let (id, rand_key) = split_short_key("🚀ab1Xy");
        assert_eq!(id, 0);
        assert!(rand_key.is_empty());
    }

    #[test]
    fn test_base62_encoding_consistency() {
        // 동일한 입력은 항상 동일한 출력
        let key1 = merge_short_key("XxYy", 12345);
        let key2 = merge_short_key("XxYy", 12345);
        assert_eq!(key1, key2);
    }

    #[test]
    fn test_split_preserves_rand_key_case() {
        let short_key = merge_short_key("aBcD", 100);
        let (_, decoded_rand) = split_short_key(&short_key);
        assert_eq!(decoded_rand, "aBcD");
    }

    // ============ 추가 merge_short_key 테스트 ============

    #[test]
    fn test_merge_short_key_numeric_rand_key() {
        let result = merge_short_key("1234", 999);
        assert!(result.starts_with("12"));
        assert!(result.ends_with("34"));
        let (id, rand) = split_short_key(&result);
        assert_eq!(id, 999);
        assert_eq!(rand, "1234");
    }

    #[test]
    fn test_merge_short_key_short_rand_key() {
        // 4자 미만의 랜덤 키는 가능한 만큼만 사용
        let result = merge_short_key("Ab", 123);
        assert!(result.starts_with("Ab"));
        // suffix가 없으므로 끝에 아무것도 붙지 않음
    }

    #[test]
    fn test_merge_short_key_special_id_values() {
        // 2의 거듭제곱 근처 값들
        let special_ids = [
            1, 61,      // Base62에서 한 자리 최대
            62,      // Base62에서 두 자리 시작
            3843,    // 62^2 - 1
            3844,    // 62^2
            238_327, // 62^3 - 1
        ];

        for id in special_ids {
            let short_key = merge_short_key("XxYy", id);
            let (decoded, rand) = split_short_key(&short_key);
            assert_eq!(decoded, id, "Failed for id: {id}");
            assert_eq!(rand, "XxYy");
        }
    }

    // ============ 추가 split_short_key 테스트 ============

    #[test]
    fn test_split_short_key_whitespace_input() {
        let (id, rand) = split_short_key("     ");
        assert_eq!(id, 0);
        assert!(rand.is_empty());
    }

    #[test]
    fn test_split_short_key_tab_input() {
        let (id, rand) = split_short_key("\t\t\t\t\t");
        assert_eq!(id, 0);
        assert!(rand.is_empty());
    }

    #[test]
    fn test_split_short_key_mixed_valid_invalid() {
        // 앞뒤 2자는 유효하지만 중간이 유효하지 않은 Base62
        let (id, rand) = split_short_key("Ab---Xy");
        assert_eq!(id, 0);
        assert!(rand.is_empty());
    }

    #[test]
    fn test_split_short_key_just_over_minimum() {
        // 정확히 5자로 유효한 경우
        let (id, rand) = split_short_key("Ab1Xy");
        assert_eq!(id, 1);
        assert_eq!(rand, "AbXy");
    }

    #[test]
    fn test_split_short_key_uppercase_encoding() {
        // 대문자만 포함된 인코딩
        let short_key = merge_short_key("ZZAA", 1000);
        let (id, rand) = split_short_key(&short_key);
        assert_eq!(id, 1000);
        assert_eq!(rand, "ZZAA");
    }

    #[test]
    fn test_split_short_key_lowercase_encoding() {
        // 소문자만 포함된 인코딩
        let short_key = merge_short_key("aabb", 2000);
        let (id, rand) = split_short_key(&short_key);
        assert_eq!(id, 2000);
        assert_eq!(rand, "aabb");
    }

    // ============ 경계값 테스트 ============

    #[test]
    fn test_boundary_id_u64_max() {
        let short_key = merge_short_key("BbCc", u64::MAX);
        let (id, rand) = split_short_key(&short_key);
        assert_eq!(id, u64::MAX);
        assert_eq!(rand, "BbCc");
    }

    #[test]
    fn test_boundary_id_u64_min() {
        let short_key = merge_short_key("CcDd", 0);
        let (id, rand) = split_short_key(&short_key);
        assert_eq!(id, 0);
        assert_eq!(rand, "CcDd");
    }

    // ============ 성능 관련 테스트 ============

    #[test]
    fn test_many_roundtrips() {
        for i in 0..1000 {
            let short_key = merge_short_key("RrSs", i);
            let (id, rand) = split_short_key(&short_key);
            assert_eq!(id, i);
            assert_eq!(rand, "RrSs");
        }
    }

    // ============ Base62 문자 집합 테스트 ============

    #[test]
    fn test_all_base62_rand_key_combinations() {
        let base62_chars = "0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";

        // 각 문자로 시작하는 랜덤 키 테스트
        for c in base62_chars.chars().take(10) {
            let rand_key = format!("{c}xY{c}");
            let short_key = merge_short_key(&rand_key, 100);
            let (id, rand) = split_short_key(&short_key);
            assert_eq!(id, 100);
            assert_eq!(rand, rand_key);
        }
    }

    // ============ 새로운 형식 검증 테스트 ============

    #[test]
    fn test_short_key_format_prefix_id_suffix() {
        let rand_key = "PrSf"; // Prefix="Pr", Suffix="Sf"
        let id = 12345;
        let short_key = merge_short_key(rand_key, id);

        // 형식 확인: Pr + base62(12345) + Sf
        let encoded_id = base62::encode(id);
        let expected = format!("Pr{encoded_id}Sf");
        assert_eq!(short_key, expected);
    }

    #[test]
    fn test_reconstructed_rand_key_is_correct() {
        let original_rand_key = "AbCd";
        let id = 777;
        let short_key = merge_short_key(original_rand_key, id);
        let (decoded_id, reconstructed_rand_key) = split_short_key(&short_key);

        assert_eq!(decoded_id, id);
        assert_eq!(reconstructed_rand_key, original_rand_key);
        assert_eq!(reconstructed_rand_key.len(), RANDOM_KEY_LEN);
    }
}
