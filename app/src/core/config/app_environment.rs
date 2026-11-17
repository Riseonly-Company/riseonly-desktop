use std::fmt;

use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_deeplink::{ENVIRONMENTS, Environment};

use crate::core::assets::packaged_file;

/// app environment, scheme and host and accepted APP_ENV spellings come from the canon row
/// so every build of one branch answers to the same links
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum AppEnvironment {
    #[default]
    Dev,
    Staging,
    Prod,
}

impl AppEnvironment {
    pub const ALL: [Self; 3] = [Self::Dev, Self::Staging, Self::Prod];

    /// constant indices so a shrinking canon table fails the build and not a lookup at runtime
    pub fn canon(self) -> &'static Environment {
        match self {
            Self::Dev => &ENVIRONMENTS[0],
            Self::Staging => &ENVIRONMENTS[1],
            Self::Prod => &ENVIRONMENTS[2],
        }
    }

    pub fn from_canon(environment: &Environment) -> Option<Self> {
        ENVIRONMENTS
            .iter()
            .position(|row| row == environment)
            .and_then(|index| Self::ALL.get(index).copied())
    }

    /// canon branch name, dev or staging or production
    pub fn as_str(self) -> &'static str {
        self.canon().name
    }

    /// exact and case sensitive like the server reads APP_ENV, one spelling list for all platforms
    pub fn parse(raw: &str) -> Option<Self> {
        ENVIRONMENTS
            .iter()
            .find(|row| row.app_env_values.contains(&raw))
            .and_then(Self::from_canon)
    }

    pub fn url_scheme(self) -> &'static str {
        self.canon().url_scheme
    }

    pub fn web_host(self) -> &'static str {
        self.canon().web_host
    }

    pub fn accepts_web_host(self, host: &str) -> bool {
        self.canon().accepts_web_host(host)
    }

    pub fn data_dir_suffix(self) -> &'static str {
        match self {
            Self::Dev => "Dev",
            Self::Staging => "Staging",
            Self::Prod => "",
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Self::Dev => "Riseonly Dev",
            Self::Staging => "Riseonly Staging",
            Self::Prod => "Riseonly",
        }
    }

    pub fn allows_local_override(self) -> bool {
        self != Self::Prod
    }

    /// build value is trimmed and lowercased then matched
    /// unset or unknown means dev only in debug builds, a release without RISE_ENV talks to production
    pub fn from_build_value(raw: Option<&str>, debug_build: bool) -> Self {
        let normalized = raw
            .map(|value| trimming_whitespaces_and_newlines(value).to_lowercase())
            .unwrap_or_default();
        match Self::parse(&normalized) {
            Some(environment) => environment,
            None if debug_build => Self::Dev,
            None => Self::Prod,
        }
    }

    pub fn compiled() -> Self {
        Self::from_build_value(option_env!("RISE_ENV"), cfg!(debug_assertions))
    }

    pub fn packaged_or_compiled(packaged: Option<&str>) -> Self {
        packaged
            .map(|value| trimming_whitespaces_and_newlines(value).to_lowercase())
            .and_then(|value| Self::parse(&value))
            .unwrap_or_else(Self::compiled)
    }

    pub fn packaged() -> Self {
        let packaged = packaged_file("environment").and_then(|path| std::fs::read_to_string(path).ok());
        Self::packaged_or_compiled(packaged.as_deref())
    }
}

impl fmt::Display for AppEnvironment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
#[path = "app_environment_tests.rs"]
mod tests;
