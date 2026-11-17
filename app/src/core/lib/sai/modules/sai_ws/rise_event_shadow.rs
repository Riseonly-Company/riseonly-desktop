#![cfg(debug_assertions)]

use std::collections::HashSet;
use std::sync::Arc;

use futures::StreamExt;
use futures::channel::mpsc;
use gpui::App;
use rise_canon::RiseEvent;
use rise_engine::rise_sync::RiseSync;
use rise_engine::{RiseJSONValue, RiseWirePushEnvelope};

use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::sai::Sai;

const VERDICT_OK: &str = "ok";
const VERDICT_UNTYPED: &str = "untyped";

pub enum RiseEventShadow {}

impl RiseEventShadow {
    pub fn install(cx: &mut App) {
        let (pushes, mut inbox) = mpsc::unbounded::<(String, RiseJSONValue)>();
        RiseSync::set_push_observer(Some(Arc::new(move |push: &RiseWirePushEnvelope| {
            let _ = pushes.unbounded_send((push.r#type.clone(), push.data.clone()));
        })));
        cx.spawn(async move |cx| {
            let mut reported: HashSet<String> = HashSet::new();
            while let Some((push_type, data)) = inbox.next().await {
                let decode_name = push_type.clone();
                let verdict = cx
                    .background_executor()
                    .spawn(async move { Self::verdict(&decode_name, &data) })
                    .await;
                let Some(verdict) = verdict else {
                    continue;
                };
                if !reported.insert(push_type.clone()) {
                    continue;
                }
                tracing::info!("CanonEvent {verdict} {push_type}");
                if verdict != VERDICT_OK {
                    cx.update(|cx| {
                        Sai::log(
                            "CanonEvent",
                            move || format!("{push_type}: {verdict}"),
                            SaiLogKind::Network,
                            cx,
                        );
                    });
                }
            }
        })
        .detach();
    }

    fn verdict(name: &str, data: &RiseJSONValue) -> Option<String> {
        let bytes = data.data(false).ok()?;
        let verdict = match RiseEvent::decode(name, &bytes) {
            Ok(RiseEvent::Untyped { .. }) => VERDICT_UNTYPED.to_owned(),
            Ok(_) => VERDICT_OK.to_owned(),
            Err(error) => format!("MISMATCH {error}"),
        };
        Some(verdict)
    }
}
