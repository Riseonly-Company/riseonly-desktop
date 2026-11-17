use std::ops::Range;
use std::rc::Rc;

use gpui::{App, Global, Window};
use rise_core::foundation::ns_range::NSRange;
use rise_canon::RiseRequestFieldAddress;
use rise_ui::swiftui::binding::Binding;
use rise_validate::{TextSchema, Unit};

use crate::core::lib::canon_rules::ValidationSchemaCanonRules;
use crate::core::rise_engine_bridge::rise_canon_endpoints::RiseCanonEndpoints;
use crate::core::rise_engine_bridge::rise_canon_fields::RiseCanonFields;

/// canon rule for one input field, resolved once
/// screen gets ceiling and required flag and closed list and violation text
/// no server state so reading it from a view is fine
#[derive(Clone, Copy, Debug)]
pub struct RiseFieldRule {
    pub address: Option<RiseRequestFieldAddress>,

    pub schema: Option<TextSchema>,

    premium: bool,
}

impl RiseFieldRule {
    pub fn new(endpoint: &str, pointer: &str) -> Self {
        let address = RiseCanonFields::field(endpoint, pointer);
        let schema = address.as_ref().and_then(RiseCanonFields::schema_for);
        Self { address, schema, premium: false }
    }

    pub fn new_service_method(service: &str, method: &str, pointer: &str) -> Self {
        let endpoint = RiseCanonEndpoints::required(service, method).name;
        Self::new(endpoint, pointer)
    }

    pub const NONE: RiseFieldRule = RiseFieldRule { address: None, schema: None, premium: false };

    /// rule with no wire address, looked up by name in canon validation
    /// a value may ride in someone elses envelope, eg a mini app payload in a message
    /// a miss is a typo in code so dont tolerate it
    #[track_caller]
    pub fn client_rule(field: &str) -> Self {
        let Some(schema) = RiseCanonFields::schema_named(field) else {
            debug_assert!(false, "rule {field} is missing from canon/validation");
            return Self::NONE;
        };
        Self { address: None, schema: Some(schema), premium: false }
    }

    /// ceiling in the rules own unit
    pub fn maximum(&self) -> Option<usize> {
        self.schema?.maximum_for_tier(self.premium).map(|(value, _)| value)
    }

    /// ceiling unit
    /// character means a unicode scalar and not a grapheme, a family emoji is one grapheme but four scalars
    pub fn unit(&self) -> Option<Unit> {
        self.schema?.maximum_for_tier(self.premium).map(|(_, unit)| unit)
    }

    pub fn is_required(&self) -> bool {
        self.address.is_some_and(|address| address.is_required)
    }

    /// true if the value is picked from a list and not typed
    pub fn allowed_values(&self) -> &'static [&'static str] {
        self.address.map_or(&[], |address| address.allowed_values)
    }

    /// what the field keeps while typing, clamped to the ceiling in its unit without splitting a character
    /// pasted long text is cut like typed text
    pub fn clamped(&self, raw: &str) -> String {
        match self.schema {
            Some(schema) => schema.clamped_for_tier(raw, self.premium),
            None => raw.to_owned(),
        }
    }

    /// rule for one tier, every tiered ceiling becomes that tiers plain ceiling
    /// so a subscribers field clamps at the edge the server accepts
    pub fn for_tier(&self, premium: bool) -> RiseFieldRule {
        if self.schema.is_none() {
            return *self;
        }
        RiseFieldRule { premium, ..*self }
    }

    /// lower bound in the rules unit, none when canon declared none
    pub fn minimum(&self) -> Option<usize> {
        self.schema?.minimum().map(|(value, _)| value)
    }

    /// characters still needed to pass the lower bound
    /// none means already enough or no bound, same for the caller
    /// computed on the normalized value since thats what the server checks
    pub fn shortfall(&self, raw: &str) -> Option<usize> {
        let schema = self.schema?;
        let (minimum, unit) = schema.minimum()?;
        let length = unit.count(&schema.normalize(raw));
        if length >= minimum { None } else { Some(minimum - length) }
    }

    /// room left, none when there is no ceiling and so no counter
    /// the floor at zero isnt cosmetic since the value is already clamped
    pub fn remaining(&self, value: &str) -> Option<usize> {
        let (maximum, unit) = self.schema?.maximum_for_tier(self.premium)?;
        Some(maximum.saturating_sub(unit.count(value)))
    }

    /// first violated rule as a ready phrase in the users language
    /// the value is checked as is, canon check normalizes first and would call Rise_One  valid
    pub fn error_text(&self, raw: &str) -> Option<String> {
        self.schema?.error_text_for_raw_value_for_tier(raw, self.premium)
    }

    /// true if the value is fit to send
    /// an empty required field is not, but the message shows only after the user touched it
    pub fn is_satisfied(&self, raw: &str) -> bool {
        match self.schema {
            Some(schema) => schema.check_for(raw, self.premium).is_ok(),
            None => true,
        }
    }

    /// binding that clamps on write so the call site doesnt clamp by hand
    pub fn binding(&self, source: Binding<String>) -> Binding<String> {
        let rule = *self;
        let reader = source.clone();
        Binding::new(
            move |cx| reader.get(cx),
            move |raw, window, cx| {
                let kept = rule.clamped(&raw);
                if kept == raw {
                    source.set(raw, window, cx);
                    return;
                }
                match RiseFocusedTextInput::clamp(&raw, &kept, window, cx) {
                    RiseFocusedTextInputOutcome::Composing => source.set(raw, window, cx),
                    RiseFocusedTextInputOutcome::Clamped | RiseFocusedTextInputOutcome::NotFocused => {
                        source.set(kept, window, cx)
                    }
                }
            },
        )
    }
}

/// the text input behind a focused field
/// implemented by the rise-ui text field, it reports itself on focus
pub trait RiseTextInputCaret {
    fn text(&self, cx: &App) -> String;

    /// open ime composition in bytes of the text, none when nothing is composed
    fn marked_range(&self, cx: &App) -> Option<Range<usize>>;

    /// selection in utf-16 units
    fn selected_range(&self, cx: &App) -> NSRange;

    /// writes the text with the caret at the end
    fn replace(&self, text: &str, window: &mut Window, cx: &mut App);

    fn replace_keeping_caret(&self, text: &str, window: &mut Window, cx: &mut App);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RiseFocusedTextInputOutcome {
    Clamped,
    Composing,
    NotFocused,
}

/// the text input behind the focused field
#[derive(Default)]
pub struct RiseFocusedTextInput {
    found: Option<Rc<dyn RiseTextInputCaret>>,
}

impl Global for RiseFocusedTextInput {}

impl RiseFocusedTextInput {
    pub fn clamp(typed: &str, kept: &str, window: &mut Window, cx: &mut App) -> RiseFocusedTextInputOutcome {
        Self::write(typed, kept, false, window, cx)
    }

    /// clamp for a value shaped anywhere in the text (a folded letter or stripped @)
    /// the caret keeps its distance from the end instead of jumping there
    pub fn shape(typed: &str, kept: &str, window: &mut Window, cx: &mut App) -> RiseFocusedTextInputOutcome {
        Self::write(typed, kept, true, window, cx)
    }

    fn write(typed: &str, kept: &str, keeping_caret: bool, window: &mut Window, cx: &mut App) -> RiseFocusedTextInputOutcome {
        let Some(input) = Self::first_responder(cx) else {
            return RiseFocusedTextInputOutcome::NotFocused;
        };
        if input.text(cx) != typed {
            return RiseFocusedTextInputOutcome::NotFocused;
        }
        if input.marked_range(cx).is_some() {
            return RiseFocusedTextInputOutcome::Composing;
        }
        if keeping_caret {
            input.replace_keeping_caret(kept, window, cx);
        } else {
            input.replace(kept, window, cx);
        }
        RiseFocusedTextInputOutcome::Clamped
    }

    /// caret of the focused field showing the text in utf-16 units
    /// none when another field or none is focused, read before an emoji panel takes the keyboard place
    pub fn caret(showing: &str, cx: &App) -> Option<NSRange> {
        let input = Self::first_responder(cx)?;
        if input.text(cx) != showing {
            return None;
        }
        Some(input.selected_range(cx))
    }

    fn first_responder(cx: &App) -> Option<Rc<dyn RiseTextInputCaret>> {
        cx.try_global::<Self>().and_then(|focused| focused.found.clone())
    }

    /// called by the field when it takes focus
    pub fn report(input: Rc<dyn RiseTextInputCaret>, cx: &mut App) {
        cx.default_global::<Self>().found = Some(input);
    }

    /// called by the field when it loses focus
    /// a field that isnt the reported one leaves the newer report alone
    pub fn resign(input: &Rc<dyn RiseTextInputCaret>, cx: &mut App) {
        let focused = cx.default_global::<Self>();
        if focused.found.as_ref().is_some_and(|found| Rc::ptr_eq(found, input)) {
            focused.found = None;
        }
    }
}
