#![cfg(all(feature = "migrations", feature = "sqlite-unbundled"))]

#[path = "migrations/mod.rs"]
mod cases;
