use std::any::Any;
use std::collections::HashSet;
use std::fmt::Debug;
use std::panic::Location;
use std::sync::LazyLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use gpui::{App, Context, Entity};
use parking_lot::Mutex;
use rise_core::foundation::foundation_text::{character_count, character_prefix};
use rise_core::foundation::foundation_url::Url;
use rise_core::foundation::foundation_uuid::Uuid;
use serde_json::Value;

use super::sai_log_entry::SaiLogEntry;
use super::sai_log_kind::SaiLogKind;

const CONSOLE_TARGET: &str = "riseonly.sai.console";
const MAXIMUM_PENDING_LINES: usize = 600;
const MAX_ENTRIES: usize = 600;
const MAX_DEPTH: usize = 6;
const MAX_PARTS: usize = 64;
const MAX_PART_CHARACTERS: usize = 4_000;
const MAX_DESCRIPTION_CHARACTERS: usize = 8_000;
const ELLIPSIS: &str = "\u{2026}";
const NOTIFY_COALESCE: Duration = Duration::from_millis(16);

static HAS_READER: AtomicBool = AtomicBool::new(false);

#[derive(Default)]
struct SaiConsoleWriterState {
    pending: Vec<String>,
    dropped_count: usize,
    is_drain_scheduled: bool,
}

/// console writer, bounded so a logging burst never grows memory
/// drained off the main thread
#[derive(Default)]
struct SaiConsoleWriter {
    state: Mutex<SaiConsoleWriterState>,
}

impl SaiConsoleWriter {
    fn enqueue(&'static self, line: String, cx: &App) {
        let should_schedule = {
            let mut state = self.state.lock();
            if state.pending.len() < MAXIMUM_PENDING_LINES {
                state.pending.push(line);
            } else {
                state.dropped_count += 1;
            }
            if state.is_drain_scheduled {
                false
            } else {
                state.is_drain_scheduled = true;
                true
            }
        };

        if should_schedule {
            cx.background_executor()
                .spawn(async move { self.drain() })
                .detach();
        }
    }

    fn drain(&self) {
        loop {
            let (batch, dropped) = {
                let mut state = self.state.lock();
                if state.pending.is_empty() && state.dropped_count == 0 {
                    state.is_drain_scheduled = false;
                    return;
                }
                let batch = std::mem::take(&mut state.pending);
                let dropped = std::mem::take(&mut state.dropped_count);
                (batch, dropped)
            };

            for line in batch {
                tracing::debug!(target: CONSOLE_TARGET, "{line}");
            }
            if dropped > 0 {
                tracing::debug!(target: CONSOLE_TARGET, "[SaiLogger] dropped {dropped} console lines during burst");
            }
        }
    }
}

static CONSOLE_WRITER: LazyLock<SaiConsoleWriter> = LazyLock::new(SaiConsoleWriter::default);

/// shared logger, a ring of the newest entries observed by the debugger
pub struct SaiLogger {
    entry_ring: Vec<Option<SaiLogEntry>>,
    entry_write_index: usize,
    entry_count: usize,
    notify_scheduled: bool,
}

impl SaiLogger {
    pub fn set_has_reader(has_reader: bool) {
        HAS_READER.store(has_reader, Ordering::Relaxed);
    }

    fn notify_readers(&mut self, cx: &mut Context<Self>) {
        if self.notify_scheduled || !HAS_READER.load(Ordering::Relaxed) {
            return;
        }
        self.notify_scheduled = true;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(NOTIFY_COALESCE).await;
            let _ = this.update(cx, |this, cx| {
                this.notify_scheduled = false;
                cx.notify();
            });
        })
        .detach();
    }

    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, |_| Self::new())
    }

    fn new() -> Self {
        Self {
            entry_ring: vec![None; MAX_ENTRIES],
            entry_write_index: 0,
            entry_count: 0,
            notify_scheduled: false,
        }
    }

    pub fn entries(&self) -> Vec<SaiLogEntry> {
        if self.entry_count == 0 {
            return Vec::new();
        }
        let mut result = Vec::with_capacity(self.entry_count);
        for offset in 0..self.entry_count {
            let index = (self.entry_write_index + MAX_ENTRIES - 1 - offset) % MAX_ENTRIES;
            if let Some(entry) = &self.entry_ring[index] {
                result.push(entry.clone());
            }
        }
        result
    }

    pub fn record<V: Debug + 'static>(
        &mut self,
        name: &str,
        payload: &V,
        kind: SaiLogKind,
        location: &Location<'_>,
        cx: &mut Context<Self>,
    ) {
        let message = Self::format_value(payload, 0);
        let entry = SaiLogEntry::new(name, kind, message, location);
        let console_line = cfg!(any(debug_assertions, feature = "sai_debug_tools"))
            .then(|| entry.console_line());
        self.entry_ring[self.entry_write_index] = Some(entry);
        self.entry_write_index = (self.entry_write_index + 1) % MAX_ENTRIES;
        self.entry_count = MAX_ENTRIES.min(self.entry_count + 1);
        self.notify_readers(cx);
        if let Some(line) = console_line {
            CONSOLE_WRITER.enqueue(line, cx);
        }
    }

    pub fn filtered(&self, kind: &SaiLogKind) -> Vec<SaiLogEntry> {
        self.entries().into_iter().filter(|entry| &entry.kind == kind).collect()
    }

    pub fn filtered_kinds(&self, kinds: &HashSet<SaiLogKind>) -> Vec<SaiLogEntry> {
        self.entries().into_iter().filter(|entry| kinds.contains(&entry.kind)).collect()
    }

    pub fn clear(&mut self, cx: &mut Context<Self>) {
        self.entry_ring = vec![None; MAX_ENTRIES];
        self.entry_write_index = 0;
        self.entry_count = 0;
        cx.notify();
    }

    pub fn remove_entry(&mut self, id: Uuid, cx: &mut Context<Self>) {
        let kept: Vec<SaiLogEntry> = self.entries().into_iter().filter(|entry| entry.id != id).collect();
        self.replace_entries(kept);
        cx.notify();
    }

    pub fn remove_all(&mut self, of_kind: &SaiLogKind, cx: &mut Context<Self>) {
        let kept: Vec<SaiLogEntry> = self.entries().into_iter().filter(|entry| &entry.kind != of_kind).collect();
        self.replace_entries(kept);
        cx.notify();
    }

    fn replace_entries(&mut self, newest_first: Vec<SaiLogEntry>) {
        self.entry_ring = vec![None; MAX_ENTRIES];
        self.entry_write_index = 0;
        self.entry_count = 0;
        for entry in newest_first.into_iter().take(MAX_ENTRIES).rev() {
            self.entry_ring[self.entry_write_index] = Some(entry);
            self.entry_write_index = (self.entry_write_index + 1) % MAX_ENTRIES;
            self.entry_count += 1;
        }
    }

    fn format_value<V: Debug + 'static>(value: &V, depth: usize) -> String {
        if depth >= MAX_DEPTH {
            return ELLIPSIS.to_owned();
        }
        let any: &dyn Any = value;
        if let Some(rendered) = Self::format_scalar(any) {
            return rendered;
        }
        if let Some(json) = any.downcast_ref::<Value>() {
            return Self::format_json(json, depth);
        }
        if let Some(json) = any.downcast_ref::<serde_json::Map<String, Value>>() {
            return Self::format_json_object(json, depth);
        }
        if let Some(json) = any.downcast_ref::<Option<Value>>() {
            return match json {
                Some(inner) => Self::format_json(inner, depth + 1),
                None => "nil".to_owned(),
            };
        }
        Self::truncated_description(format!("{value:?}"))
    }

    fn format_scalar(any: &dyn Any) -> Option<String> {
        if any.is::<()>() {
            return Some("()".to_owned());
        }
        if let Some(v) = any.downcast_ref::<String>() {
            return Some(v.clone());
        }
        if let Some(v) = any.downcast_ref::<&str>() {
            return Some((*v).to_owned());
        }
        if let Some(v) = any.downcast_ref::<bool>() {
            return Some(v.to_string());
        }
        if let Some(v) = any.downcast_ref::<char>() {
            return Some(v.to_string());
        }
        macro_rules! integer {
            ($($t:ty),*) => {
                $(if let Some(v) = any.downcast_ref::<$t>() {
                    return Some(v.to_string());
                })*
            };
        }
        integer!(i64, i8, i16, i32, isize, u64, u8, u16, u32, usize);
        if let Some(v) = any.downcast_ref::<f32>() {
            return Some(v.to_string());
        }
        if let Some(v) = any.downcast_ref::<f64>() {
            return Some(v.to_string());
        }
        if let Some(v) = any.downcast_ref::<Url>() {
            return Some(v.absolute_string().to_owned());
        }
        if let Some(v) = any.downcast_ref::<anyhow::Error>() {
            return Some(v.to_string());
        }
        None
    }

    fn format_json(value: &Value, depth: usize) -> String {
        if depth >= MAX_DEPTH {
            return ELLIPSIS.to_owned();
        }
        match value {
            Value::Null => "nil".to_owned(),
            Value::Bool(v) => v.to_string(),
            Value::Number(v) => v.to_string(),
            Value::String(v) => v.clone(),
            Value::Array(items) => {
                let mut parts: Vec<String> = Vec::new();
                let mut character_total = 0;
                let mut truncated = false;
                for item in items {
                    if parts.len() >= MAX_PARTS || character_total >= MAX_PART_CHARACTERS {
                        truncated = true;
                        break;
                    }
                    let part = Self::format_json(item, depth + 1);
                    let remaining = MAX_PART_CHARACTERS.saturating_sub(character_total);
                    let part_count = character_count(&part);
                    if part_count > remaining {
                        parts.push(character_prefix(&part, remaining).to_owned());
                        truncated = true;
                        break;
                    }
                    parts.push(part);
                    character_total += part_count + 2;
                }
                if parts.is_empty() {
                    return "[]".to_owned();
                }
                let suffix = if truncated { format!(", {ELLIPSIS}") } else { String::new() };
                format!("[{}{suffix}]", parts.join(", "))
            }
            Value::Object(map) => Self::format_json_object(map, depth),
        }
    }

    fn format_json_object(map: &serde_json::Map<String, Value>, depth: usize) -> String {
        if depth >= MAX_DEPTH {
            return ELLIPSIS.to_owned();
        }
        let mut pairs: Vec<String> = Vec::new();
        let mut character_total = 0;
        let mut truncated = false;
        for (key, value) in map {
            if pairs.len() >= MAX_PARTS || character_total >= MAX_PART_CHARACTERS {
                truncated = true;
                break;
            }
            let rendered = format!("{key}: {}", Self::format_json(value, depth + 1));
            let remaining = MAX_PART_CHARACTERS.saturating_sub(character_total);
            let rendered_count = character_count(&rendered);
            if rendered_count > remaining {
                pairs.push(character_prefix(&rendered, remaining).to_owned());
                truncated = true;
                break;
            }
            pairs.push(rendered);
            character_total += rendered_count + 2;
        }
        if pairs.is_empty() {
            return "[:]".to_owned();
        }
        let suffix = if truncated { format!(", {ELLIPSIS}") } else { String::new() };
        format!("{{{}{suffix}}}", pairs.join(", "))
    }

    fn truncated_description(description: String) -> String {
        if character_count(&description) > MAX_DESCRIPTION_CHARACTERS {
            format!("{}{ELLIPSIS}", character_prefix(&description, MAX_DESCRIPTION_CHARACTERS))
        } else {
            description
        }
    }
}
