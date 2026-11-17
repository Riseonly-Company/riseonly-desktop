use reqwest::Error;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_media::map_view::map_http::{MapHttpAnswer, MapHttpFailure, MapHttpReply, MapHttpRequest};
use rise_media::map_view::map_http_hooks::{MapHttpCancel, MapHttpDone, MapHttpHooks, set_map_http_hooks};

use super::network_transport_config::NetworkClientSlot;

static MAP_HTTP_CLIENT: NetworkClientSlot = NetworkClientSlot::standard();

pub fn install_map_http_hooks() {
    set_map_http_hooks(MapHttpHooks { start });
}

fn start(request: MapHttpRequest, done: MapHttpDone) -> MapHttpCancel {
    let transfer = RiseAccountRuntime::current().spawn(async move {
        done(fetch(request).await);
    });
    let abort = transfer.abort_handle();
    Box::new(move || abort.abort())
}

async fn fetch(request: MapHttpRequest) -> MapHttpAnswer {
    let Some(client) = MAP_HTTP_CLIENT.current() else {
        return Err(MapHttpFailure::Connection("the HTTP client could not be built".to_owned()));
    };
    let mut builder = client.get(&request.url);
    for (name, value) in &request.headers {
        builder = builder.header(*name, value);
    }
    let response = builder.send().await.map_err(failure)?;
    let status = response.status().as_u16();
    let headers = response
        .headers()
        .iter()
        .filter_map(|(name, value)| Some((name.as_str().to_owned(), value.to_str().ok()?.to_owned())))
        .collect();
    let body = response.bytes().await.map_err(failure)?.to_vec();
    Ok(MapHttpReply { status, headers, body })
}

fn failure(error: Error) -> MapHttpFailure {
    if error.is_connect() || error.is_timeout() {
        MapHttpFailure::Connection(error.to_string())
    } else {
        MapHttpFailure::Other(error.to_string())
    }
}
