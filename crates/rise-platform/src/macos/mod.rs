//! macOS-only backends. Nothing here decides: every seam's policy is a pure function of
//! [`HostOs`](crate::host_os::HostOs) one level up, so all platforms are exercised from a Mac.

pub mod glass;
pub mod window_corner;
