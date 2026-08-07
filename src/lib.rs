//! rogue — a terminal roguelike, faithful to 1980 Rogue.
//!
//! Modules stay small and single-purpose (see `AGENTS.md`); the binary in
//! `main.rs` is a thin shell over them.

pub mod game;
pub mod map;
pub mod rng;
