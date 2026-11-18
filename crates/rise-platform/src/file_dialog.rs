use std::future::Future;
use std::path::{Path, PathBuf};

use rise_canon::{RiseUploadFormat, upload_formats};
use rise_core::foundation::foundation_text::{
    trimming_characters, trimming_whitespaces_and_newlines,
};
use thiserror::Error;

use crate::crash_diagnostics::NativeModalGuard;
use crate::gpui_shim::PlatformSupport;
use crate::host_os::HostOs;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum FileDialogError {
    #[error("the file dialog went away before it answered")]
    Abandoned,
    #[error("the system file dialog failed: {0}")]
    System(String),
}

/// catalogue key a screen renders with tr to name a filter next to its picker
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct FilterLabel(&'static str);

impl FilterLabel {
    pub const ALL_FILES: Self = Self("desktop_file_filter_all");
    pub const IMAGES: Self = Self("desktop_file_filter_images");
    pub const VIDEO: Self = Self("desktop_file_filter_video");
    pub const AUDIO: Self = Self("desktop_file_filter_audio");
    pub const MEDIA: Self = Self("desktop_file_filter_media");

    pub const fn key(self) -> &'static str {
        self.0
    }
}

/// class of a canon upload format, read off the top level type of its mime
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MediaKind {
    Image,
    Video,
    Audio,
}

impl MediaKind {
    const fn top_level_type(self) -> &'static str {
        match self {
            Self::Image => "image",
            Self::Video => "video",
            Self::Audio => "audio",
        }
    }

    pub fn holds(self, format: RiseUploadFormat) -> bool {
        format
            .mime
            .split_once('/')
            .is_some_and(|(top_level, _)| top_level == self.top_level_type())
    }
}

/// extensions a request accepts, matched against what the dialog returns
/// an empty list accepts anything and a named filter rejects a file with no extension
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct FileFilter {
    label: FilterLabel,
    extensions: Vec<String>,
}

impl FileFilter {
    /// entries are normalised, .PNG and png are one filter and tar.gz matches whole
    pub fn new<E: AsRef<str>>(label: FilterLabel, extensions: impl IntoIterator<Item = E>) -> Self {
        let mut normalised: Vec<String> = Vec::new();

        for entry in extensions {
            let spaced = trimming_whitespaces_and_newlines(entry.as_ref());
            let trimmed = trimming_characters(spaced, |character| character == '.');
            if trimmed.is_empty() {
                continue;
            }

            let extension = trimmed.to_ascii_lowercase();
            if !normalised.contains(&extension) {
                normalised.push(extension);
            }
        }

        Self {
            label,
            extensions: normalised,
        }
    }

    pub fn any() -> Self {
        Self {
            label: FilterLabel::ALL_FILES,
            extensions: Vec::new(),
        }
    }

    /// every extension the canon upload context accepts, same answer as context_accepts_extension
    /// none for a context canon doesnt declare or one that accepts nothing
    /// an empty filter would accept every file
    pub fn upload_context(label: FilterLabel, context: &str) -> Option<Self> {
        let declared = upload_formats(context)?;
        Self::non_empty(Self::new(label, declared.extensions()))
    }

    pub fn images(context: &str) -> Option<Self> {
        Self::of_kinds(FilterLabel::IMAGES, context, &[MediaKind::Image])
    }

    pub fn video(context: &str) -> Option<Self> {
        Self::of_kinds(FilterLabel::VIDEO, context, &[MediaKind::Video])
    }

    pub fn audio(context: &str) -> Option<Self> {
        Self::of_kinds(FilterLabel::AUDIO, context, &[MediaKind::Audio])
    }

    pub fn media(context: &str) -> Option<Self> {
        Self::of_kinds(
            FilterLabel::MEDIA,
            context,
            &[MediaKind::Image, MediaKind::Video],
        )
    }

    /// formats of the context of the given kinds in canon order
    fn of_kinds(label: FilterLabel, context: &str, kinds: &[MediaKind]) -> Option<Self> {
        let declared = upload_formats(context)?;
        let extensions = declared
            .formats
            .iter()
            .copied()
            .filter(|format| kinds.iter().any(|kind| kind.holds(*format)))
            .map(|format| format.extension);

        Self::non_empty(Self::new(label, extensions))
    }

    fn non_empty(filter: Self) -> Option<Self> {
        if filter.accepts_everything() {
            None
        } else {
            Some(filter)
        }
    }

    pub fn label(&self) -> FilterLabel {
        self.label
    }

    pub fn extensions(&self) -> &[String] {
        &self.extensions
    }

    pub fn accepts_everything(&self) -> bool {
        self.extensions.is_empty()
    }

    /// extension used to complete a save name the user left bare, the first one listed
    pub fn default_extension(&self) -> Option<&str> {
        self.extensions.first().map(String::as_str)
    }

    /// matches the final dot separated suffix, gz takes archive.tar.gz and png refuses photo.png.exe
    /// a leading dot name has no extension and a non utf8 name is refused
    pub fn accepts(&self, path: &Path) -> bool {
        if self.accepts_everything() {
            return true;
        }

        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            return false;
        };
        let name = name.to_ascii_lowercase();

        self.extensions
            .iter()
            .any(|extension| ends_with_extension(&name, extension))
    }

    /// input order is kept in both halves
    pub fn partition(&self, paths: impl IntoIterator<Item = PathBuf>) -> FilteredSelection {
        let mut selection = FilteredSelection::default();

        for path in paths {
            if self.accepts(&path) {
                selection.accepted.push(path);
            } else {
                selection.rejected.push(path);
            }
        }

        selection
    }

    /// appends the default extension to a name the filter rejects
    /// a name that already fits keeps its own spelling and case
    pub fn complete_extension(&self, path: PathBuf) -> PathBuf {
        if self.accepts(&path) {
            return path;
        }

        let Some(extension) = self.default_extension() else {
            return path;
        };

        let completed = path
            .file_name()
            .and_then(|name| name.to_str())
            .map(|name| format!("{name}.{extension}"));

        match completed {
            Some(completed) => path.with_file_name(completed),
            None => path,
        }
    }
}

fn ends_with_extension(name: &str, extension: &str) -> bool {
    name.len() > extension.len() + 1
        && name.ends_with(extension)
        && name.as_bytes()[name.len() - extension.len() - 1] == b'.'
}

#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct FilteredSelection {
    pub accepted: Vec<PathBuf>,
    pub rejected: Vec<PathBuf>,
}

impl FilteredSelection {
    pub fn total(&self) -> usize {
        self.accepted.len() + self.rejected.len()
    }

    pub fn has_rejections(&self) -> bool {
        !self.rejected.is_empty()
    }

    pub fn is_empty(&self) -> bool {
        self.total() == 0
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SelectionTarget {
    Files,
    Directories,
    FilesOrDirectories,
}

impl SelectionTarget {
    pub fn allows_files(self) -> bool {
        matches!(self, Self::Files | Self::FilesOrDirectories)
    }

    pub fn allows_directories(self) -> bool {
        matches!(self, Self::Directories | Self::FilesOrDirectories)
    }

    /// what this target becomes once the host dialog is considered
    /// files or directories drops to files off macos, where asking for directories hides files
    pub fn resolve(self, host: HostOs) -> Self {
        match self {
            Self::FilesOrDirectories if !supports_mixed_selection(host) => Self::Files,
            resolved => resolved,
        }
    }
}

/// true when one dialog can offer files and directories at once, only macos
pub fn supports_mixed_selection(host: HostOs) -> bool {
    match host {
        HostOs::MacOs => true,
        HostOs::Windows | HostOs::Linux => false,
    }
}

pub fn dialog_enforces_filter(host: HostOs) -> PlatformSupport {
    match host {
        HostOs::MacOs | HostOs::Windows | HostOs::Linux => PlatformSupport::Unsupported,
    }
}

/// what to ask the system picker for
/// use the constructors, a directory request has no named filter because that would reject every folder
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct OpenRequest {
    target: SelectionTarget,
    multiple: bool,
    prompt: Option<String>,
    filter: FileFilter,
}

impl OpenRequest {
    pub fn files(filter: FileFilter) -> Self {
        Self {
            target: SelectionTarget::Files,
            multiple: false,
            prompt: None,
            filter,
        }
    }

    pub fn directories() -> Self {
        Self {
            target: SelectionTarget::Directories,
            multiple: false,
            prompt: None,
            filter: FileFilter::any(),
        }
    }

    pub fn files_or_directories(filter: FileFilter) -> Self {
        Self {
            target: SelectionTarget::FilesOrDirectories,
            multiple: false,
            prompt: None,
            filter,
        }
    }

    pub fn allowing_multiple(mut self, multiple: bool) -> Self {
        self.multiple = multiple;
        self
    }

    /// label on the confirm button, not a window title
    pub fn with_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.prompt = Some(prompt.into());
        self
    }

    pub fn target(&self) -> SelectionTarget {
        self.target
    }

    pub fn is_multiple(&self) -> bool {
        self.multiple
    }

    pub fn prompt(&self) -> Option<&str> {
        self.prompt.as_deref()
    }

    pub fn filter(&self) -> &FileFilter {
        &self.filter
    }

    fn prompt_options(&self, host: HostOs) -> gpui::PathPromptOptions {
        let target = self.target.resolve(host);

        gpui::PathPromptOptions {
            files: target.allows_files(),
            directories: target.allows_directories(),
            multiple: self.multiple,
            prompt: self.prompt.clone().map(gpui::SharedString::from),
        }
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct SaveRequest {
    pub directory: PathBuf,
    pub suggested_name: Option<String>,
    pub filter: FileFilter,
}

impl SaveRequest {
    pub fn new(directory: impl Into<PathBuf>, filter: FileFilter) -> Self {
        Self {
            directory: directory.into(),
            suggested_name: None,
            filter,
        }
    }

    pub fn with_suggested_name(mut self, name: impl Into<String>) -> Self {
        self.suggested_name = Some(name.into());
        self
    }
}

/// the three ways an open dialog ends, a failure is not a cancel
#[derive(Debug, PartialEq, Eq)]
pub enum Selection {
    Picked(FilteredSelection),
    Cancelled,
    Failed(FileDialogError),
}

impl Selection {
    /// none and an empty list both mean cancel, backends disagree about which they send
    /// a selection where every path fails the filter is still a pick
    pub fn from_paths(paths: Option<Vec<PathBuf>>, filter: &FileFilter) -> Self {
        match paths {
            None => Self::Cancelled,
            Some(paths) if paths.is_empty() => Self::Cancelled,
            Some(paths) => Self::Picked(filter.partition(paths)),
        }
    }

    pub fn picked(&self) -> Option<&FilteredSelection> {
        match self {
            Self::Picked(selection) => Some(selection),
            Self::Cancelled | Self::Failed(_) => None,
        }
    }

    pub fn is_cancelled(&self) -> bool {
        matches!(self, Self::Cancelled)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum SaveOutcome {
    Chosen(PathBuf),
    Cancelled,
    Failed(FileDialogError),
}

impl SaveOutcome {
    pub fn from_path(path: Option<PathBuf>, filter: &FileFilter) -> Self {
        match path {
            None => Self::Cancelled,
            Some(path) => Self::Chosen(filter.complete_extension(path)),
        }
    }

    pub fn chosen(&self) -> Option<&Path> {
        match self {
            Self::Chosen(path) => Some(path),
            Self::Cancelled | Self::Failed(_) => None,
        }
    }
}

static TESTER_OPEN_ANSWERS: std::sync::Mutex<Option<std::collections::VecDeque<Vec<PathBuf>>>> =
    std::sync::Mutex::new(None);

fn tester_open_answers<T>(f: impl FnOnce(&mut Option<std::collections::VecDeque<Vec<PathBuf>>>) -> T) -> T {
    let mut guard = TESTER_OPEN_ANSWERS.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    f(&mut *guard)
}

pub fn enable_tester_open_answers() -> bool {
    tester_open_answers(|answers| {
        if answers.is_some() {
            return false;
        }
        *answers = Some(std::collections::VecDeque::new());
        true
    })
}

pub fn push_tester_open_answer(paths: Vec<PathBuf>) -> bool {
    tester_open_answers(|answers| match answers {
        Some(queue) => {
            if !queue.is_empty() {
                tracing::warn!(target: "riseonly::X1064", stale = queue.len(), "stealth open panel: unanswered presets replaced");
                queue.clear();
            }
            queue.push_back(paths);
            true
        }
        None => false,
    })
}

/// none means no stealth and the real panel answers, some none means stealth with nothing queued
fn take_tester_open_answer() -> Option<Option<Vec<PathBuf>>> {
    tester_open_answers(|answers| answers.as_mut().map(std::collections::VecDeque::pop_front))
}

/// what a stealth open resolves to, the driver paths through the filter
/// no answer in time is abandoned
fn tester_selection(answer: Option<Vec<PathBuf>>, filter: &FileFilter) -> Selection {
    match answer {
        Some(paths) => Selection::from_paths(Some(paths), filter),
        None => {
            tracing::warn!(target: "riseonly", "stealth open panel: no drive files answer in time, abandoned");
            Selection::Failed(FileDialogError::Abandoned)
        }
    }
}

pub const TESTER_OPEN_WAIT: std::time::Duration = std::time::Duration::from_secs(300);
const TESTER_OPEN_POLL: std::time::Duration = std::time::Duration::from_millis(100);

static TESTER_OPEN_WAITING: std::sync::Mutex<Vec<u64>> = std::sync::Mutex::new(Vec::new());
static TESTER_OPEN_NEXT_TICKET: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn tester_open_waiting<T>(f: impl FnOnce(&mut Vec<u64>) -> T) -> T {
    let mut guard = TESTER_OPEN_WAITING.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    f(&mut *guard)
}

/// true while a stealth open panel waits for the driver files answer
pub fn tester_open_panel_waiting() -> bool {
    tester_open_waiting(|waiting| !waiting.is_empty())
}

/// one stealth open panel on the wait list, tickets grow so the last one is the newest panel
struct TesterOpenWaiting {
    ticket: u64,
}

impl TesterOpenWaiting {
    fn begin() -> Self {
        let ticket = TESTER_OPEN_NEXT_TICKET.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let older = tester_open_waiting(|waiting| {
            let older = waiting.len();
            waiting.push(ticket);
            older
        });
        if older > 0 {
            tracing::warn!(target: "riseonly::X1417", older, "stealth open panel: opened over panels still waiting");
        }
        Self { ticket }
    }

    fn is_newest(&self) -> bool {
        tester_open_waiting(|waiting| waiting.last() == Some(&self.ticket))
    }
}

impl Drop for TesterOpenWaiting {
    fn drop(&mut self) {
        let ticket = self.ticket;
        tester_open_waiting(|waiting| waiting.retain(|waiting_ticket| *waiting_ticket != ticket));
    }
}

async fn wait_for_tester_open_answer(executor: gpui::BackgroundExecutor) -> Option<Vec<PathBuf>> {
    let waiting = TesterOpenWaiting::begin();
    tracing::warn!(target: "riseonly", "stealth open panel: waiting for drive files");
    let deadline = rise_core::foundation::foundation_date::RiseClock::monotonic() + TESTER_OPEN_WAIT;
    loop {
        if waiting.is_newest() {
            if let Some(Some(paths)) = take_tester_open_answer() {
                tracing::warn!(target: "riseonly::X1417", paths = paths.len(), "stealth open panel: answered");
                return Some(paths);
            }
        }
        if rise_core::foundation::foundation_date::RiseClock::monotonic() >= deadline {
            return None;
        }
        executor.timer(TESTER_OPEN_POLL).await;
    }
}

/// a single choice panel takes the first path only, like the real panel lets a user pick one
fn tester_paths_for_request(paths: Vec<PathBuf>, multiple: bool) -> Vec<PathBuf> {
    if multiple { paths } else { paths.into_iter().take(1).collect() }
}

/// opens the picker and resolves once the user is done
/// the dialog is raised before the future is built so the future borrows nothing
pub fn open(app: &gpui::App, request: &OpenRequest) -> impl Future<Output = Selection> + use<> {
    let tester = take_tester_open_answer();
    let modal = tester.is_none().then(NativeModalGuard::enter);
    let receiver = tester
        .is_none()
        .then(|| app.prompt_for_paths(request.prompt_options(HostOs::current())));
    let filter = request.filter.clone();
    let multiple = request.multiple;
    let executor = app.background_executor().clone();

    async move {
        if let Some(answer) = tester {
            let answer = match answer {
                Some(paths) => Some(paths),
                None => wait_for_tester_open_answer(executor).await,
            };
            return tester_selection(answer.map(|paths| tester_paths_for_request(paths, multiple)), &filter);
        }
        let Some(receiver) = receiver else {
            return Selection::Failed(FileDialogError::Abandoned);
        };
        let answer = receiver.await;
        drop(modal);
        match answer {
            Ok(Ok(paths)) => Selection::from_paths(paths, &filter),
            Ok(Err(error)) => Selection::Failed(FileDialogError::System(error.to_string())),
            Err(_) => Selection::Failed(FileDialogError::Abandoned),
        }
    }
}

pub const TESTER_UNNAMED_FILE: &str = "untitled";

static TESTER_SAVE_DIRECTORY: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

pub fn set_tester_save_directory(directory: PathBuf) -> bool {
    TESTER_SAVE_DIRECTORY.set(directory).is_ok()
}

/// opens the save panel and resolves once the user is done
/// the chosen path is completed against the filter, no backend adds an extension itself
pub fn save(app: &gpui::App, request: &SaveRequest) -> impl Future<Output = SaveOutcome> + use<> {
    let suggested = request.suggested_name.as_deref();
    let tester_path = TESTER_SAVE_DIRECTORY
        .get()
        .map(|directory| directory.join(suggested.filter(|name| !name.is_empty()).unwrap_or(TESTER_UNNAMED_FILE)));
    let modal = tester_path.is_none().then(NativeModalGuard::enter);
    let receiver = tester_path.is_none().then(|| app.prompt_for_new_path(&request.directory, suggested));
    let filter = request.filter.clone();

    async move {
        let Some(receiver) = receiver else {
            return SaveOutcome::from_path(tester_path, &filter);
        };
        let answer = receiver.await;
        drop(modal);
        match answer {
            Ok(Ok(path)) => SaveOutcome::from_path(path, &filter),
            Ok(Err(error)) => SaveOutcome::Failed(FileDialogError::System(error.to_string())),
            Err(_) => SaveOutcome::Failed(FileDialogError::Abandoned),
        }
    }
}

#[cfg(test)]
mod tests {
    use rise_canon::{RISE_UPLOAD_CONTEXT_FORMATS, context_accepts_extension};

    use super::*;

    const CHAT: &str = "chat";
    const POST: &str = "post";
    const MUSIC_AUDIO: &str = "music_audio";

    fn paths(names: &[&str]) -> Vec<PathBuf> {
        names.iter().copied().map(PathBuf::from).collect()
    }

    fn named(stem: &str, format: RiseUploadFormat) -> PathBuf {
        PathBuf::from(format!("{stem}.{}", format.extension))
    }

    fn images() -> FileFilter {
        FileFilter::images(CHAT).expect("the chat context takes images")
    }

    fn video() -> FileFilter {
        FileFilter::video(CHAT).expect("the chat context takes video")
    }

    fn audio() -> FileFilter {
        FileFilter::audio(CHAT).expect("the chat context takes audio")
    }

    fn media() -> FileFilter {
        FileFilter::media(CHAT).expect("the chat context takes media")
    }

    #[test]
    fn a_context_filter_answers_exactly_what_the_canon_accepts() {
        for declared in RISE_UPLOAD_CONTEXT_FORMATS.iter() {
            let Some(filter) = FileFilter::upload_context(FilterLabel::ALL_FILES, declared.context)
            else {
                assert!(declared.formats.is_empty(), "{}", declared.context);
                continue;
            };

            for other in RISE_UPLOAD_CONTEXT_FORMATS.iter() {
                for format in other.formats {
                    assert_eq!(
                        filter.accepts(&named("a", *format)),
                        context_accepts_extension(declared.context, format.extension),
                        "{} with .{}",
                        declared.context,
                        format.extension
                    );
                }
            }
        }
    }

    #[test]
    fn a_kind_filter_keeps_only_the_formats_of_that_kind_in_canon_order() {
        let declared = upload_formats(CHAT).expect("chat is a canon context");

        for format in declared.formats {
            let path = named("a", *format);
            assert_eq!(images().accepts(&path), MediaKind::Image.holds(*format));
            assert_eq!(video().accepts(&path), MediaKind::Video.holds(*format));
            assert_eq!(audio().accepts(&path), MediaKind::Audio.holds(*format));
        }

        let first_image = declared
            .formats
            .iter()
            .find(|format| MediaKind::Image.holds(**format))
            .map(|format| format.extension);
        assert_eq!(images().default_extension(), first_image);
    }

    #[test]
    fn an_alias_of_a_context_opens_the_same_filter() {
        assert_eq!(FileFilter::media("chat_message"), FileFilter::media(CHAT));
        assert_eq!(FileFilter::media(" CHAT "), FileFilter::media(CHAT));
    }

    #[test]
    fn an_undeclared_context_never_turns_into_a_filter_that_takes_everything() {
        assert!(FileFilter::upload_context(FilterLabel::MEDIA, "no_such_context").is_none());
        assert!(FileFilter::images("no_such_context").is_none());
    }

    #[test]
    fn a_context_without_a_kind_gives_no_filter_for_that_kind() {
        assert!(FileFilter::images(MUSIC_AUDIO).is_none());
        assert!(FileFilter::audio(POST).is_none());
        assert!(FileFilter::audio(MUSIC_AUDIO).is_some());
    }

    #[test]
    fn each_filter_is_named_by_its_catalogue_key() {
        assert_eq!(FileFilter::any().label(), FilterLabel::ALL_FILES);
        assert_eq!(images().label(), FilterLabel::IMAGES);
        assert_eq!(video().label(), FilterLabel::VIDEO);
        assert_eq!(audio().label(), FilterLabel::AUDIO);
        assert_eq!(media().label(), FilterLabel::MEDIA);
        assert!(
            [
                FilterLabel::ALL_FILES,
                FilterLabel::IMAGES,
                FilterLabel::VIDEO,
                FilterLabel::AUDIO,
                FilterLabel::MEDIA,
            ]
            .iter()
            .all(|label| label.key().starts_with("desktop_file_filter_"))
        );
    }

    #[test]
    fn an_extension_a_camera_wrote_in_capitals_is_still_an_image() {
        let jpeg = RiseUploadFormat::IMAGE_JPEG.extension.to_ascii_uppercase();
        let heic = RiseUploadFormat::IMAGE_HEIC.extension.to_ascii_uppercase();

        assert!(images().accepts(Path::new(&format!("IMG_0001.{jpeg}"))));
        assert!(images().accepts(Path::new(&format!("IMG_0002.{heic}"))));
    }

    #[test]
    fn a_file_with_no_extension_is_refused_by_a_named_filter() {
        assert!(!images().accepts(Path::new("README")));
        assert!(!audio().accepts(Path::new("Makefile")));
    }

    #[test]
    fn a_filter_that_allows_everything_allows_a_file_with_no_extension() {
        let any = FileFilter::any();
        assert!(any.accepts_everything());
        assert!(any.accepts(Path::new("README")));
        assert!(any.accepts(Path::new("photo.jpg")));
        assert!(any.accepts(Path::new("installer.exe")));
    }

    #[test]
    fn a_compound_name_is_matched_by_its_last_extension() {
        let gz = FileFilter::new(FilterLabel::ALL_FILES, ["gz"]);
        assert!(gz.accepts(Path::new("archive.tar.gz")));

        let tar_gz = FileFilter::new(FilterLabel::ALL_FILES, ["tar.gz"]);
        assert!(tar_gz.accepts(Path::new("archive.tar.gz")));
        assert!(
            !tar_gz.accepts(Path::new("archive.gz")),
            "an entry with a dot in it has to match as a whole suffix"
        );
    }

    #[test]
    fn a_second_extension_cannot_smuggle_a_program_past_an_image_filter() {
        let png = RiseUploadFormat::IMAGE_PNG.extension;

        assert!(!images().accepts(Path::new(&format!("photo.{png}.exe"))));
        assert!(!images().accepts(Path::new(&format!("photo.{png}."))));
    }

    #[test]
    fn a_leading_dot_name_counts_as_having_no_extension() {
        let png = RiseUploadFormat::IMAGE_PNG.extension;
        assert!(!images().accepts(Path::new(&format!(".{png}"))));

        let git = FileFilter::new(FilterLabel::ALL_FILES, ["gitignore"]);
        assert!(
            !git.accepts(Path::new(".gitignore")),
            "a dotfile is a stem, which is how Path::extension reads it too"
        );
    }

    #[test]
    fn dots_and_case_in_a_filter_entry_are_normalised_away() {
        let filter = FileFilter::new(FilterLabel::IMAGES, [".PNG", "png", "  .JpG. "]);
        assert_eq!(filter.extensions().len(), 2, "png was written twice");
        assert_eq!(filter.default_extension(), Some("png"));
        assert!(filter.accepts(Path::new("a.png")));
        assert!(filter.accepts(Path::new("b.JPG")));
    }

    #[test]
    fn a_filter_whose_entries_all_normalise_away_allows_everything() {
        let filter = FileFilter::new(FilterLabel::ALL_FILES, ["", "  ", "."]);
        assert!(filter.accepts_everything());
        assert!(filter.default_extension().is_none());
        assert!(
            filter.accepts(Path::new("anything")),
            "a filter nobody can satisfy would lock the user out of the dialog"
        );
    }

    #[test]
    fn mp4_is_the_video_container_and_m4a_the_audio_one() {
        let mp4 = named("a", RiseUploadFormat::VIDEO_MP4);
        let m4a = named("a", RiseUploadFormat::AUDIO_MP4);

        assert!(video().accepts(&mp4));
        assert!(!audio().accepts(&mp4));
        assert!(audio().accepts(&m4a));
        assert!(!images().accepts(&mp4));
        assert!(!video().accepts(&named("a", RiseUploadFormat::IMAGE_JPEG)));
    }

    #[test]
    fn media_is_images_and_video_but_not_every_file() {
        assert!(media().accepts(&named("a", RiseUploadFormat::IMAGE_JPEG)));
        assert!(media().accepts(&named("a", RiseUploadFormat::VIDEO_MP4)));
        assert!(!media().accepts(&named("a", RiseUploadFormat::APPLICATION_PDF)));
        assert!(!media().accepts(&named("a", RiseUploadFormat::AUDIO_MPEG)));
    }

    #[test]
    fn a_mixed_selection_is_split_in_the_order_the_user_chose_it() {
        let png = RiseUploadFormat::IMAGE_PNG.extension;
        let jpeg = RiseUploadFormat::IMAGE_JPEG.extension.to_ascii_uppercase();
        let first = format!("a.{png}");
        let third = format!("c.{jpeg}");
        let chosen = paths(&[first.as_str(), "b.pdf", third.as_str(), "d.txt", "e.zip"]);
        let selection = images().partition(chosen);

        assert_eq!(selection.accepted, paths(&[first.as_str(), third.as_str()]));
        assert_eq!(selection.rejected, paths(&["b.pdf", "d.txt", "e.zip"]));
        assert_eq!(selection.total(), 5);
        assert!(selection.has_rejections());
        assert!(!selection.is_empty());
    }

    #[test]
    fn a_selection_where_nothing_qualifies_is_a_pick_not_a_cancel() {
        let chosen = paths(&["a.pdf", "b.txt"]);
        let selection = Selection::from_paths(Some(chosen), &images());

        let picked = selection.picked().expect("the user did choose files");
        assert!(picked.accepted.is_empty());
        assert_eq!(picked.rejected.len(), 2, "say which two failed");
        assert!(!selection.is_cancelled());
    }

    #[test]
    fn nothing_and_an_empty_list_both_read_as_a_cancel() {
        let filter = FileFilter::any();
        let empty = Selection::from_paths(Some(Vec::new()), &filter);

        assert!(Selection::from_paths(None, &filter).is_cancelled());
        assert!(empty.is_cancelled());
    }

    #[test]
    fn a_failed_dialog_is_never_mistaken_for_a_cancel_or_a_pick() {
        let failed = Selection::Failed(FileDialogError::Abandoned);
        assert!(!failed.is_cancelled(), "a failure needs its own message");
        assert!(failed.picked().is_none());

        let dead = SaveOutcome::Failed(FileDialogError::System("no portal".into()));
        assert!(dead.chosen().is_none());
        assert_ne!(dead, SaveOutcome::Cancelled);
    }

    #[test]
    fn only_macos_can_offer_files_and_directories_in_one_dialog() {
        assert!(supports_mixed_selection(HostOs::MacOs));
        assert!(!supports_mixed_selection(HostOs::Windows));
        assert!(!supports_mixed_selection(HostOs::Linux));
    }

    #[test]
    fn a_mixed_request_drops_to_files_where_the_dialog_cannot_do_both() {
        for host in HostOs::ALL {
            let resolved = SelectionTarget::FilesOrDirectories.resolve(host);
            let expected = if host == HostOs::MacOs {
                SelectionTarget::FilesOrDirectories
            } else {
                SelectionTarget::Files
            };
            assert_eq!(resolved, expected, "on {host:?}");
        }
    }

    #[test]
    fn a_single_purpose_request_is_never_rewritten_for_any_host() {
        let files = SelectionTarget::Files;
        let directories = SelectionTarget::Directories;

        for host in HostOs::ALL {
            assert_eq!(files.resolve(host), files, "on {host:?}");
            assert_eq!(
                directories.resolve(host),
                directories,
                "asking for a folder must still ask for a folder on {host:?}"
            );
        }
    }

    #[test]
    fn asking_for_a_file_never_asks_the_picker_for_a_folder() {
        let request = OpenRequest::files(images());

        for host in HostOs::ALL {
            let options = request.prompt_options(host);

            assert!(options.files);
            assert!(
                !options.directories,
                "that flag becomes FOS_PICKFOLDERS and hides every file on {host:?}"
            );
        }
    }

    #[test]
    fn a_mixed_request_only_asks_for_folders_where_that_does_not_hide_files() {
        let request = OpenRequest::files_or_directories(FileFilter::any());

        for host in HostOs::ALL {
            let options = request.prompt_options(host);

            assert!(options.files);
            assert_eq!(options.directories, host == HostOs::MacOs, "on {host:?}");
        }
    }

    #[test]
    fn a_directory_request_asks_only_for_directories() {
        let request = OpenRequest::directories();
        assert_eq!(request.target(), SelectionTarget::Directories);

        for host in HostOs::ALL {
            let options = request.prompt_options(host);
            assert!(options.directories, "on {host:?}");
            assert!(!options.files, "on {host:?}");
        }
    }

    #[test]
    fn a_directory_request_cannot_carry_a_file_filter() {
        assert!(
            OpenRequest::directories().filter().accepts_everything(),
            "a folder has no extension, so a named filter would reject the pick"
        );
    }

    #[test]
    fn the_multiple_flag_and_the_prompt_label_reach_gpui() {
        let request = OpenRequest::files(media())
            .allowing_multiple(true)
            .with_prompt("Attach");
        let options = request.prompt_options(HostOs::current());
        let label = options.prompt.as_ref().map(gpui::SharedString::as_str);

        assert!(options.multiple);
        assert_eq!(label, Some("Attach"));
        assert!(request.is_multiple());
        assert_eq!(request.prompt(), Some("Attach"));
    }

    #[test]
    fn a_request_selects_one_path_unless_it_was_told_otherwise() {
        let request = OpenRequest::files(FileFilter::any());
        assert!(!request.is_multiple());
        assert!(!request.prompt_options(HostOs::current()).multiple);
        assert!(request.prompt().is_none());
    }

    #[test]
    fn no_backend_narrows_the_dialog_itself() {
        for host in HostOs::ALL {
            assert!(
                !dialog_enforces_filter(host).is_supported(),
                "a screen promising 'images only' has to say so itself on {host:?}"
            );
        }
    }

    #[test]
    fn a_save_name_typed_without_an_extension_gains_the_filters_default() {
        let filter = FileFilter::new(FilterLabel::IMAGES, ["png", "jpg"]);
        let typed = Some(PathBuf::from("/a/holiday"));
        let outcome = SaveOutcome::from_path(typed, &filter);

        assert_eq!(outcome.chosen(), Some(Path::new("/a/holiday.png")));
    }

    #[test]
    fn a_save_name_that_already_matches_keeps_its_own_case() {
        let png = RiseUploadFormat::IMAGE_PNG.extension;
        let upper = format!("/a/HOLIDAY.{}", png.to_ascii_uppercase());
        let lower = format!("/a/holiday.{png}");

        assert_eq!(
            images().complete_extension(PathBuf::from(&upper)),
            PathBuf::from(&upper)
        );
        assert_eq!(
            images().complete_extension(PathBuf::from(&lower)),
            PathBuf::from(&lower)
        );
    }

    #[test]
    fn a_save_name_with_the_wrong_extension_keeps_what_the_user_typed() {
        let filter = images();
        let extension = filter
            .default_extension()
            .expect("an image filter names an extension");
        let completed = filter.complete_extension(PathBuf::from("/a/notes.txt"));

        assert_eq!(
            completed,
            PathBuf::from(format!("/a/notes.txt.{extension}")),
            "renaming notes.txt to an image name would lose which file it was"
        );
    }

    #[test]
    fn a_save_with_no_filter_writes_exactly_what_was_typed() {
        let typed = PathBuf::from("/a/notes");
        let completed = FileFilter::any().complete_extension(typed);

        assert_eq!(completed, PathBuf::from("/a/notes"));
    }

    #[test]
    fn completing_a_name_never_touches_the_directory_it_sits_in() {
        let filter = FileFilter::audio(MUSIC_AUDIO).expect("music_audio takes audio");
        let completed = filter.complete_extension(PathBuf::from("/a/b.c/track"));
        let expected = format!("/a/b.c/track.{}", RiseUploadFormat::AUDIO_MPEG.extension);

        assert_eq!(completed, PathBuf::from(expected));
    }

    #[test]
    fn a_save_dialog_that_was_dismissed_yields_no_path() {
        let outcome = SaveOutcome::from_path(None, &images());

        assert_eq!(outcome, SaveOutcome::Cancelled);
        assert!(outcome.chosen().is_none());
    }

    #[test]
    fn a_save_request_carries_the_name_it_was_given() {
        let bare = SaveRequest::new("/a", images());
        let request = bare.with_suggested_name("holiday");

        assert_eq!(request.directory, PathBuf::from("/a"));
        assert_eq!(request.suggested_name.as_deref(), Some("holiday"));
    }

    #[test]
    fn a_stealth_open_answers_from_the_queue_and_abandons_when_it_is_empty() {
        let filter = media();
        assert!(tester_selection(Some(paths(&["/a/shot.png"])), &filter).picked().is_some());
        assert!(tester_selection(Some(Vec::new()), &filter).is_cancelled());
        assert_eq!(tester_selection(None, &filter), Selection::Failed(FileDialogError::Abandoned));
    }

    #[test]
    fn a_single_choice_stealth_open_takes_the_first_path_and_a_multiple_one_takes_all() {
        let both = paths(&["/a/shot.png", "/a/clip.mp4"]);
        assert_eq!(tester_paths_for_request(both.clone(), false), paths(&["/a/shot.png"]));
        assert_eq!(tester_paths_for_request(both.clone(), true), both);
    }

    #[test]
    fn the_waiting_flag_is_cleared_when_the_wait_ends_and_only_the_newest_panel_answers() {
        {
            let older = TesterOpenWaiting::begin();
            assert!(tester_open_panel_waiting());
            {
                let newer = TesterOpenWaiting::begin();
                assert!(newer.is_newest());
                assert!(!older.is_newest());
            }
            assert!(older.is_newest());
        }
        assert!(!tester_open_panel_waiting());
    }
}
