use crate::core::push_notifications::universal_link_parser_hooks::{
    UniversalLinkParserHooks, set_universal_link_parser_hooks,
};
use crate::modules::bot::stores::bot::bot_actions::bot_types::BotTagRules;

pub fn install_universal_link_parser_hooks() {
    set_universal_link_parser_hooks(UniversalLinkParserHooks { is_valid_bot_tag: BotTagRules::is_valid });
}
