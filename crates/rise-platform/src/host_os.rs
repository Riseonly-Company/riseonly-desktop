/// Which operating system a decision is being made *for*.
///
/// Express per-OS behaviour as a pure function of this type, never as a
/// `#[cfg(target_os)]` decision — that puts the decision beyond the reach of the
/// tests. The cfg belongs at the binding, after the decision is already made.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum HostOs {
    MacOs,
    Windows,
    Linux,
}

impl HostOs {
    /// The one place the running platform is read; everything downstream takes it as an argument.
    pub const fn current() -> Self {
        #[cfg(target_os = "macos")]
        {
            Self::MacOs
        }
        #[cfg(target_os = "windows")]
        {
            Self::Windows
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            Self::Linux
        }
    }

    pub const ALL: [Self; 3] = [Self::MacOs, Self::Windows, Self::Linux];
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_current_host_matches_what_this_build_targets() {
        let expected = if cfg!(target_os = "macos") {
            HostOs::MacOs
        } else if cfg!(target_os = "windows") {
            HostOs::Windows
        } else {
            HostOs::Linux
        };

        assert_eq!(HostOs::current(), expected);
    }

    #[test]
    fn every_variant_appears_in_all_exactly_once() {
        for host in HostOs::ALL {
            assert_eq!(
                HostOs::ALL.iter().filter(|other| **other == host).count(),
                1,
                "a seam iterating ALL must visit {host:?} exactly once"
            );
        }
    }
}
