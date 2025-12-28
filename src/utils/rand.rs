//! 랜덤 문자열 생성 모듈.

use rand::distributions::{Alphanumeric, DistString};

/// Generates a random alphanumeric string of the specified length.
/// Uses the optimized `Alphanumeric` distribution for better performance.
#[must_use]
pub fn gen_rand_str(len: usize) -> String {
    Alphanumeric.sample_string(&mut rand::thread_rng(), len)
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn test_gen_rand_str_length_zero() {
        let s = gen_rand_str(0);
        assert!(s.is_empty());
    }

    #[test]
    fn test_gen_rand_str_length_one() {
        let s = gen_rand_str(1);
        assert_eq!(s.len(), 1);
        assert!(s.chars().next().unwrap().is_ascii_alphanumeric());
    }

    #[test]
    fn test_gen_rand_str_length_two() {
        let s = gen_rand_str(2);
        assert_eq!(s.len(), 2);
    }

    #[test]
    fn test_gen_rand_str_length_ten() {
        let s = gen_rand_str(10);
        assert_eq!(s.len(), 10);
    }

    #[test]
    fn test_gen_rand_str_length_hundred() {
        let s = gen_rand_str(100);
        assert_eq!(s.len(), 100);
    }

    #[test]
    fn test_gen_rand_str_alphanumeric_small() {
        let s = gen_rand_str(50);
        assert!(s.chars().all(|c| c.is_ascii_alphanumeric()));
    }

    #[test]
    fn test_gen_rand_str_alphanumeric_large() {
        let s = gen_rand_str(1000);
        assert!(s.chars().all(|c| c.is_ascii_alphanumeric()));
    }

    #[test]
    fn test_gen_rand_str_unique() {
        let s1 = gen_rand_str(10);
        let s2 = gen_rand_str(10);
        // 랜덤이므로 다를 확률이 매우 높음 (62^10 중 하나)
        assert_ne!(s1, s2);
    }

    #[test]
    fn test_gen_rand_str_uniqueness_multiple() {
        let mut set = HashSet::new();
        for _ in 0..100 {
            let s = gen_rand_str(8);
            set.insert(s);
        }
        // 100개 생성 시 모두 고유해야 함 (충돌 확률 매우 낮음)
        assert_eq!(set.len(), 100);
    }

    #[test]
    fn test_gen_rand_str_contains_digits() {
        // 많이 생성하면 숫자가 포함될 확률이 높음
        let mut found_digit = false;
        for _ in 0..100 {
            let s = gen_rand_str(20);
            if s.chars().any(|c| c.is_ascii_digit()) {
                found_digit = true;
                break;
            }
        }
        assert!(found_digit, "숫자가 한 번도 생성되지 않음");
    }

    #[test]
    fn test_gen_rand_str_contains_uppercase() {
        let mut found_upper = false;
        for _ in 0..100 {
            let s = gen_rand_str(20);
            if s.chars().any(|c| c.is_ascii_uppercase()) {
                found_upper = true;
                break;
            }
        }
        assert!(found_upper, "대문자가 한 번도 생성되지 않음");
    }

    #[test]
    fn test_gen_rand_str_contains_lowercase() {
        let mut found_lower = false;
        for _ in 0..100 {
            let s = gen_rand_str(20);
            if s.chars().any(|c| c.is_ascii_lowercase()) {
                found_lower = true;
                break;
            }
        }
        assert!(found_lower, "소문자가 한 번도 생성되지 않음");
    }

    #[test]
    fn test_gen_rand_str_no_special_chars() {
        let s = gen_rand_str(1000);
        assert!(!s.chars().any(|c| !c.is_ascii_alphanumeric()));
    }
}
