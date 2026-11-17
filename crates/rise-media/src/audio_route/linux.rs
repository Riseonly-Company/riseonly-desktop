use std::io::{BufRead, BufReader, ErrorKind};
use std::process::{Child, ChildStdout, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::{self, JoinHandle};
use rise_core::foundation::foundation_date::RiseClock;

use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_platform::HostOs;

use super::audio_route::*;
use super::pulse_route::{default_sink, is_route_event, next_attempt, restart_delay, sink_names};

pub(super) struct NativeRouteSource;

const PACTL: &str = "pactl";

/// the running subscriber, shared by the reader thread and Drop
struct Subscription {
    stopped: bool,
    child: Option<Child>,
}

struct PulseRoutes {
    subscription: Arc<Mutex<Subscription>>,
    stop: Sender<()>,
    reader: Option<JoinHandle<()>>,
}

impl AudioRouteSource for NativeRouteSource {
    fn open(&self, signal: RouteSignal) -> Result<Box<dyn AudioRouteBackend>, AudioRouteError> {
        let (child, stdout) = spawn_subscriber()?;
        let subscription = Arc::new(Mutex::new(Subscription { stopped: false, child: Some(child) }));
        let (stop, stopped) = mpsc::channel::<()>();
        let shared = Arc::clone(&subscription);
        let reader = thread::Builder::new()
            .name("rise-audio-route-pactl".to_owned())
            .spawn(move || run_reader(shared, stdout, signal, stopped));
        match reader {
            Ok(reader) => Ok(Box::new(PulseRoutes { subscription, stop, reader: Some(reader) })),
            Err(error) => {
                if let Some(mut child) = lock(&subscription).child.take() {
                    let _ = child.kill();
                    let _ = child.wait();
                }
                Err(AudioRouteError::Failed { call: "thread spawn", detail: error.to_string() })
            }
        }
    }
}

impl AudioRouteBackend for PulseRoutes {
    fn snapshot(&mut self) -> Result<AudioOutputSnapshot, AudioRouteError> {
        let info = run_pactl(&["info"])?;
        let sinks = run_pactl(&["list", "short", "sinks"])?;
        Ok(AudioOutputSnapshot { default_output: default_sink(&info), outputs: sink_names(&sinks) })
    }
}

impl Drop for PulseRoutes {
    fn drop(&mut self) {
        {
            let mut subscription = lock(&self.subscription);
            subscription.stopped = true;
            if let Some(child) = subscription.child.as_mut() {
                let _ = child.kill();
            }
        }
        let _ = self.stop.send(());
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

fn run_reader(
    subscription: Arc<Mutex<Subscription>>,
    first: ChildStdout,
    signal: RouteSignal,
    stopped: Receiver<()>,
) {
    let mut stdout = Some(first);
    let mut attempt = 0u32;
    loop {
        if let Some(out) = stdout.take() {
            let started = RiseClock::monotonic();
            read_events(out, &signal);
            let child = lock(&subscription).child.take();
            if let Some(mut child) = child {
                let _ = child.wait();
            }
            attempt = next_attempt(attempt, RiseClock::monotonic().saturating_sub(started));
        }
        if lock(&subscription).stopped {
            return;
        }
        match stopped.recv_timeout(restart_delay(attempt)) {
            Err(RecvTimeoutError::Timeout) => {}
            Ok(()) | Err(RecvTimeoutError::Disconnected) => return,
        }
        if run_pactl(&["info"]).is_err() {
            attempt = attempt.saturating_add(1);
            continue;
        }
        let Ok((child, out)) = spawn_subscriber() else {
            attempt = attempt.saturating_add(1);
            continue;
        };
        {
            let mut guard = lock(&subscription);
            if guard.stopped {
                let mut child = child;
                let _ = child.kill();
                let _ = child.wait();
                return;
            }
            guard.child = Some(child);
        }
        signal();
        stdout = Some(out);
    }
}

fn read_events(stdout: ChildStdout, signal: &RouteSignal) {
    for line in BufReader::new(stdout).lines() {
        let Ok(line) = line else { break };
        if is_route_event(&line) {
            signal();
        }
    }
}

fn spawn_subscriber() -> Result<(Child, ChildStdout), AudioRouteError> {
    let mut child = pactl()
        .arg("subscribe")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| spawn_error("pactl subscribe", &error))?;
    match child.stdout.take() {
        Some(stdout) => Ok((child, stdout)),
        None => {
            let _ = child.kill();
            let _ = child.wait();
            Err(AudioRouteError::Failed { call: "pactl subscribe", detail: "no output pipe".to_owned() })
        }
    }
}

fn lock(subscription: &Mutex<Subscription>) -> std::sync::MutexGuard<'_, Subscription> {
    subscription.lock().unwrap_or_else(PoisonError::into_inner)
}

fn pactl() -> Command {
    let mut command = Command::new(PACTL);
    command.env("LC_ALL", "C");
    command
}

fn spawn_error(call: &'static str, error: &std::io::Error) -> AudioRouteError {
    if error.kind() == ErrorKind::NotFound {
        AudioRouteError::Unavailable {
            host: HostOs::Linux,
            reason: "pactl is not installed (no PulseAudio or pipewire-pulse client)".to_owned(),
        }
    } else {
        AudioRouteError::Failed { call, detail: error.to_string() }
    }
}

fn run_pactl(arguments: &[&str]) -> Result<String, AudioRouteError> {
    let output = pactl()
        .args(arguments)
        .stdin(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .map_err(|error| spawn_error("pactl", &error))?;
    if !output.status.success() {
        return Err(AudioRouteError::Failed {
            call: "pactl",
            detail: format!(
                "{} exited with {}: {}",
                arguments.join(" "),
                output.status,
                trimming_whitespaces_and_newlines(&String::from_utf8_lossy(&output.stderr))
            ),
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}
