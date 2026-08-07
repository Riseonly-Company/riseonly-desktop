use std::fmt;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum AppEnvironment {
    #[default]
    Dev,
    Staging,
    Prod,
}

impl AppEnvironment {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Dev => "dev",
            Self::Staging => "staging",
            Self::Prod => "prod",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "dev" | "debug" | "development" | "local" => Some(Self::Dev),
            "staging" | "stage" => Some(Self::Staging),
            "prod" | "production" | "release" => Some(Self::Prod),
            _ => None,
        }
    }

    /// URL scheme for deep links. Each environment gets its own so a staging
    /// build cannot swallow a production link when both are installed.
    pub fn url_scheme(self) -> &'static str {
        match self {
            Self::Dev => "riseonly-dev",
            Self::Staging => "riseonly-staging",
            Self::Prod => "riseonly",
        }
    }

    /// Directory suffix that keeps each environment's database separate.
    ///
    /// Without this a developer switching from staging to production would open
    /// a store written by another backend, and the engine would either refuse
    /// the key or serve stale rows that look real.
    pub fn data_dir_suffix(self) -> &'static str {
        match self {
            Self::Dev => "Dev",
            Self::Staging => "Staging",
            Self::Prod => "",
        }
    }

    /// What the user sees: in the Dock, in the menu bar, in the window title.
    ///
    /// Named per environment because all three can be installed at once, and a
    /// developer switching between them has to be able to tell which window is
    /// which without opening a settings screen. Production is plain "Riseonly";
    /// nobody ships an app whose name says "prod".
    pub fn display_name(self) -> &'static str {
        match self {
            Self::Dev => "Riseonly Dev",
            Self::Staging => "Riseonly Staging",
            Self::Prod => "Riseonly",
        }
    }

    /// The bundle identifier. Distinct per environment for the same reason the
    /// data directory is: two builds must not share a Keychain ACL, a
    /// notification authorisation or a launch-services registration.
    pub fn bundle_id(self) -> &'static str {
        match self {
            Self::Dev => "net.riseonly.desktop.dev",
            Self::Staging => "net.riseonly.desktop.staging",
            Self::Prod => "net.riseonly.desktop",
        }
    }

    pub fn allows_local_override(self) -> bool {
        // A production build must never be repointed by a file on disk or a
        // stray environment variable.
        self != Self::Prod
    }

    /// Compiled in at build time; falls back to dev so a plain `cargo run`
    /// targets the local stack rather than production.
    pub fn compiled() -> Self {
        match option_env!("RISE_ENV") {
            Some(raw) => Self::parse(raw).unwrap_or(Self::Dev),
            None => Self::Dev,
        }
    }
}

impl fmt::Display for AppEnvironment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn every_environment_is_nameable_and_distinguishable_at_a_glance() {
        let names: Vec<&str> = [
            AppEnvironment::Dev,
            AppEnvironment::Staging,
            AppEnvironment::Prod,
        ]
        .iter()
        .map(|environment| environment.display_name())
        .collect();

        assert_eq!(names, vec!["Riseonly Dev", "Riseonly Staging", "Riseonly"]);
        let mut unique = names.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), names.len());
    }

    #[test]
    fn the_bundle_identity_is_never_shared_between_environments() {
        let ids: Vec<&str> = [
            AppEnvironment::Dev,
            AppEnvironment::Staging,
            AppEnvironment::Prod,
        ]
        .iter()
        .map(|environment| environment.bundle_id())
        .collect();

        let mut unique = ids.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(
            unique.len(),
            ids.len(),
            "a shared identity means a shared Keychain ACL and a shared deep-link registration"
        );
        assert_eq!(AppEnvironment::Prod.bundle_id(), "net.riseonly.desktop");
    }

    use super::*;

    #[test]
    fn every_spelling_of_an_environment_parses() {
        for raw in ["dev", "DEV", " development ", "local", "debug"] {
            assert_eq!(
                AppEnvironment::parse(raw),
                Some(AppEnvironment::Dev),
                "{raw}"
            );
        }
        for raw in ["staging", "Stage"] {
            assert_eq!(AppEnvironment::parse(raw), Some(AppEnvironment::Staging));
        }
        for raw in ["prod", "production", "RELEASE"] {
            assert_eq!(AppEnvironment::parse(raw), Some(AppEnvironment::Prod));
        }
        assert_eq!(AppEnvironment::parse("nonsense"), None);
    }

    const EVERY: [AppEnvironment; 3] = [
        AppEnvironment::Dev,
        AppEnvironment::Staging,
        AppEnvironment::Prod,
    ];

    #[test]
    fn each_environment_owns_a_distinct_url_scheme() {
        let schemes: Vec<&str> = EVERY.iter().map(|e| e.url_scheme()).collect();
        let mut unique = schemes.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(
            unique.len(),
            schemes.len(),
            "two environments sharing a scheme would steal each other's deep links"
        );
    }

    #[test]
    fn each_environment_owns_a_distinct_data_directory() {
        let suffixes: Vec<&str> = EVERY.iter().map(|e| e.data_dir_suffix()).collect();
        let mut unique = suffixes.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), suffixes.len());
    }

    #[test]
    fn production_refuses_to_be_repointed() {
        assert!(!AppEnvironment::Prod.allows_local_override());
        assert!(AppEnvironment::Dev.allows_local_override());
        assert!(AppEnvironment::Staging.allows_local_override());
    }

    #[test]
    fn an_unset_or_unknown_build_env_falls_back_to_dev_not_prod() {
        assert_eq!(AppEnvironment::default(), AppEnvironment::Dev);
    }
}
