use crate::core::lib::canon_format::RiseFormat;

/// facade over the size canon, base thresholds rounding and keys all live in file_size
/// name kept because ios call sites use it
pub enum ByteFormat {}

impl ByteFormat {
    pub fn telegram(bytes: i64) -> String {
        RiseFormat::file_size(bytes)
    }

    /// downloaded part is printed in the unit of the total
    /// otherwise 512 KB / 10.0 MB stops reading as a fraction
    pub fn telegram_transferred(bytes: i64, total_bytes: i64) -> String {
        let total_bytes = total_bytes.max(0);
        let bytes = bytes.max(0).min(total_bytes);
        RiseFormat::file_size_unit_of(bytes, total_bytes)
    }
}
