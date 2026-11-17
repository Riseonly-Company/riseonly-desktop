use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

use futures::StreamExt;
use futures::channel::mpsc;
use gpui::{App, Context, Entity, Task};
use rise_platform::host_os::HostOs;
use rise_platform::paths::AppPaths;
use rise_platform::updater::{
    self, ApplyPolicy, AvailableUpdate, BuildProfile, FeedProvenance, InstallMethod, InstallPlan,
    InstallSite, ReleaseChannel, Target, UpdateDecision, UpdateManifest, Version,
};

use crate::core::config::app_environment::AppEnvironment;
use crate::core::engine_bridge::network_transport_config::NetworkClientSlot;

static FEED_CLIENT: NetworkClientSlot = NetworkClientSlot::standard();

const FEED_BASE: &str = "https://cdn.riseonly.net/desktop";
const FIRST_CHECK: Duration = Duration::from_secs(8);
const RECHECK: Duration = Duration::from_secs(6 * 60 * 60);
const RESTART_PAINT: Duration = Duration::from_millis(400);
const FEED_LIMIT: usize = 1 << 20;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum AppUpdatePhase {
    Idle,
    Available,
    Downloading(f32),
    Restarting,
    Failed,
}

#[derive(Clone)]
struct AppUpdateOffer {
    update: AvailableUpdate,
    count: usize,
    site: InstallSite,
    policy: ApplyPolicy,
}

struct FetchedFeed {
    body: Vec<u8>,
    signature: Option<String>,
}

pub struct AppUpdateStore {
    phase: AppUpdatePhase,
    offer: Option<AppUpdateOffer>,
    started: bool,
    check: Option<Task<()>>,
    progress: Option<Task<()>>,
    install: Option<Task<()>>,
}

impl AppUpdateStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            phase: AppUpdatePhase::Idle,
            offer: None,
            started: false,
            check: None,
            progress: None,
            install: None,
        }
    }

    pub fn phase(&self) -> AppUpdatePhase {
        self.phase
    }

    pub fn count(&self) -> usize {
        self.offer.as_ref().map_or(0, |offer| offer.count)
    }

    pub fn start(&mut self, cx: &mut Context<Self>) {
        if self.started {
            return;
        }
        self.started = true;
        let Some(feed) = feed_url() else {
            return;
        };
        if let Some(runtime) = io_runtime() {
            runtime.spawn_blocking(clear_leftovers);
        }
        self.check = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(FIRST_CHECK).await;
            loop {
                let fetched = match io_runtime() {
                    Some(runtime) => runtime
                        .spawn(fetch_feed(feed.clone()))
                        .await
                        .unwrap_or_else(|error| Err(error.to_string())),
                    None => Err("no io runtime".to_owned()),
                };
                if this
                    .update(cx, |store, cx| store.take_feed(fetched, cx))
                    .is_err()
                {
                    break;
                }
                cx.background_executor().timer(RECHECK).await;
            }
        }));
    }

    fn take_feed(&mut self, fetched: Result<FetchedFeed, String>, cx: &mut Context<Self>) {
        if matches!(
            self.phase,
            AppUpdatePhase::Downloading(_) | AppUpdatePhase::Restarting
        ) {
            return;
        }
        let feed = match fetched {
            Ok(feed) => feed,
            Err(error) => {
                tracing::warn!(target: "riseonly", "update feed not read: {error}");
                return;
            }
        };
        self.offer = offer_from(&feed, cx);
        self.phase = if self.offer.is_some() {
            AppUpdatePhase::Available
        } else {
            AppUpdatePhase::Idle
        };
        cx.notify();
    }

    pub fn install(&mut self, cx: &mut Context<Self>) {
        if !matches!(
            self.phase,
            AppUpdatePhase::Available | AppUpdatePhase::Failed
        ) {
            return;
        }
        let (Some(offer), Some(runtime)) = (self.offer.clone(), io_runtime()) else {
            return;
        };
        self.phase = AppUpdatePhase::Downloading(0.0);
        cx.notify();

        let (sender, mut receiver) = mpsc::unbounded::<f32>();
        let job = runtime.spawn(download_and_install(offer, sender));
        self.progress = Some(cx.spawn(async move |this, cx| {
            while let Some(fraction) = receiver.next().await {
                if this
                    .update(cx, |store, cx| store.set_progress(fraction, cx))
                    .is_err()
                {
                    break;
                }
            }
        }));
        self.install = Some(cx.spawn(async move |this, cx| {
            let result = job.await.unwrap_or_else(|error| Err(error.to_string()));
            let _ = this.update(cx, |store, cx| store.finish(result, cx));
        }));
    }

    fn set_progress(&mut self, fraction: f32, cx: &mut Context<Self>) {
        let AppUpdatePhase::Downloading(shown) = self.phase else {
            return;
        };
        let fraction = fraction.clamp(0.0, 1.0);
        if (fraction * 100.0).floor() > (shown * 100.0).floor() {
            self.phase = AppUpdatePhase::Downloading(fraction);
            cx.notify();
        }
    }

    fn finish(&mut self, result: Result<PathBuf, String>, cx: &mut Context<Self>) {
        self.progress = None;
        match result {
            Ok(relaunch) => {
                self.phase = AppUpdatePhase::Restarting;
                cx.notify();
                self.install = Some(cx.spawn(async move |_, cx| {
                    cx.background_executor().timer(RESTART_PAINT).await;
                    let _ = cx.update(|cx| {
                        cx.set_restart_path(relaunch);
                        cx.restart();
                    });
                }));
            }
            Err(error) => {
                tracing::error!(target: "riseonly", "update not installed: {error}");
                self.phase = AppUpdatePhase::Failed;
                cx.notify();
            }
        }
    }
}

fn feed_url() -> Option<String> {
    if let Some(url) = std::env::var("RISE_UPDATE_FEED_URL")
        .ok()
        .filter(|url| !url.trim().is_empty())
    {
        return Some(url);
    }
    if let Some(url) = option_env!("RISE_UPDATE_FEED_URL").filter(|url| !url.trim().is_empty()) {
        return Some(url.to_owned());
    }
    let environment = AppEnvironment::compiled().as_str();
    matches!(environment, "staging" | "production")
        .then(|| format!("{FEED_BASE}/{environment}/feed.json"))
}

fn public_key() -> Option<&'static str> {
    option_env!("RISE_UPDATE_FEED_PUBLIC_KEY").filter(|key| !key.trim().is_empty())
}

fn staging_dir() -> Option<PathBuf> {
    AppPaths::resolve(AppEnvironment::compiled().canon())
        .ok()
        .map(|paths| paths.cache_dir().join("updates"))
}

fn io_runtime() -> Option<&'static tokio::runtime::Runtime> {
    static RUNTIME: OnceLock<Option<tokio::runtime::Runtime>> = OnceLock::new();
    RUNTIME
        .get_or_init(|| {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(1)
                .thread_name("rise-app-update")
                .enable_all()
                .build()
                .inspect_err(
                    |error| tracing::error!(target: "riseonly", "no update runtime: {error}"),
                )
                .ok()
        })
        .as_ref()
}

fn clear_leftovers() {
    if let Some(staging) = staging_dir() {
        let _ = std::fs::remove_dir_all(staging);
    }
    if HostOs::current() == HostOs::Windows
        && let Some(dir) = std::env::current_exe()
            .ok()
            .as_deref()
            .and_then(Path::parent)
    {
        updater::remove_leftovers(dir);
    }
}

fn offer_from(feed: &FetchedFeed, cx: &App) -> Option<AppUpdateOffer> {
    let manifest = UpdateManifest::parse(&String::from_utf8_lossy(&feed.body))
        .inspect_err(|error| tracing::warn!(target: "riseonly", "update feed refused: {error}"))
        .ok()?;
    let provenance = match (feed.signature.as_deref(), public_key()) {
        (Some(signature), Some(key))
            if updater::feed_signature_valid(&feed.body, signature, key) =>
        {
            FeedProvenance::Signed
        }
        _ => FeedProvenance::Unsigned,
    };
    let profile = if cfg!(debug_assertions) {
        BuildProfile::Development
    } else {
        BuildProfile::Production
    };
    let policy = ApplyPolicy {
        profile,
        provenance,
        enabled: true,
    };
    let permission = updater::apply_permission(policy);
    if !permission.is_allowed() {
        tracing::warn!(target: "riseonly", "update feed not used: {permission}");
        return None;
    }

    let installed = Version::of_this_build().ok()?;
    let target = Target::current()?;
    let UpdateDecision::Available(update) =
        updater::decide(&installed, ReleaseChannel::Stable, target, &manifest)
    else {
        return None;
    };
    let site = updater::current_site(cx, staging_dir()?).ok()?;
    if !installable(&site, &update) {
        return None;
    }
    let count = updater::newer_count(&installed, ReleaseChannel::Stable, target, &manifest).max(1);
    Some(AppUpdateOffer {
        update,
        count,
        site,
        policy,
    })
}

fn installable(site: &InstallSite, update: &AvailableUpdate) -> bool {
    let installed = site.installed.to_string_lossy();
    match updater::plan_install(site, update).method {
        InstallMethod::SwapBundle => {
            site.installed
                .extension()
                .is_some_and(|extension| extension == "app")
                && !installed.contains("/AppTranslocation/")
        }
        InstallMethod::ReplaceFile
        | InstallMethod::HelperProcess { .. }
        | InstallMethod::HelperUnavailable => true,
        InstallMethod::ReplaceTree | InstallMethod::PackageManager => false,
    }
}

async fn fetch_feed(url: String) -> Result<FetchedFeed, String> {
    let client = FEED_CLIENT
        .current()
        .ok_or_else(|| "no http client".to_owned())?;
    let body = fetch_small(&client, &url).await?;
    let signature = fetch_small(&client, &format!("{url}.sig"))
        .await
        .ok()
        .and_then(|bytes| String::from_utf8(bytes).ok());
    Ok(FetchedFeed { body, signature })
}

async fn fetch_small(client: &reqwest::Client, url: &str) -> Result<Vec<u8>, String> {
    let response = client
        .get(url)
        .header(reqwest::header::CACHE_CONTROL, "no-cache")
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|error| error.to_string())?;
    let bytes = response.bytes().await.map_err(|error| error.to_string())?;
    if bytes.len() > FEED_LIMIT {
        return Err(format!("{url} is larger than a feed"));
    }
    Ok(bytes.to_vec())
}

async fn download_and_install(
    offer: AppUpdateOffer,
    progress: mpsc::UnboundedSender<f32>,
) -> Result<PathBuf, String> {
    let client = FEED_CLIENT
        .current()
        .ok_or_else(|| "no http client".to_owned())?;
    let expected = offer.update.artifact.size;
    let mut response = client
        .get(&offer.update.artifact.url)
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|error| error.to_string())?;
    let mut bytes = Vec::with_capacity(usize::try_from(expected).unwrap_or(0));
    while let Some(chunk) = response.chunk().await.map_err(|error| error.to_string())? {
        bytes.extend_from_slice(&chunk);
        if bytes.len() as u64 > expected {
            return Err("the download is larger than the feed says".to_owned());
        }
        let _ = progress.unbounded_send(bytes.len() as f32 / expected.max(1) as f32);
    }
    drop(progress);
    tokio::task::spawn_blocking(move || install_blocking(offer, bytes))
        .await
        .map_err(|error| error.to_string())?
}

fn install_blocking(offer: AppUpdateOffer, bytes: Vec<u8>) -> Result<PathBuf, String> {
    let plan = updater::prepare(&offer.site, offer.policy, &offer.update, &bytes)
        .map_err(|error| error.to_string())?;
    std::fs::create_dir_all(&offer.site.staging).map_err(|error| error.to_string())?;
    std::fs::write(&plan.staged, &bytes).map_err(|error| error.to_string())?;
    drop(bytes);
    let unpacked = updater::unpack(
        offer.site.host,
        &plan.staged,
        &offer.site.staging.join("unpacked"),
    )
    .map_err(|error| error.to_string())?;

    if offer.site.host == HostOs::Windows {
        let running = std::env::current_exe().map_err(|error| error.to_string())?;
        let install_dir = running
            .parent()
            .ok_or_else(|| "no install folder".to_owned())?
            .to_path_buf();
        updater::replace_running_tree(&unpacked, &install_dir, &running)
            .map_err(|error| error.to_string())?;
        return Ok(install_dir.join("riseonly.exe"));
    }

    let plan = InstallPlan {
        staged: unpacked,
        ..plan
    };
    updater::install(&plan).map_err(|error| error.to_string())?;
    Ok(plan.installed)
}
