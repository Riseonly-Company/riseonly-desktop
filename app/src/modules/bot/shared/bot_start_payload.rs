use rise_deeplink::{ParameterKind, ParameterKindSpec};

pub enum BotStartPayload {}

impl BotStartPayload {
    fn rule() -> &'static ParameterKindSpec {
        ParameterKind::BotStartPayload.spec()
    }

    pub fn is_valid(value: &str) -> bool {
        let rule = Self::rule();
        let bytes = value.as_bytes();
        (rule.minimum_utf8_bytes..=rule.maximum_utf8_bytes).contains(&bytes.len())
            && bytes.iter().all(|byte| byte.is_ascii_alphanumeric() || *byte == b'-' || *byte == b'_')
    }
}
