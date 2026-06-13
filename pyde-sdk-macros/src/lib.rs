//! Proc-macros for [`pyde-rust-sdk`].
//!
//! ## T7 stub
//!
//! Real implementation lands in T10 (Phase A3). This crate will expose
//! the [`pyde_abi!`] declarative macro:
//!
//! ```ignore
//! pyde_abi! {
//!     contract Counter {
//!         fn increment();
//!         fn get_count() -> u64;
//!         event CountChanged(u64);
//!     }
//! }
//! ```
//!
//! Expands to a typed `Counter::at(addr, &provider)` factory plus one
//! method per declared function (returning a `CallBuilder`) and one
//! type per declared event (with a `.filter()` constructor for use
//! with [`crate::provider::Provider::subscribe_logs`]).
//!
//! Selectors are derived as `Blake3(function_name)[..4]` per
//! [`HOST_FN_ABI_SPEC §3.7`]. (Confirm with otigen session before
//! shipping — see T3 action A1.)
//!
//! ## Why a separate crate?
//!
//! Rust requires `proc-macro = true` crates to be standalone — they
//! can't coexist with regular library code in the same crate. Same
//! pattern as `serde` / `serde_derive` and `alloy` / `alloy-sol-macro`.
