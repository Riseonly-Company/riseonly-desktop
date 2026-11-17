use std::path::{Path, PathBuf};

use gpui::{ClipboardItem, Image};
use rise_platform::host_os::HostOs;
use rise_platform::pasteboard::{file_paths_in, has_strings_in, image_in};

pub enum ChatComposerPaste {
    Files(Vec<PathBuf>),
    Image(Image),
    Text,
}

impl ChatComposerPaste {
    /// files win over text, a file manager puts names beside the files and those arent what was copied
    /// an image goes only without text, text beside an image means a document was copied
    pub fn of(item: &ClipboardItem, host: HostOs, is_file: impl Fn(&Path) -> bool) -> Self {
        let paths = file_paths_in(item, host);
        if !paths.is_empty() && paths.iter().all(|path| is_file(path)) {
            return Self::Files(paths);
        }
        if has_strings_in(item) {
            return Self::Text;
        }
        if let Some(image) = image_in(item) {
            return Self::Image(image);
        }
        if paths.is_empty() { Self::Text } else { Self::Files(paths) }
    }
}
