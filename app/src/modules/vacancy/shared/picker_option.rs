/// one row of a vacancy form picker sheet. lives beside the catalog so it builds rows without
/// pulling in ui code
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PickerOption {
    pub id: String,
    pub leading: Option<String>,
    pub title: String,
    pub subtitle: Option<String>,
}

impl PickerOption {
    /// option with just id and title, no leading and no subtitle
    pub fn new(id: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            leading: None,
            title: title.into(),
            subtitle: None,
        }
    }

    /// option with id, leading text and title
    pub fn with_leading(id: impl Into<String>, leading: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            leading: Some(leading.into()),
            title: title.into(),
            subtitle: None,
        }
    }
}
