use rise_core::foundation::foundation_date::{Date, RiseClock};
use rise_engine::RiseWireRemoteError;

use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::sai::Sai;
use crate::core::rise_engine_bridge::rise_engine_main_hop::RiseEngineMainHop;
use crate::modules::post::engine::rise_post_engine_models::RisePostCacheSource;
use crate::modules::search::engine::rise_search_rpc::RiseSearchEndpoint;

pub enum RiseSearchDebugTrace {}

impl RiseSearchDebugTrace {
    const ENABLED: bool = cfg!(any(debug_assertions, feature = "sai_debug_tools"));

    pub fn cache(scope_key: &str, source: RisePostCacheSource, revision: i64) {
        if !Self::ENABLED {
            return;
        }
        let message =
            format!("cache source={} scope={} revision={} rpc_attempts=0", source.wire(), scope_key, revision);
        Self::log(message, SaiLogKind::Debug);
    }

    pub fn network_started(endpoint: &RiseSearchEndpoint) {
        if !Self::ENABLED {
            return;
        }
        let message = format!("rpc start service={} method={} kind=read", endpoint.service, endpoint.method);
        Self::log(message, SaiLogKind::Network);
    }

    /// server errors come as code plus text, so print both or the log just says error 1
    fn describe(error: &anyhow::Error) -> String {
        if let Some(remote) = error.downcast_ref::<RiseWireRemoteError>() {
            return format!("{}: {}", remote.code, remote.message);
        }
        error.to_string()
    }

    pub fn network_finished(endpoint: &RiseSearchEndpoint, started_at: Date, error: Option<&anyhow::Error>) {
        if !Self::ENABLED {
            return;
        }
        let latency = (RiseClock::now().time_interval_since(&started_at) * 1_000.0).max(0.0) as i64;
        let result = if error.is_none() { "ok" } else { "error" };
        let described = error.map_or_else(|| "-".to_owned(), Self::describe);
        let message = format!(
            "rpc finish service={} method={} latency_ms={} result={} error={}",
            endpoint.service, endpoint.method, latency, result, described
        );
        Self::log(message, SaiLogKind::Network);
    }

    /// writes a log line for the search module
    fn log(message: String, kind: SaiLogKind) {
        let Some(main) = RiseEngineMainHop::installed() else {
            return;
        };
        main.post(move |cx| Sai::log("RiseSearch", || message, kind, cx));
    }
}
