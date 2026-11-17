#![cfg(debug_assertions)]

use std::collections::HashSet;

use gpui::{App, Global};
use rise_canon::rise_response;
use serde_json::Value;

use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::sai::Sai;

/// set of reported reply shadows, one per app and main thread only
#[derive(Default)]
struct RiseReplyShadowReported(HashSet<String>);

impl Global for RiseReplyShadowReported {}

pub enum RiseReplyShadow {}

impl RiseReplyShadow {
    pub fn check(service: &str, method: &str, payload: Option<&Value>, cx: &mut App) {
        let Some(payload) = payload.filter(|payload| payload.is_object() || payload.is_array())
        else {
            return;
        };
        let wire = format!("{service}.{method}");
        if cx.default_global::<RiseReplyShadowReported>().0.contains(&wire) {
            return;
        }
        let captured = payload.clone();
        let service = service.to_owned();
        let method = method.to_owned();
        cx.spawn(async move |cx| {
            let problem = cx
                .background_executor()
                .spawn(async move {
                    let data = serde_json::to_vec(&captured).ok()?;
                    Some(rise_response::mismatch(&service, &method, &data))
                })
                .await;
            let Some(problem) = problem else {
                return;
            };
            cx.update(|cx| {
                if !cx.default_global::<RiseReplyShadowReported>().0.insert(wire.clone()) {
                    return;
                }
                match problem {
                    Some(problem) => {
                        tracing::info!("CanonReply MISMATCH {wire} {problem}");
                        Sai::log(
                            "CanonReply",
                            move || format!("{wire} does not decode as its canon type: {problem}"),
                            SaiLogKind::Network,
                            cx,
                        );
                    }
                    None => {
                        tracing::info!("CanonReply ok {wire}");
                    }
                }
            });
        })
        .detach();
    }
}
