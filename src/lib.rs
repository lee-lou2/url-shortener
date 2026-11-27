//! Fast, stateless URL shortener backed by PostgreSQL and an in-process `TinyLFU` cache.

pub mod cache;
pub mod config;
pub mod error;
pub mod guard;
pub mod handlers;
pub mod links;
pub mod redirect;
pub mod short_key;
pub mod webhook;
