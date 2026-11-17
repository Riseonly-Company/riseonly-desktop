use gpui::{App, Context, Entity};
use rise_core::foundation::foundation_date::{Date, RiseClock};
use rise_core::foundation::foundation_uuid::Uuid;
use serde_json::{Map, Value};

use crate::core::lib::sai::modules::sai_debugger::sai_http_trace_recorder::SaiHttpTraceRecorder;

#[derive(Clone, Debug, PartialEq)]
pub struct SaiWsEventEntry {
    pub id: Uuid,
    pub created_at: Date,
    pub r#type: String,
    pub payload_json: String,
    pub handled: bool,
}

impl SaiWsEventEntry {
    pub fn new(created_at: Date, r#type: String, payload_json: String, handled: bool) -> Self {
        Self {
            id: Uuid::new_v4(),
            created_at,
            r#type,
            payload_json,
            handled,
        }
    }

    pub fn copy_block_text(&self) -> String {
        format!("[{}] handled={}\n{}", self.r#type, self.handled, self.payload_json)
    }
}

pub struct SaiWsEventStore {
    entries: Vec<SaiWsEventEntry>,
    max_entries: usize,
}

impl SaiWsEventStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            entries: Vec::new(),
            max_entries: 300,
        }
    }

    const fn debug_tools() -> bool {
        cfg!(debug_assertions) || cfg!(feature = "sai_debug_tools")
    }

    pub fn entries(&self) -> &[SaiWsEventEntry] {
        &self.entries
    }

    pub fn record(&mut self, r#type: &str, json: &Map<String, Value>, handled: bool, cx: &mut Context<Self>) {
        if !Self::debug_tools() {
            return;
        }
        let pretty = SaiHttpTraceRecorder::pretty_json_string_from_json_object(json);
        self.entries.insert(
            0,
            SaiWsEventEntry::new(RiseClock::now(), r#type.to_owned(), pretty, handled),
        );
        if self.entries.len() > self.max_entries {
            self.entries.truncate(self.max_entries);
        }
        cx.notify();
    }

    pub fn remove_entry(&mut self, id: Uuid, cx: &mut Context<Self>) {
        self.entries.retain(|entry| entry.id != id);
        cx.notify();
    }

    pub fn clear(&mut self, cx: &mut Context<Self>) {
        self.entries.clear();
        cx.notify();
    }
}
