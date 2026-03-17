// Suppress stylistic clippy lints that don't affect correctness
#![allow(clippy::match_single_binding)]
#![allow(clippy::map_unwrap_or)]
#![allow(clippy::len_zero)]
#![allow(clippy::should_implement_trait)]
#![allow(clippy::new_without_default)]
#![allow(clippy::let_and_return)]
#![allow(clippy::manual_range_contains)]
#![allow(clippy::if_same_then_else)]
#![allow(private_interfaces)]
#![allow(clippy::unnecessary_lazy_evaluations)]
#![allow(clippy::manual_clamp)]
#![allow(clippy::or_fun_call)]
#![allow(clippy::too_many_arguments)]
#![allow(clippy::collapsible_if)]

/// tradingbots.fun — live trading bot crate.
///
/// The live binary lives entirely in `main.rs` (and its `mod` declarations).
/// This lib crate is reserved for future library consumers or integration tests.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
