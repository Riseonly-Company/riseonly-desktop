use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_deeplink::{ENVIRONMENTS, Environment};

pub const PLACEHOLDERS: [&str; 6] = [
    "@ICON_PATH@",
    "@FILE_VERSION@",
    "@FILE_FLAGS@",
    "@FILE_VERSION_TEXT@",
    "@APP_NAME@",
    "@SHORT_VERSION@",
];

pub const BUILD_NUMBER_FALLBACK: &str = "1";

pub fn environment_for_build(raw: Option<&str>, debug_build: bool) -> &'static Environment {
    let normalized = raw
        .map(|value| trimming_whitespaces_and_newlines(value).to_lowercase())
        .unwrap_or_default();
    let matched = ENVIRONMENTS
        .iter()
        .find(|row| row.app_env_values.contains(&normalized.as_str()));
    match matched {
        Some(row) => row,
        None if debug_build => &ENVIRONMENTS[0],
        None => &ENVIRONMENTS[2],
    }
}

pub fn display_name(environment: &str) -> Option<&'static str> {
    match environment {
        "dev" => Some("Riseonly Dev"),
        "staging" => Some("Riseonly Staging"),
        "production" => Some("Riseonly"),
        _ => None,
    }
}

pub fn build_number(raw: Option<&str>) -> &str {
    match raw {
        Some(value)
            if !value.is_empty()
                && value
                    .split('.')
                    .all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit())) =>
        {
            value
        }
        _ => BUILD_NUMBER_FALLBACK,
    }
}

pub fn file_version(major: &str, minor: &str, patch: &str, build: &str) -> [u16; 4] {
    let field = |text: &str| {
        let digits: String = text.chars().take_while(char::is_ascii_digit).collect();
        match digits.parse::<u64>() {
            Ok(value) => u16::try_from(value).unwrap_or(u16::MAX),
            Err(_) if digits.is_empty() => 0,
            Err(_) => u16::MAX,
        }
    };
    let first_build_part = build.split('.').next().unwrap_or_default();
    [field(major), field(minor), field(patch), field(first_build_part)]
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ResourceValues {
    pub app_name: &'static str,
    pub short_version: String,
    pub build: String,
    pub file_version: [u16; 4],
    pub debug_build: bool,
}

impl ResourceValues {
    pub fn resolve(
        rise_env: Option<&str>,
        rise_build_number: Option<&str>,
        debug_build: bool,
        version: [&str; 4],
    ) -> Option<Self> {
        let [short_version, major, minor, patch] = version;
        let environment = environment_for_build(rise_env, debug_build);
        let build = build_number(rise_build_number);
        Some(Self {
            app_name: display_name(environment.name)?,
            short_version: short_version.to_owned(),
            build: build.to_owned(),
            file_version: file_version(major, minor, patch, build),
            debug_build,
        })
    }

    pub fn render(&self, template: &str, icon_path: &str) -> Result<String, String> {
        let mut unknown = template.to_owned();
        for placeholder in PLACEHOLDERS {
            if !template.contains(placeholder) {
                return Err(format!("riseonly.rc.in lost {placeholder}"));
            }
            unknown = unknown.replace(placeholder, "");
        }
        if let Some(at) = unknown.find('@') {
            return Err(format!("riseonly.rc.in has an unknown placeholder near byte {at}"));
        }
        let [major, minor, patch, build] = self.file_version;
        let rendered = template
            .replace("@ICON_PATH@", &rc_text(&icon_path.replace('\\', "/")))
            .replace(
                "@FILE_VERSION_TEXT@",
                &rc_text(&format!("{} ({})", self.short_version, self.build)),
            )
            .replace("@FILE_VERSION@", &format!("{major},{minor},{patch},{build}"))
            .replace("@FILE_FLAGS@", if self.debug_build { "0x1L" } else { "0x0L" })
            .replace("@APP_NAME@", &rc_text(self.app_name))
            .replace("@SHORT_VERSION@", &rc_text(&self.short_version));
        Ok(rendered)
    }
}

pub fn rc_text(value: &str) -> String {
    value.replace('"', "\"\"")
}
