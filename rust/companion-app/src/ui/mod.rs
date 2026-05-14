//! Tab-specific rendering for `CompanionApp`.
//!
//! Splitting the three tabs into their own files keeps `main.rs` focused on
//! state management (struct, lifecycle, status.json polling, async result
//! channels) while keeping each rendering block in a file small enough to
//! hold in your head when iterating on visuals.
//!
//! Each sub-module hangs new methods off `CompanionApp` via `impl` blocks —
//! Rust allows the impl to be split across files of the same crate, so the
//! UI methods stay accessible via `self.render_*(...)` from `App::update`.

pub mod deploy;
pub mod home;
pub mod settings;
