use std::ffi::OsStr;
use std::path::{Component, Path, PathBuf};

use crate::app_info::AppIdentity;
use crate::autostart::{desktop_file_name, escape_desktop_value, exec_value};
use crate::url_scheme_handler::mime_apps::{entry_is_hidden, group_values, scheme_mime_type};

pub const APPIMAGE_VARIABLE: &str = "APPIMAGE";

const DESKTOP_ENTRY: &str = "Desktop Entry";
const GENERATED_KEY: &str = "X-Riseonly-Generated";

/// what a launch does about the entry file before asking for the default
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EntryStep {
    Keep,
    Write,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct SchemeEntry {
    desktop_id: String,
    display_name: String,
    bundle_identifier: String,
    executable: PathBuf,
    mime_type: String,
}

impl SchemeEntry {
    pub fn new(identity: &AppIdentity, executable: &Path, scheme: &str) -> Option<Self> {
        Some(Self {
            desktop_id: desktop_file_name(identity.bundle_identifier),
            display_name: identity.display_name.to_owned(),
            bundle_identifier: identity.bundle_identifier.to_owned(),
            executable: executable.to_owned(),
            mime_type: scheme_mime_type(scheme)?,
        })
    }

    /// file name, also the id mimeapps.list and the notification hint use
    pub fn desktop_id(&self) -> &str {
        &self.desktop_id
    }

    pub fn mime_type(&self) -> &str {
        &self.mime_type
    }

    pub fn render(&self) -> String {
        format!(
            "[{DESKTOP_ENTRY}]\n\
             Type=Application\n\
             Name={name}\n\
             Exec={exec} %u\n\
             Icon={id}\n\
             Terminal=false\n\
             NoDisplay=true\n\
             MimeType={mime};\n\
             StartupWMClass={id}\n\
             {GENERATED_KEY}=true\n",
            name = escape_desktop_value(&self.display_name),
            exec = exec_value(&self.executable, &[]),
            id = self.bundle_identifier,
            mime = self.mime_type,
        )
    }

    /// an app owned entry is renewed when the binary moved
    /// a user written one stays, and a package entry that declares the type isnt shadowed by a user copy
    pub fn entry_step(&self, user_entry: Option<&str>, package_entry: Option<&str>) -> EntryStep {
        match user_entry {
            Some(text) if is_generated(text) && text != self.render() => EntryStep::Write,
            Some(_) => EntryStep::Keep,
            None if package_entry.is_some_and(|text| self.declares_type(text)) => EntryStep::Keep,
            None => EntryStep::Write,
        }
    }

    /// true when the default is this entry and it still starts this binary
    pub fn claim_holds(&self, default_handler: Option<&str>, user_entry: Option<&str>) -> bool {
        default_handler == Some(self.desktop_id.as_str())
            && !user_entry.is_some_and(|text| is_generated(text) && text != self.render())
    }

    fn declares_type(&self, text: &str) -> bool {
        !entry_is_hidden(text)
            && group_values(text, DESKTOP_ENTRY, "MimeType").contains(&self.mime_type.as_str())
    }
}

fn is_generated(text: &str) -> bool {
    group_values(text, DESKTOP_ENTRY, GENERATED_KEY).contains(&"true")
}

/// file a link starts, the appimage image or else the running binary
pub fn launch_path(appimage: Option<&OsStr>, current_exe: &Path) -> PathBuf {
    appimage
        .map(Path::new)
        .filter(|path| path.is_absolute())
        .unwrap_or(current_exe)
        .to_owned()
}

/// binary under a cargo target dir next to debug or release, maybe past a target triple
/// replaced every build so no link should point at it
pub fn is_build_tree_executable(path: &Path) -> bool {
    let names: Vec<&OsStr> = path
        .components()
        .filter_map(|component| match component {
            Component::Normal(name) => Some(name),
            _ => None,
        })
        .collect();
    names.iter().enumerate().any(|(index, name)| {
        name.to_str().is_some_and(|name| name.starts_with("target"))
            && names
                .iter()
                .skip(index + 1)
                .take(2)
                .any(|below| matches!(below.to_str(), Some("debug" | "release")))
    })
}
