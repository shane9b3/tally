//! Transport-agnostic API for Tally.
//!
//! - [`dto`]: data transfer objects, exported to TypeScript with ts-rs into
//!   `packages/bridge/src/generated/` (`cargo xtask gen-bindings`).
//! - [`service`]: the [`DocumentService`] trait, implemented by the engine
//!   (WP1) and wrapped 1:1 by the Tauri commands (WP7). The mapping from
//!   Tauri command names to trait methods is in `docs/api.md`.
//!
//! This crate is a frozen WP0 contract. Additive changes are fine through a
//! lead-reviewed PR; breaking changes need sign-off (see
//! `docs/contributing.md`).

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod dto;
pub mod service;

pub use dto::*;
pub use service::{DocumentService, Result};
