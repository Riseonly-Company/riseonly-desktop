//! The macOS-only implementations behind seams whose policy lives one directory
//! up.
//!
//! Nothing in here makes a decision. Every decision a seam takes is a pure
//! function of [`HostOs`](crate::host_os::HostOs) beside the trait it serves, so
//! that all three platforms are exercised from a Mac; this tree is the final
//! call into the OS after that decision has already been made.

pub mod glass;
