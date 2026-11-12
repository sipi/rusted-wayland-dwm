//! **rwdwm** — un clone de dwm pour Wayland, écrit en Rust avec Smithay.
//!
//! La sémantique de dwm (tags, dispositions, focus, barre) vit dans le crate
//! pur [`rwdwm_core`]. Ce crate-ci est le compositeur : il traduit les
//! protocoles Wayland et les entrées en appels au noyau, puis applique le
//! modèle obtenu ([`state::State::sync`]) et compose l'image
//! ([`render::output_elements`]).
//!
//! Voir `docs/04-architecture-specifications-techniques.md`.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod backend;
pub mod config;
pub mod cursor;
mod handlers;
pub mod input;
pub mod render;
pub mod spawn;
pub mod state;
pub mod status;

pub use rwdwm_core as core;
