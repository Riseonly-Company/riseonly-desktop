use crate::modules::story::stores::story::story_services::story_share_handoff_models::StoryShareHandoffPresentationStage;

#[test]
fn foreground_editing_stages_own_the_interface() {
    assert!(StoryShareHandoffPresentationStage::Preparing.blocks_interface());
    assert!(StoryShareHandoffPresentationStage::Editing.blocks_interface());
    assert!(StoryShareHandoffPresentationStage::ForegroundExport.blocks_interface());
    assert!(StoryShareHandoffPresentationStage::ForegroundComposer.blocks_interface());
}

#[test]
fn send_and_recovery_stages_never_cover_the_profile() {
    assert!(!StoryShareHandoffPresentationStage::BackgroundWork.blocks_interface());
    assert!(!StoryShareHandoffPresentationStage::BackgroundFailure.blocks_interface());
    assert!(!StoryShareHandoffPresentationStage::Idle.blocks_interface());
}

#[test]
fn only_active_background_work_drives_the_avatar_ring() {
    assert!(StoryShareHandoffPresentationStage::BackgroundWork.shows_profile_avatar_activity());

    for stage in StoryShareHandoffPresentationStage::ALL_CASES {
        if stage == StoryShareHandoffPresentationStage::BackgroundWork {
            continue;
        }
        assert!(!stage.shows_profile_avatar_activity());
    }
}
