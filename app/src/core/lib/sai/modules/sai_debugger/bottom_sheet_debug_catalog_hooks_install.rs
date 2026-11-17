#[cfg(any(debug_assertions, feature = "sai_debug_tools"))]
use super::bottom_sheet_debug_catalog::BottomSheetDebugCatalog;
use super::bottom_sheet_debug_catalog_hooks::{BottomSheetDebugCatalogHooks, set_bottom_sheet_debug_catalog_hooks};

pub fn install_bottom_sheet_debug_catalog_hooks() {
    set_bottom_sheet_debug_catalog_hooks(BottomSheetDebugCatalogHooks {
        #[cfg(any(debug_assertions, feature = "sai_debug_tools"))]
        entries: BottomSheetDebugCatalog::entries,
        #[cfg(any(debug_assertions, feature = "sai_debug_tools"))]
        counts: BottomSheetDebugCatalog::counts,
        #[cfg(any(debug_assertions, feature = "sai_debug_tools"))]
        modules: BottomSheetDebugCatalog::modules,
    });
}
