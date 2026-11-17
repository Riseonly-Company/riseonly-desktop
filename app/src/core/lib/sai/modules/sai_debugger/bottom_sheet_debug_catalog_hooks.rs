#[cfg(any(debug_assertions, feature = "sai_debug_tools"))]
use std::collections::HashMap;
#[cfg(any(debug_assertions, feature = "sai_debug_tools"))]
use std::rc::Rc;

use crate::core::feature_hooks::FeatureHookSlot;
#[cfg(any(debug_assertions, feature = "sai_debug_tools"))]
use crate::core::ui::bottom_sheet_ui::bottom_sheet_debug_registration::{
    BottomSheetDebugRegistration, BottomSheetDebugRegistrationModule,
};

/// what the sheet gallery asks the catalog of feature previews, no fields without the debug tools
pub struct BottomSheetDebugCatalogHooks {
    #[cfg(any(debug_assertions, feature = "sai_debug_tools"))]
    pub entries: fn() -> Rc<Vec<BottomSheetDebugRegistration>>,
    #[cfg(any(debug_assertions, feature = "sai_debug_tools"))]
    pub counts: fn() -> Rc<HashMap<BottomSheetDebugRegistrationModule, i64>>,
    #[cfg(any(debug_assertions, feature = "sai_debug_tools"))]
    pub modules: fn() -> Vec<BottomSheetDebugRegistrationModule>,
}

pub(crate) static BOTTOM_SHEET_DEBUG_CATALOG_HOOKS: FeatureHookSlot<BottomSheetDebugCatalogHooks> =
    FeatureHookSlot::new("BOTTOM_SHEET_DEBUG_CATALOG_HOOKS");

pub fn set_bottom_sheet_debug_catalog_hooks(hooks: BottomSheetDebugCatalogHooks) {
    let _ = BOTTOM_SHEET_DEBUG_CATALOG_HOOKS.set(hooks);
}

pub fn bottom_sheet_debug_catalog_hooks() -> Option<&'static BottomSheetDebugCatalogHooks> {
    BOTTOM_SHEET_DEBUG_CATALOG_HOOKS.get()
}
