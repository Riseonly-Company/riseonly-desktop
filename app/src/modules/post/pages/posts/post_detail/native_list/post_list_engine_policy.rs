use std::sync::LazyLock;

pub enum PostListEnginePolicy {}

static USES_NATIVE_LIST: LazyLock<bool> = LazyLock::new(|| {
    let launch_arguments: Vec<String> = rise_platform::process_info::arguments();
    PostListEnginePolicy::resolve_uses_native_list(&launch_arguments)
});

static TRACE_ENABLED: LazyLock<bool> = LazyLock::new(|| {
    let launch_arguments: Vec<String> = rise_platform::process_info::arguments();
    PostListEnginePolicy::resolve_trace_enabled(&launch_arguments)
});

static PARITY_AUDIT_ENABLED: LazyLock<bool> = LazyLock::new(|| {
    let launch_arguments: Vec<String> = rise_platform::process_info::arguments();
    PostListEnginePolicy::resolve_parity_audit_enabled(&launch_arguments, PostListEnginePolicy::is_debug_build())
});

impl PostListEnginePolicy {
    pub const NATIVE_OFF_LAUNCH_ARGUMENT: &'static str = "-risePostListNativeOff";
    pub const TRACE_LAUNCH_ARGUMENT: &'static str = "-risePostListTrace";
    pub const PARITY_AUDIT_LAUNCH_ARGUMENT: &'static str = "-risePostListParityAudit";

    pub fn uses_native_list() -> bool {
        *USES_NATIVE_LIST
    }

    pub fn trace_enabled() -> bool {
        *TRACE_ENABLED
    }

    pub fn parity_audit_enabled() -> bool {
        *PARITY_AUDIT_ENABLED
    }

    pub fn resolve_uses_native_list(launch_arguments: &[String]) -> bool {
        !launch_arguments.iter().any(|argument| argument == Self::NATIVE_OFF_LAUNCH_ARGUMENT)
    }

    pub fn resolve_trace_enabled(launch_arguments: &[String]) -> bool {
        launch_arguments.iter().any(|argument| argument == Self::TRACE_LAUNCH_ARGUMENT)
    }

    pub fn resolve_parity_audit_enabled(launch_arguments: &[String], is_debug_build: bool) -> bool {
        is_debug_build && launch_arguments.iter().any(|argument| argument == Self::PARITY_AUDIT_LAUNCH_ARGUMENT)
    }

    fn is_debug_build() -> bool {
        cfg!(debug_assertions)
    }
}
