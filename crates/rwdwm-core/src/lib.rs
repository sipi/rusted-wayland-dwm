//! Noyau de **rwdwm** : la sémantique de dwm, sans Wayland ni E/S.
//!
//! Ce crate ne dépend que de la bibliothèque standard. Il contient :
//!
//! - [`model`] : moniteurs, clients, tags, pile de focus — l'équivalent des
//!   structures et fonctions de `dwm.c` ;
//! - [`layout`] : les dispositions (tuile, monocle) sous forme de fonctions pures ;
//! - [`bar`] et [`font`] : le modèle, le dessin et le test de clic de la barre ;
//! - [`config`] et [`action`] : les types de la configuration (équivalent de `config.h`).
//!
//! Le compositeur traduit les événements en appels à [`model::Wm`], puis relit
//! le modèle pour l'appliquer (voir `docs/04-architecture-specifications-techniques.md`).

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod action;
pub mod bar;
pub mod config;
pub mod font;
pub mod geometry;
pub mod keysyms;
pub mod layout;
pub mod model;

pub use action::{Action, Effect};
pub use config::Config;
pub use geometry::Rect;
pub use model::{Client, ClientId, Monitor, MonitorId, NewClient, TagMask, Wm};
