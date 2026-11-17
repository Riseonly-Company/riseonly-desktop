use gpui::{App, Task};
use rise_deeplink::{environment, environment_for_url_scheme};
use thiserror::Error;

use crate::gpui_shim::PlatformSupport;
use crate::host_os::HostOs;

/// why the os wasnt asked about a scheme
#[derive(Clone, PartialEq, Eq, Debug, Error)]
pub enum UrlSchemeUnavailable {
    #[error("URL schemes are not registered on {host:?}: the branch is not written")]
    BranchNotWritten { host: HostOs },
    #[error("this process is not an installed application")]
    NotInstalled,
    #[error("the sandboxed package declares its URL scheme itself")]
    DeclaredByPackage,
}

/// the system end of one os
pub trait UrlSchemeRegistrar {
    fn is_default_handler(&self, scheme: &str) -> Result<bool, UrlSchemeUnavailable> {
        let _ = scheme;
        Err(UrlSchemeUnavailable::BranchNotWritten {
            host: HostOs::current(),
        })
    }

    /// claims the scheme for this app, slow work goes inside the returned task
    fn register(
        &self,
        scheme: &str,
        cx: &App,
    ) -> Result<Task<anyhow::Result<()>>, UrlSchemeUnavailable> {
        let _ = (scheme, cx);
        Err(UrlSchemeUnavailable::BranchNotWritten {
            host: HostOs::current(),
        })
    }
}

/// what launch does about a scheme
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum UrlSchemeRegistrationStep {
    AlreadyDefault,
    Register,
    ForeignScheme,
    Unavailable(UrlSchemeUnavailable),
}

/// app scheme of a canon environment by its branch name
pub fn url_scheme_for_environment(name: &str) -> Option<&'static str> {
    environment(name).map(|environment| environment.url_scheme)
}

/// decides without changing anything in the os
pub fn registration_step(
    source: &dyn UrlSchemeRegistrar,
    scheme: &str,
) -> UrlSchemeRegistrationStep {
    if environment_for_url_scheme(scheme).is_none() {
        return UrlSchemeRegistrationStep::ForeignScheme;
    }
    match source.is_default_handler(scheme) {
        Ok(true) => UrlSchemeRegistrationStep::AlreadyDefault,
        Ok(false) => UrlSchemeRegistrationStep::Register,
        Err(reason) => UrlSchemeRegistrationStep::Unavailable(reason),
    }
}

pub fn register_url_scheme_with(
    source: &dyn UrlSchemeRegistrar,
    scheme: &str,
    cx: &App,
) -> PlatformSupport {
    match registration_step(source, scheme) {
        UrlSchemeRegistrationStep::AlreadyDefault => PlatformSupport::Performed,
        UrlSchemeRegistrationStep::ForeignScheme => {
            tracing::warn!(target: "riseonly", scheme, "refused to claim a URL scheme outside canon");
            PlatformSupport::Unsupported
        }
        UrlSchemeRegistrationStep::Unavailable(reason) => {
            tracing::debug!(target: "riseonly", %reason, scheme, "URL scheme not registered");
            PlatformSupport::Unsupported
        }
        UrlSchemeRegistrationStep::Register => match source.register(scheme, cx) {
            Ok(registration) => {
                let scheme = scheme.to_owned();
                cx.spawn(async move |_| {
                    if let Err(reason) = registration.await {
                        tracing::warn!(target: "riseonly", %reason, scheme, "the OS refused a URL scheme");
                    }
                })
                .detach();
                PlatformSupport::Performed
            }
            Err(reason) => {
                tracing::debug!(target: "riseonly", %reason, scheme, "URL scheme not registered");
                PlatformSupport::Unsupported
            }
        },
    }
}

/// registers scheme for this app on this os
pub fn register_url_scheme(scheme: &str, cx: &App) -> PlatformSupport {
    register_url_scheme_with(&super::os::NativeUrlScheme, scheme, cx)
}

pub fn is_default_handler(scheme: &str) -> bool {
    super::os::NativeUrlScheme
        .is_default_handler(scheme)
        .unwrap_or(false)
}
