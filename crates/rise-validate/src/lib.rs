//! The product's validation rules, in one place, as data.
//!
//! A schema is a `const` declared beside the field it belongs to:
//!
//! ```
//! use rise_validate::{Charset, Check, TextSchema};
//!
//! pub const TAG: TextSchema = TextSchema::new("tag")
//!     .trim()
//!     .lowercase()
//!     .missing_message("auth_tag_required")
//!     .checks(&[
//!         Check::min_chars(3).message("auth_tag_too_short"),
//!         Check::max_bytes(32).message("auth_tag_too_long"),
//!         Check::charset(Charset::TAG).message("auth_tag_alphabet"),
//!         Check::not_edged_with('_').message("auth_tag_underscore_edge"),
//!         Check::no_run_of("__").message("auth_tag_double_underscore"),
//!     ]);
//!
//! // One call judges the value AND hands back what goes on the wire.
//! assert_eq!(TAG.check("  RiseOnly  ").unwrap(), "riseonly");
//! assert!(TAG.check("тег").is_err());
//! ```
//!
//! - **`check` returns the NORMALISED value**, so there is no second `normalize`
//!   call a caller can forget and no way for the checked and sent values to differ.
//! - **Violations are data, never sentences.** A [`Violation`] carries the rule, the
//!   numbers, the offending character and an i18n KEY; nothing here renders English.
//! - **Schemas are `const`.** No allocation, no `LazyLock`.
//!
//! Every validation in the product goes through a schema here: a bare
//! `if value.len() < 3` in a store or a screen is a second copy of a server bound.

pub mod charset;
pub mod report;
pub mod text;
pub mod violation;

pub use charset::Charset;
pub use report::Report;
pub use text::{Check, TextSchema, confirm};
pub use violation::{Edge, Unit, Violation, ViolationKind};
