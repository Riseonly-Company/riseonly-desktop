use std::collections::HashMap;

use futures::FutureExt;
use gpui::AsyncApp;
use rise_core::foundation::foundation_date::Date;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::CancellationError;

use crate::core::lib::sai::modules::sai_debugger::sai_http_trace_entry::{
    SaiHttpTraceEntry, SaiTraceTag, SaiTraceTransport,
};
use crate::core::lib::sai::modules::sai_debugger::sai_http_trace_store::SaiHttpTraceStore;
use crate::core::rise_engine_bridge::rise_engine_main_hop::RiseEngineMainHop;
use crate::modules::user::engine::core::rise_user_achievements_resource_repository::RiseUserAchievementsCacheSource;
use crate::modules::user::engine::core::rise_user_rpc::{RiseUserEndpoint, RiseUserEndpointReplay};
use crate::modules::user::engine::rise_user_engine_models::{
    RiseUserSettingsError, RiseUserThemeSettingsCacheSource,
};

#[derive(Clone)]
pub struct RiseUserDebugTraceNetworkSpan {
    pub id: Uuid,
    pub started_at: Date,
    pub endpoint: RiseUserEndpoint,
    pub generation: u64,
}

pub enum RiseUserDebugTrace {}

impl RiseUserDebugTrace {
    const ENABLED: bool = cfg!(any(debug_assertions, feature = "sai_debug_tools"));

    pub async fn record_cache_read(
        source: RiseUserThemeSettingsCacheSource,
        resource_key: &str,
        revision: i64,
    ) {
        if !Self::ENABLED {
            return;
        }
        let served_from = match source {
            RiseUserThemeSettingsCacheSource::Memory => "memory",
            RiseUserThemeSettingsCacheSource::PersistentStore => "disk",
            RiseUserThemeSettingsCacheSource::Miss => "",
        };
        let is_miss = source == RiseUserThemeSettingsCacheSource::Miss;
        let entry = Self::cache_entry(
            "RiseStore user/theme-settings",
            "user.theme-settings.cache",
            source.raw_value(),
            served_from,
            "0",
            is_miss,
            resource_key,
            revision,
        );
        Self::upsert(move |_| entry).await;
    }

    pub async fn record_achievements_cache_read(
        source: RiseUserAchievementsCacheSource,
        resource_key: &str,
        revision: i64,
    ) {
        if !Self::ENABLED {
            return;
        }
        let served_from = match source {
            RiseUserAchievementsCacheSource::Memory => "memory",
            RiseUserAchievementsCacheSource::PersistentStore => "disk",
            RiseUserAchievementsCacheSource::Network => "network",
            RiseUserAchievementsCacheSource::Miss => "",
        };
        let rpc_attempts = if source == RiseUserAchievementsCacheSource::Network { "1" } else { "0" };
        let is_miss = source == RiseUserAchievementsCacheSource::Miss;
        let entry = Self::cache_entry(
            "RiseStore user/achievements",
            "user.achievements.cache",
            source.raw_value(),
            served_from,
            rpc_attempts,
            is_miss,
            resource_key,
            revision,
        );
        Self::upsert(move |_| entry).await;
    }

    pub async fn begin_network(endpoint: RiseUserEndpoint, generation: u64) -> RiseUserDebugTraceNetworkSpan {
        let span = RiseUserDebugTraceNetworkSpan {
            id: Uuid::new_v4(),
            started_at: Date::now(),
            endpoint,
            generation,
        };
        if Self::ENABLED {
            let pending = span.clone();
            Self::upsert(move |now| Self::entry(&pending, true, 0, false, false, None, now)).await;
        }
        span
    }

    pub async fn finish_network(
        span: &RiseUserDebugTraceNetworkSpan,
        cache_committed: bool,
        published: bool,
        error: Option<&anyhow::Error>,
    ) {
        if !Self::ENABLED {
            return;
        }
        let span = span.clone();
        let status = if error.is_none() { 200 } else { 0 };
        let label = error.map(Self::error_label);
        Self::upsert(move |now| Self::entry(&span, false, status, cache_committed, published, label, now)).await;
    }

    pub fn error_label(error: &anyhow::Error) -> String {
        if let Some(error) = error.downcast_ref::<RiseUserSettingsError>() {
            return match error {
                RiseUserSettingsError::MissingActiveSession => "theme_settings_session_missing".to_owned(),
                RiseUserSettingsError::StaleAccount => "theme_settings_stale_account".to_owned(),
                RiseUserSettingsError::InvalidResponse => "theme_settings_invalid_response".to_owned(),
                RiseUserSettingsError::Rejected(message) => {
                    let normalized = trimming_whitespaces_and_newlines(message);
                    if normalized.is_empty() {
                        "theme_settings_rejected".to_owned()
                    } else {
                        normalized.chars().take(256).collect()
                    }
                }
            };
        }
        if error.downcast_ref::<CancellationError>().is_some() {
            return "theme_settings_cancelled".to_owned();
        }
        "theme_settings_unavailable".to_owned()
    }

    #[allow(clippy::too_many_arguments)]
    fn cache_entry(
        url: &str,
        fetch_id: &str,
        source: &str,
        served_from: &str,
        rpc_attempts: &str,
        is_miss: bool,
        resource_key: &str,
        revision: i64,
    ) -> SaiHttpTraceEntry {
        SaiHttpTraceEntry {
            id: Uuid::new_v4(),
            transport: SaiTraceTransport::Websocket,
            created_at: Date::now(),
            is_pending: false,
            method: "CACHE".to_owned(),
            url: url.to_owned(),
            fetch_id: Some(fetch_id.to_owned()),
            duration_ms: 0,
            request_headers: Self::headers(&[
                ("x-rise-policy", "cache-first"),
                ("x-rise-presentation-cache", source),
                ("x-rise-request-generation", "0"),
            ]),
            request_body_wire: format!("{{\"resource\":\"{resource_key}\"}}"),
            response_status: if is_miss { 204 } else { 200 },
            response_headers: Self::headers(&[
                ("x-sai-served-from", served_from),
                ("x-rise-cache-lookup", source),
                ("x-rise-cache-revision", &revision.to_string()),
                ("x-rise-rpc-attempts", rpc_attempts),
                ("x-rise-source", source),
            ]),
            response_body_preview: String::new(),
            transport_error: None,
            trace_tags: vec![SaiTraceTag::RISE_ENGINE.to_owned(), SaiTraceTag::SYSTEM.to_owned()],
        }
    }

    /// builds the trace entry, clock is read on the main thread too
    fn entry(
        span: &RiseUserDebugTraceNetworkSpan,
        is_pending: bool,
        status: i64,
        cache_committed: bool,
        published: bool,
        transport_error: Option<String>,
        now: Date,
    ) -> SaiHttpTraceEntry {
        let replay = match span.endpoint.replay {
            RiseUserEndpointReplay::ReadOnly => "read-only",
            RiseUserEndpointReplay::Never => "never",
        };
        let duration_ms =
            if is_pending { 0 } else { ((now.time_interval_since(&span.started_at) * 1_000.0) as i64).max(0) };
        let cache_write = if cache_committed {
            RiseUserThemeSettingsCacheSource::PersistentStore.raw_value()
        } else {
            "none"
        };
        let is_error = transport_error.is_some();
        SaiHttpTraceEntry {
            id: span.id,
            transport: SaiTraceTransport::Websocket,
            created_at: span.started_at,
            is_pending,
            method: "WS".to_owned(),
            url: format!("user/{}", span.endpoint.method),
            fetch_id: Some(format!("user.theme-settings.{}", span.generation)),
            duration_ms,
            request_headers: Self::headers(&[
                ("x-rise-policy", replay),
                ("x-rise-presentation-cache", "retained"),
            ]),
            request_body_wire: format!("{{\"generation\":{}}}", span.generation),
            response_status: status,
            response_headers: Self::headers(&[
                ("x-rise-cache-write", cache_write),
                ("x-rise-rpc-attempts", if is_pending { "0" } else { "1" }),
                ("x-rise-source", if is_error { "network-error" } else { "network" }),
                ("x-rise-ui-result", if published { "published" } else { "retained" }),
            ]),
            response_body_preview: String::new(),
            transport_error,
            trace_tags: vec![SaiTraceTag::RISE_ENGINE.to_owned(), SaiTraceTag::DIRECT_RPC.to_owned()],
        }
    }

    fn headers(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect()
    }

    /// inserts or updates the entry on the main thread,
    /// it lands before the caller goes on so a finish never beats its start
    async fn upsert(make: impl FnOnce(Date) -> SaiHttpTraceEntry + Send + 'static) {
        let Some(main) = RiseEngineMainHop::installed() else {
            return;
        };
        let _ = main
            .run(move |cx: AsyncApp| {
                async move {
                    cx.update(|cx| {
                        let entry = make(Date::now());
                        SaiHttpTraceStore::shared(cx).update(cx, |store, cx| store.upsert(entry, cx));
                    });
                    Ok::<(), anyhow::Error>(())
                }
                .boxed_local()
            })
            .await;
    }
}
