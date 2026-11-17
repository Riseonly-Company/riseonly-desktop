use std::collections::HashMap;
use std::path::Path;

use zbus::zvariant::{Fd, OwnedObjectPath, Value};

use super::open_document::*;

pub(super) fn quick_look(_path: &Path) -> Result<(), String> {
    Err("Linux has no system previewer".to_owned())
}

pub(super) fn open_default(app: &gpui::App, path: &Path) {
    let target = path.to_path_buf();
    let spawned = std::thread::Builder::new()
        .name("rise-open-document".to_owned())
        .spawn(move || {
            if let Err(reason) = open_through_portal(&target) {
                tracing::debug!(target: "riseonly", %reason, "OpenURI portal unavailable, using xdg-open");
                if let Err(error) = std::process::Command::new(XDG_OPEN).arg(&target).status() {
                    tracing::warn!(target: "riseonly", %error, "xdg-open did not start");
                }
            }
        });
    if spawned.is_err() {
        app.open_with_system(path);
    }
}

fn open_through_portal(path: &Path) -> Result<(), String> {
    let file = std::fs::File::open(path).map_err(|error| error.to_string())?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| error.to_string())?;
    runtime
        .block_on(async {
            let connection = zbus::Connection::session().await?;
            let portal = zbus::Proxy::new(
                &connection,
                PORTAL_BUS_NAME,
                PORTAL_OBJECT_PATH,
                OPEN_URI_INTERFACE,
            )
            .await?;
            let options: HashMap<&str, Value<'_>> = HashMap::new();
            let _request: OwnedObjectPath = portal
                .call(OPEN_FILE_METHOD, &(PARENT_WINDOW_NONE, Fd::from(&file), options))
                .await?;
            Ok::<(), zbus::Error>(())
        })
        .map_err(|error| error.to_string())
}
