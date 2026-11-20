/// which os a decision is made for
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum HostOs {
    MacOs,
    Windows,
    Linux,
}

impl HostOs {
    /// the only place that reads the running platform, everything else gets it as an argument
    pub const fn current() -> Self {
        super::os::CURRENT
    }

    pub const ALL: [Self; 3] = [Self::MacOs, Self::Windows, Self::Linux];
}

#[cfg(test)]
mod tests {
    use super::*;

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

/// keeps a helper process from flashing its own console window
pub fn hide_console(command: &mut std::process::Command) {
    super::os::hide_console(command);
}
