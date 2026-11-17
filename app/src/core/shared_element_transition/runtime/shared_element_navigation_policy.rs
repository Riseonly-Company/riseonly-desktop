use std::sync::LazyLock;

use crate::modules::post::engine::rise_post_engine_models::RisePostProfileKind;

pub enum SharedElementNavigationPolicy {}

static FLIGHTS_ENABLED: LazyLock<bool> = LazyLock::new(|| {
    let launch_arguments: Vec<String> = rise_platform::process_info::arguments();
    SharedElementNavigationPolicy::resolve_flights_enabled(
        SharedElementNavigationPolicy::FLIGHTS_SHIPPED,
        &launch_arguments,
    )
});

static INTERACTIVE_ENABLED: LazyLock<bool> = LazyLock::new(|| {
    let launch_arguments: Vec<String> = rise_platform::process_info::arguments();
    SharedElementNavigationPolicy::resolve_interactive_enabled(&launch_arguments)
});

impl SharedElementNavigationPolicy {
    pub const OFF_LAUNCH_ARGUMENT: &'static str = "-riseSharedElementOff";
    pub const NO_INTERACTIVE_LAUNCH_ARGUMENT: &'static str = "-riseSharedElementNoInteractive";

    pub const FLIGHTS_SHIPPED: bool = true;

    pub const ENABLED_KINDS: [RisePostProfileKind; 4] = [
        RisePostProfileKind::Publications,
        RisePostProfileKind::Threads,
        RisePostProfileKind::Reposts,
        RisePostProfileKind::Mentions,
    ];

    pub const VIEWER_KINDS: [RisePostProfileKind; 1] = [RisePostProfileKind::Riffs];

    pub fn flights_enabled() -> bool {
        *FLIGHTS_ENABLED
    }

    pub fn interactive_enabled() -> bool {
        *INTERACTIVE_ENABLED
    }

    /// stories grid and highlights fly into the story player overlay
    pub fn stories_enabled() -> bool {
        Self::flights_enabled()
    }

    pub fn stories_interactive_enabled() -> bool {
        Self::interactive_enabled()
    }

    pub fn resolve_flights_enabled(shipped: bool, launch_arguments: &[String]) -> bool {
        shipped && !launch_arguments.iter().any(|argument| argument == Self::OFF_LAUNCH_ARGUMENT)
    }

    pub fn resolve_interactive_enabled(launch_arguments: &[String]) -> bool {
        !launch_arguments
            .iter()
            .any(|argument| argument == Self::OFF_LAUNCH_ARGUMENT || argument == Self::NO_INTERACTIVE_LAUNCH_ARGUMENT)
    }
}
