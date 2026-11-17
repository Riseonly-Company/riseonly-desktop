use super::text_capabilities::TextCoverageUnavailable;
use crate::host_os::HostOs;

pub(super) fn glyph_coverage(_text: &str, host: HostOs) -> Result<bool, TextCoverageUnavailable> {
    Err(TextCoverageUnavailable::BranchNotWritten { host })
}
