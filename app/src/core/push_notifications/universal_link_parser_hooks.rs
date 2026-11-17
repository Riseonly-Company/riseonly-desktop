use crate::core::feature_hooks::FeatureHookSlot;

pub struct UniversalLinkParserHooks {
    pub is_valid_bot_tag: fn(&str) -> bool,
}

pub(crate) static UNIVERSAL_LINK_PARSER_HOOKS: FeatureHookSlot<UniversalLinkParserHooks> =
    FeatureHookSlot::new("UNIVERSAL_LINK_PARSER_HOOKS");

pub fn set_universal_link_parser_hooks(hooks: UniversalLinkParserHooks) {
    let _ = UNIVERSAL_LINK_PARSER_HOOKS.set(hooks);
}

pub fn universal_link_parser_hooks() -> Option<&'static UniversalLinkParserHooks> {
    UNIVERSAL_LINK_PARSER_HOOKS.get()
}
