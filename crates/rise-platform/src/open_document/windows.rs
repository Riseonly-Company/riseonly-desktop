use std::path::Path;

pub(super) fn quick_look(_path: &Path) -> Result<(), String> {
    Err("Windows has no system previewer".to_owned())
}

pub(super) fn open_default(app: &gpui::App, path: &Path) {
    app.open_with_system(path);
}
