// Lives in its own crate so dev builds can optimize it; see `zcord-convert` in Cargo.toml.
#[cfg(windows)]
pub(crate) use zcord_convert::bgra_to_nv12;
pub(crate) use zcord_convert::{fit_dimensions, fit_even, nv12_to_bgra, scale_bgra};
