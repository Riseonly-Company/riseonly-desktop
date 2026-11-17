use crate::core::lib::sai::sai_guide_hooks::{SaiGuideHooks, set_sai_guide_hooks};
use crate::modules::guides::shared::guide_sandbox::GuideSandbox;

pub(crate) fn install_guide_sandbox_hooks() {
    set_sai_guide_hooks(SaiGuideHooks {
        refuses_json_object: Box::new(GuideSandbox::refuses_json_object),
        refuses_service_method: Box::new(GuideSandbox::refuses_service_method),
    });
}
