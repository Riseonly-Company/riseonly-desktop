use rise_core::foundation::foundation_uuid::Uuid;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EditorMode {
    Initial,
    Crop,
    Draw,
    Text { entity_id: Option<Uuid> },
    Adjust,
    TagPeople,
}
