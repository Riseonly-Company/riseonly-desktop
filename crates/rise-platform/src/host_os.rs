/// Which operating system a decision is being made *for*.
///
/// Only macOS is built and run today; the Linux and Windows arms of every seam
/// are written blind. This type is what keeps "blind" from also meaning
/// "untested": per-OS behaviour is expressed as a pure function of `HostOs`, so
/// all three arms are exercised by the suite on a Mac, and the only thing left
/// unverified elsewhere is the final call into the OS API itself.
///
/// A seam that reaches for `#[cfg(target_os)]` to make a *decision* has put that
/// decision beyond the reach of the tests. The cfg belongs at the binding, after
/// the decision is already made.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum HostOs {
    MacOs,
    Windows,
    Linux,
}

impl HostOs {
    /// The one place the running platform is read. Everything downstream takes
    /// it as an argument.
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
