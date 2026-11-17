use std::sync::Arc;

use futures::FutureExt;
use futures::channel::oneshot;
use futures::future::Shared;
use gpui::{AppContext as _, TestAppContext};
use parking_lot::Mutex;

use super::*;

#[derive(Clone, Default)]
struct VideoWarmStartProbeSnapshot {
    started: Vec<String>,
    completed: Vec<String>,
    cancelled: Vec<String>,
}

struct VideoWarmStartProbe {
    state: Mutex<VideoWarmStartProbeSnapshot>,
    release: Mutex<Option<oneshot::Sender<()>>>,
    gate: Shared<oneshot::Receiver<()>>,
}

impl VideoWarmStartProbe {
    fn new() -> Arc<Self> {
        let (release, gate) = oneshot::channel();
        Arc::new(Self {
            state: Mutex::new(VideoWarmStartProbeSnapshot::default()),
            release: Mutex::new(Some(release)),
            gate: gate.shared(),
        })
    }

    fn begin(&self, url: &str) {
        self.state.lock().started.push(url.to_owned());
    }

    fn complete(&self, url: &str) {
        self.state.lock().completed.push(url.to_owned());
    }

    fn cancel(&self, url: &str) {
        self.state.lock().cancelled.push(url.to_owned());
    }

    fn release(&self) {
        if let Some(release) = self.release.lock().take() {
            let _ = release.send(());
        }
    }

    fn snapshot(&self) -> VideoWarmStartProbeSnapshot {
        self.state.lock().clone()
    }
}

struct VideoWarmStartProbeRun {
    probe: Arc<VideoWarmStartProbe>,
    url: String,
    finished: bool,
}

impl Drop for VideoWarmStartProbeRun {
    fn drop(&mut self) {
        if !self.finished {
            self.probe.cancel(&self.url);
        }
    }
}

fn make_coordinator(
    maximum_pending: usize,
    maximum_prepared: usize,
    probe: &Arc<VideoWarmStartProbe>,
) -> Arc<VideoWarmStartCoordinator> {
    let probe = Arc::clone(probe);
    VideoWarmStartCoordinator::new(
        Some(maximum_pending),
        Some(maximum_prepared),
        None,
        None,
        Arc::new(move |request: VideoWarmStartCoordinatorRequest, cx: AsyncApp| {
            let probe = Arc::clone(&probe);
            async move {
                probe.begin(&request.url_string);
                let mut run = VideoWarmStartProbeRun {
                    probe: Arc::clone(&probe),
                    url: request.url_string.clone(),
                    finished: false,
                };
                let _ = probe.gate.clone().await;
                run.finished = true;
                probe.complete(&request.url_string);
                let source = Url::from_string(&request.url_string)?;
                let item = AVPlayerItem::new(source.clone());
                let player_item = item.clone();
                let player = cx.update(|cx| cx.new(|cx| AVQueuePlayer::new(player_item, cx)));
                Some(VideoWarmPlayerBox::new(source, player, item))
            }
            .boxed_local()
        }),
    )
}

fn account() -> (Option<String>, Option<String>) {
    (Some("account-a".to_owned()), Some("epoch-a".to_owned()))
}

#[gpui::test]
async fn replaces_direction_cancels_old_preroll_and_rejects_stale_generation(cx: &mut TestAppContext) {
    let async_cx = cx.to_async();
    let probe = VideoWarmStartProbe::new();
    let coordinator = make_coordinator(2, 1, &probe);
    let old = "https://example.test/old.mp4".to_owned();
    let new = "https://example.test/new.mp4".to_owned();
    let stale = "https://example.test/stale.mp4".to_owned();

    let (user, epoch) = account();
    coordinator
        .replace_directional(std::slice::from_ref(&old), "chat-a", 1, user, epoch, &async_cx)
        .await;
    cx.run_until_parked();
    assert_eq!(probe.snapshot().started, vec![old.clone()]);

    let (user, epoch) = account();
    coordinator
        .replace_directional(std::slice::from_ref(&new), "chat-a", 2, user, epoch, &async_cx)
        .await;
    probe.release();
    cx.run_until_parked();
    assert_eq!(coordinator.prepared_count_for_testing().await, 1);

    let (user, epoch) = account();
    coordinator
        .replace_directional(std::slice::from_ref(&stale), "chat-a", 1, user, epoch, &async_cx)
        .await;
    cx.run_until_parked();

    let snapshot = probe.snapshot();
    assert_eq!(snapshot.cancelled, vec![old]);
    assert_eq!(snapshot.completed, vec![new.clone()]);
    assert!(!snapshot.started.contains(&stale));

    let (user, epoch) = account();
    let prepared = coordinator.take_prepared(&new, user, epoch).await;
    assert!(prepared.is_some());
    assert_eq!(coordinator.prepared_count_for_testing().await, 0);
    coordinator.reset().await;
}
