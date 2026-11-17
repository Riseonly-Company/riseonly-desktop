use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context as _, bail};
use gpui::{App, AppContext as _, Task};
use rise_deeplink::environment_for_url_scheme;

use super::scheme_desktop_entry::*;
use super::url_scheme::*;
use crate::app_info::identity_for;
use crate::autostart::background_portal::{FLATPAK_INFO_PATH, SNAP_NAME_VARIABLE, Sandbox};
use crate::url_scheme_handler::mime_apps::{
    XdgDisk, XdgFiles as _, XdgSearchPaths, XdgValues, default_scheme_handler,
};

pub(super) struct NativeUrlScheme;

/// ok none means not a scheme of any canon environment
fn entry_for(scheme: &str) -> Result<Option<SchemeEntry>, UrlSchemeUnavailable> {
    let sandbox = Sandbox::detect(
        Path::new(FLATPAK_INFO_PATH).exists(),
        std::env::var_os(SNAP_NAME_VARIABLE).as_deref(),
    );
    if sandbox.needs_portal() {
        return Err(UrlSchemeUnavailable::DeclaredByPackage);
    }
    let current = std::env::current_exe().map_err(|_| UrlSchemeUnavailable::NotInstalled)?;
    let executable = launch_path(std::env::var_os(APPIMAGE_VARIABLE).as_deref(), &current);
    if is_build_tree_executable(&executable) {
        return Err(UrlSchemeUnavailable::NotInstalled);
    }
    Ok(environment_for_url_scheme(scheme)
        .and_then(|environment| identity_for(environment.name))
        .and_then(|identity| SchemeEntry::new(identity, &executable, scheme)))
}

fn user_entry_path(paths: &XdgSearchPaths, entry: &SchemeEntry) -> Option<PathBuf> {
    paths
        .data_home
        .as_ref()
        .map(|dir| dir.join("applications").join(entry.desktop_id()))
}

fn write_entry(path: &Path, text: &str) -> anyhow::Result<()> {
    let directory = path.parent().context("the entry path has no directory")?;
    std::fs::create_dir_all(directory)?;
    let staging = path.with_extension("desktop.tmp");
    std::fs::write(&staging, text)?;
    std::fs::rename(&staging, path)?;
    let refreshed = Command::new("update-desktop-database").arg(directory).status();
    if !refreshed.is_ok_and(|status| status.success()) {
        tracing::debug!(target: "riseonly", "update-desktop-database did not refresh the cache");
    }
    Ok(())
}

fn claim(entry: &SchemeEntry) -> anyhow::Result<()> {
    let paths = XdgSearchPaths::from_values(&XdgValues::from_environment());
    let path = user_entry_path(&paths, entry).context("no XDG data directory")?;
    let user_entry = XdgDisk.read(&path);
    let package_entry = paths
        .data_dirs
        .iter()
        .find_map(|dir| XdgDisk.read(&dir.join("applications").join(entry.desktop_id())));
    if entry.entry_step(user_entry.as_deref(), package_entry.as_deref()) == EntryStep::Write {
        write_entry(&path, &entry.render())?;
    }
    let status = Command::new("xdg-mime")
        .args(["default", entry.desktop_id(), entry.mime_type()])
        .status()
        .context("xdg-mime did not start")?;
    if !status.success() {
        bail!("xdg-mime default answered {status}");
    }
    Ok(())
}

impl UrlSchemeRegistrar for NativeUrlScheme {
    fn is_default_handler(&self, scheme: &str) -> Result<bool, UrlSchemeUnavailable> {
        let Some(entry) = entry_for(scheme)? else {
            return Ok(false);
        };
        let paths = XdgSearchPaths::from_values(&XdgValues::from_environment());
        let default_handler = default_scheme_handler(scheme, &paths, &XdgDisk);
        let user_entry = user_entry_path(&paths, &entry).and_then(|path| XdgDisk.read(&path));
        Ok(entry.claim_holds(default_handler.as_deref(), user_entry.as_deref()))
    }

    fn register(
        &self,
        scheme: &str,
        cx: &App,
    ) -> Result<Task<anyhow::Result<()>>, UrlSchemeUnavailable> {
        let entry = entry_for(scheme)?.ok_or(UrlSchemeUnavailable::NotInstalled)?;
        Ok(cx.background_spawn(async move { claim(&entry) }))
    }
}
