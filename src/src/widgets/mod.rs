//! Obsolete Relm4 widget components from the pre-integration desktop skeleton.
//!
//! The desktop UI is currently built procedurally in `app.rs` (see the plan in
//! `docs/postmortem.md`: the old components referenced a stale `AppMsg`
//! contract and an invalid debounce implementation). These modules are kept
//! for reference only and are excluded from compilation until a decision is
//! made to restore a Relm4-based component structure.

#[cfg(any())]
pub mod results_list;
#[cfg(any())]
pub mod search_bar;
#[cfg(any())]
pub mod sidebar;
