#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum SaiLogKind {
    Default,
    Ui,
    Network,
    Websocket,
    Cache,
    Util,
    Event,
    Debug,
    Custom(String),
}

impl SaiLogKind {
    pub fn filter_key(&self) -> String {
        match self {
            Self::Default => "default".to_owned(),
            Self::Ui => "ui".to_owned(),
            Self::Network => "network".to_owned(),
            Self::Websocket => "websocket".to_owned(),
            Self::Cache => "cache".to_owned(),
            Self::Util => "util".to_owned(),
            Self::Event => "event".to_owned(),
            Self::Debug => "debug".to_owned(),
            Self::Custom(s) => format!("custom:{s}"),
        }
    }

    pub fn short_label(&self) -> &str {
        match self {
            Self::Default => "default",
            Self::Ui => "ui",
            Self::Network => "net",
            Self::Websocket => "ws",
            Self::Cache => "cache",
            Self::Util => "util",
            Self::Event => "event",
            Self::Debug => "debug",
            Self::Custom(s) => s,
        }
    }
}
