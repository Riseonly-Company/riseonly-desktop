use rise_core::foundation::foundation_url::Url;
use thiserror::Error;

use crate::host_os::HostOs;

/// why the os wasnt asked, the caller reads no handler and takes its fallback
#[derive(Clone, PartialEq, Eq, Debug, Error)]
pub enum UrlSchemeHandlerUnavailable {
    #[error("URL handlers are not queried on {host:?}: the branch is not written")]
    BranchNotWritten { host: HostOs },
}

/// system query of one os, runs on the caller thread (main for open_url) and answers synchronously
/// so a branch must not start a process or wait on a bus
pub trait UrlSchemeHandlerSource {
    fn has_handler(&self, url: &Url) -> Result<bool, UrlSchemeHandlerUnavailable> {
        let _ = url;
        Err(UrlSchemeHandlerUnavailable::BranchNotWritten {
            host: HostOs::current(),
        })
    }
}

/// answer of source, an os that cant be asked counts as no handler
pub fn can_open_url_with(source: &dyn UrlSchemeHandlerSource, url: &Url) -> bool {
    match source.has_handler(url) {
        Ok(found) => found,
        Err(reason) => {
            tracing::debug!(target: "riseonly", %reason, "no answer about a URL handler");
            false
        }
    }
}

pub fn can_open_url(url: &Url) -> bool {
    can_open_url_with(&super::os::NativeUrlSchemeHandler, url)
}
