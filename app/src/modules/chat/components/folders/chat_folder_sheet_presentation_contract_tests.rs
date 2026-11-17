use super::chat_folder_sheet_presentation_contract::{ChatFolderSheetEntryPoint, ChatFolderSheetPresentationContract};

#[test]
fn folder_flows_use_one_presentation_host() {
    assert_eq!(ChatFolderSheetPresentationContract::ROOT_HOST_COUNT, 1);
    assert!(!ChatFolderSheetPresentationContract::USES_CUSTOM_FULL_PAGE);
    assert_eq!(ChatFolderSheetPresentationContract::SNAP_FRACTIONS, [0.93]);
}

#[test]
fn content_owns_its_only_scroll_container() {
    assert!(!ChatFolderSheetPresentationContract::CONTENT_SCROLLABLE);
}

#[test]
fn editor_and_chat_selectors_route_through_shared_host() {
    assert!(ChatFolderSheetPresentationContract::uses_shared_editor_host(ChatFolderSheetEntryPoint::Create));
    assert!(ChatFolderSheetPresentationContract::uses_shared_editor_host(ChatFolderSheetEntryPoint::Edit));
    assert!(ChatFolderSheetPresentationContract::uses_shared_editor_host(ChatFolderSheetEntryPoint::IncludeChats));
    assert!(ChatFolderSheetPresentationContract::uses_shared_editor_host(ChatFolderSheetEntryPoint::ExcludeChats));
}

#[test]
fn inline_actions_do_not_allocate_another_sheet_host() {
    assert!(!ChatFolderSheetPresentationContract::uses_shared_editor_host(ChatFolderSheetEntryPoint::Delete));
    assert!(!ChatFolderSheetPresentationContract::uses_shared_editor_host(ChatFolderSheetEntryPoint::Limits));
}
