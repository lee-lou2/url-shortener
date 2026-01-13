//! Utility module.
//!
//! Provides JWT, random string generation, and short key encoding utilities.

pub mod jwt;
pub mod rand;
pub mod short_key;

pub use jwt::{gen_token, parse_token, Claims};
pub use rand::gen_rand_str;
pub use short_key::{merge_short_key, split_short_key};
