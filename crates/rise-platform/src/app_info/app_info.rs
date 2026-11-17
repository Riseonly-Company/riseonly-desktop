/// bundle identity one environment ships under
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct AppIdentity {
    pub environment: &'static str,
    pub display_name: &'static str,
    pub bundle_identifier: &'static str,
}

pub const IDENTITIES: [AppIdentity; 3] = [
    AppIdentity {
        environment: "dev",
        display_name: "Riseonly Dev",
        bundle_identifier: "net.riseonly.desktop.dev",
    },
    AppIdentity {
        environment: "staging",
        display_name: "Riseonly Staging",
        bundle_identifier: "net.riseonly.desktop.staging",
    },
    AppIdentity {
        environment: "production",
        display_name: "Riseonly",
        bundle_identifier: "net.riseonly.desktop",
    },
];

/// identity of a canon environment name, exact and case sensitive
pub fn identity_for(environment: &str) -> Option<&'static AppIdentity> {
    IDENTITIES
        .iter()
        .find(|identity| identity.environment == environment)
}

pub const MARKETING_VERSION: &str = env!("CARGO_PKG_VERSION");

pub const BUILD_NUMBER_FALLBACK: &str = "1";

/// bundle version of this build
pub fn compiled_build_number() -> &'static str {
    build_number_from(option_env!("RISE_BUILD_NUMBER"))
}

/// build number is one or more integers separated by periods
/// anything else (empty too) falls back, so a stray value never reaches a header
pub fn build_number_from(raw: Option<&str>) -> &str {
    match raw {
        Some(value) if is_bundle_version(value) => value,
        _ => BUILD_NUMBER_FALLBACK,
    }
}

fn is_bundle_version(value: &str) -> bool {
    !value.is_empty()
        && value
            .split('.')
            .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
}

/// where the answer came from
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum AppInfoSource {
    InfoPlist,
    Compiled,
}

/// what an os layer read from its own bundle dictionary, none for a key it doesnt carry
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct BundleValues {
    pub short_version: Option<String>,
    pub build: Option<String>,
    pub bundle_identifier: Option<String>,
    pub display_name: Option<String>,
}

impl BundleValues {
    pub fn is_app_bundle(&self) -> bool {
        has_text(self.short_version.as_deref()) && has_text(self.bundle_identifier.as_deref())
    }
}

fn has_text(value: Option<&str>) -> bool {
    value.is_some_and(|text| !text.is_empty())
}

/// identity of the main bundle as callers read it
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct AppInfo {
    pub short_version: String,
    pub build: String,
    pub bundle_identifier: String,
    pub display_name: String,
    pub source: AppInfoSource,
}

impl AppInfo {
    pub fn compiled(identity: &AppIdentity) -> Self {
        Self {
            short_version: MARKETING_VERSION.to_owned(),
            build: compiled_build_number().to_owned(),
            bundle_identifier: identity.bundle_identifier.to_owned(),
            display_name: identity.display_name.to_owned(),
            source: AppInfoSource::Compiled,
        }
    }

    /// own values of the bundle when its the app bundle, each missing key filled from the table
    pub fn resolve(identity: &AppIdentity, bundle: BundleValues) -> Self {
        let compiled = Self::compiled(identity);
        if !bundle.is_app_bundle() {
            return compiled;
        }
        let pick = |value: Option<String>, fallback: String| {
            value.filter(|text| !text.is_empty()).unwrap_or(fallback)
        };
        Self {
            short_version: pick(bundle.short_version, compiled.short_version),
            build: pick(bundle.build, compiled.build),
            bundle_identifier: pick(bundle.bundle_identifier, compiled.bundle_identifier),
            display_name: pick(bundle.display_name, compiled.display_name),
            source: AppInfoSource::InfoPlist,
        }
    }

    /// the running app for a canon environment name, none for a name the table doesnt know
    pub fn current(environment: &str) -> Option<Self> {
        identity_for(environment).map(|identity| Self::resolve(identity, super::os::bundle_values()))
    }
}
