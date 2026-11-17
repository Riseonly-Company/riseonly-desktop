#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChatFolderSheetEntryPoint {
    Create,
    Edit,
    IncludeChats,
    ExcludeChats,
    Delete,
    Limits,
}

pub enum ChatFolderSheetPresentationContract {}

impl ChatFolderSheetPresentationContract {
    pub const SNAP_FRACTIONS: [f32; 1] = [0.93];
    pub const USES_CUSTOM_FULL_PAGE: bool = false;
    pub const CONTENT_SCROLLABLE: bool = false;
    pub const ROOT_HOST_COUNT: usize = 1;

    pub fn uses_shared_editor_host(entry_point: ChatFolderSheetEntryPoint) -> bool {
        match entry_point {
            ChatFolderSheetEntryPoint::Create
            | ChatFolderSheetEntryPoint::Edit
            | ChatFolderSheetEntryPoint::IncludeChats
            | ChatFolderSheetEntryPoint::ExcludeChats => true,
            ChatFolderSheetEntryPoint::Delete | ChatFolderSheetEntryPoint::Limits => false,
        }
    }
}
