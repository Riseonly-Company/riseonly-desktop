use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::gpui_shim::PlatformSupport;
use crate::host_os::HostOs;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum VersionError {
    #[error("a version with no digits in it")]
    Empty,
    #[error("{0} is not a numeric version")]
    NotNumeric(String),
    #[error("{0} has more than three components")]
    TooManyComponents(String),
    #[error("{0} has a component too large to hold")]
    OutOfRange(String),
}

/// three numbers ordered by the first that differs
/// field order is the ordering, reorder them and release precedence flips
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct Version {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl Version {
    pub const fn new(major: u32, minor: u32, patch: u32) -> Self {
        Self {
            major,
            minor,
            patch,
        }
    }

    /// takes one to three numeric parts and a leading v, missing part is zero
    /// pre release suffix like 1.2.3-beta is refused not truncated
    pub fn parse(raw: &str) -> Result<Self, VersionError> {
        let trimmed = raw.trim();
        let text = trimmed.strip_prefix(['v', 'V']).unwrap_or(trimmed);

        if text.is_empty() {
            return Err(VersionError::Empty);
        }

        let mut components = [0_u32; 3];

        for (index, part) in text.split('.').enumerate() {
            if index == components.len() {
                return Err(VersionError::TooManyComponents(raw.to_owned()));
            }
            components[index] = parse_component(part, raw)?;
        }

        Ok(Self::new(components[0], components[1], components[2]))
    }

    pub fn of_this_build() -> Result<Self, VersionError> {
        Self::parse(env!("CARGO_PKG_VERSION"))
    }
}

fn parse_component(part: &str, raw: &str) -> Result<u32, VersionError> {
    if part.is_empty() || !part.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(VersionError::NotNumeric(raw.to_owned()));
    }

    part.parse::<u32>()
        .map_err(|_| VersionError::OutOfRange(raw.to_owned()))
}

impl fmt::Display for Version {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

impl std::str::FromStr for Version {
    type Err = VersionError;

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        Self::parse(raw)
    }
}

impl Serialize for Version {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Version {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Self::parse(&raw).map_err(serde::de::Error::custom)
    }
}

/// which builds an install wants to receive, default is stable
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum ReleaseChannel {
    #[default]
    Stable,
    Beta,
}

impl ReleaseChannel {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Stable => "stable",
            Self::Beta => "beta",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "stable" | "release" => Some(Self::Stable),
            "beta" | "preview" => Some(Self::Beta),
            _ => None,
        }
    }

    /// beta takes both and picks the newer, stable takes only stable
    pub const fn accepts(self, offered: Self) -> bool {
        !matches!((self, offered), (Self::Stable, Self::Beta))
    }
}

impl fmt::Display for ReleaseChannel {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// channel exactly as the feed spelled it, unknown values are kept not rejected
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ChannelTag(String);

impl ChannelTag {
    pub fn new(raw: impl Into<String>) -> Self {
        Self(raw.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn known(&self) -> Option<ReleaseChannel> {
        ReleaseChannel::parse(&self.0)
    }
}

impl From<ReleaseChannel> for ChannelTag {
    fn from(channel: ReleaseChannel) -> Self {
        Self(channel.as_str().to_owned())
    }
}

/// cpu an artifact was built for, the feed has one artifact per arch
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Arch {
    X86_64,
    Aarch64,
}

impl Arch {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::X86_64 => "x86_64",
            Self::Aarch64 => "aarch64",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "x86_64" | "x64" | "amd64" => Some(Self::X86_64),
            "aarch64" | "arm64" => Some(Self::Aarch64),
            _ => None,
        }
    }

    /// none on a cpu with no published build
    pub const fn current() -> Option<Self> {
        #[cfg(target_arch = "x86_64")]
        {
            Some(Self::X86_64)
        }
        #[cfg(target_arch = "aarch64")]
        {
            Some(Self::Aarch64)
        }
        #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
        {
            None
        }
    }
}

/// the key one artifact is published under
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Target {
    pub host: HostOs,
    pub arch: Arch,
}

impl Target {
    pub const fn new(host: HostOs, arch: Arch) -> Self {
        Self { host, arch }
    }

    pub fn key(&self) -> String {
        format!("{}-{}", host_key(self.host), self.arch.as_str())
    }

    pub fn parse(raw: &str) -> Option<Self> {
        let (host, arch) = raw.split_once('-')?;
        let host = match host.trim().to_ascii_lowercase().as_str() {
            "macos" | "darwin" => HostOs::MacOs,
            "windows" | "win" => HostOs::Windows,
            "linux" => HostOs::Linux,
            _ => return None,
        };
        Some(Self::new(host, Arch::parse(arch)?))
    }

    pub fn current() -> Option<Self> {
        Some(Self::new(HostOs::current(), Arch::current()?))
    }
}

const fn host_key(host: HostOs) -> &'static str {
    match host {
        HostOs::MacOs => "macos",
        HostOs::Windows => "windows",
        HostOs::Linux => "linux",
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ManifestError {
    #[error("the feed is not a manifest: {0}")]
    Malformed(String),
    #[error("{version} publishes {target} with no usable sha256")]
    Unverifiable { version: Version, target: String },
}

/// one downloadable file
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Artifact {
    pub url: String,
    pub size: u64,
    pub sha256: String,
}

/// one published build in every shape it was published in
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Release {
    pub version: Version,
    pub channel: ChannelTag,
    #[serde(default)]
    pub notes_url: Option<String>,
    #[serde(default)]
    pub artifacts: BTreeMap<String, Artifact>,
}

#[derive(Clone, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub struct UpdateManifest {
    #[serde(default)]
    pub releases: Vec<Release>,
}

impl UpdateManifest {
    /// unknown fields and channels are fine
    /// a bad sha256 fails the whole document
    pub fn parse(json: &str) -> Result<Self, ManifestError> {
        let document: serde_json::Value = serde_json::from_str(json)
            .map_err(|error| ManifestError::Malformed(error.to_string()))?;

        if !document.is_object() {
            return Err(ManifestError::Malformed(
                "a manifest is a JSON object".to_owned(),
            ));
        }

        let manifest: Self = serde_json::from_value(document)
            .map_err(|error| ManifestError::Malformed(error.to_string()))?;

        for release in &manifest.releases {
            for (target, artifact) in &release.artifacts {
                if !is_sha256_hex(&artifact.sha256) {
                    return Err(ManifestError::Unverifiable {
                        version: release.version,
                        target: target.clone(),
                    });
                }
            }
        }

        Ok(manifest)
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| "{\"releases\":[]}".to_owned())
    }
}

fn is_sha256_hex(raw: &str) -> bool {
    raw.len() == 64 && raw.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// a build this install may take with all it needs to fetch and check it
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct AvailableUpdate {
    pub version: Version,
    pub channel: ReleaseChannel,
    pub target: String,
    pub artifact: Artifact,
    pub notes_url: Option<String>,
}

/// why there is nothing to install
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum UpToDateReason {
    NothingNewer,
    NewerBuildOnAnotherChannel {
        version: Version,
        channel: ReleaseChannel,
    },
    AheadOfFeed {
        newest: Version,
    },
    FeedEmptyForChannel,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum UnsupportedReason {
    UnknownArchitecture,
    NoArtifactForTarget { target: String, version: Version },
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum UpdateDecision {
    UpToDate(UpToDateReason),
    Available(AvailableUpdate),
    Unsupported(UnsupportedReason),
}

impl UpdateDecision {
    pub const fn available(&self) -> Option<&AvailableUpdate> {
        match self {
            Self::Available(update) => Some(update),
            _ => None,
        }
    }

    pub const fn is_up_to_date(&self) -> bool {
        matches!(self, Self::UpToDate(_))
    }
}

/// picks the best release for target
/// only a release strictly newer than installed is offered
pub fn decide(
    installed: &Version,
    channel: ReleaseChannel,
    target: Target,
    manifest: &UpdateManifest,
) -> UpdateDecision {
    let key = target.key();

    let mut best: Option<(&Release, &Artifact, ReleaseChannel)> = None;
    let mut newest_acceptable: Option<Version> = None;
    let mut newest_without_artifact: Option<Version> = None;
    let mut newest_elsewhere: Option<(Version, ReleaseChannel)> = None;

    for release in &manifest.releases {
        let Some(offered) = release.channel.known() else {
            continue;
        };

        if !channel.accepts(offered) {
            if release.version > *installed
                && newest_elsewhere.is_none_or(|(version, _)| release.version > version)
            {
                newest_elsewhere = Some((release.version, offered));
            }
            continue;
        }

        if newest_acceptable.is_none_or(|version| release.version > version) {
            newest_acceptable = Some(release.version);
        }

        if release.version <= *installed {
            continue;
        }

        match release.artifacts.get(&key) {
            Some(artifact) => {
                let wins = best.is_none_or(|(incumbent, _, incumbent_channel)| {
                    beats(
                        (release.version, offered),
                        (incumbent.version, incumbent_channel),
                    )
                });
                if wins {
                    best = Some((release, artifact, offered));
                }
            }
            None => {
                if newest_without_artifact.is_none_or(|version| release.version > version) {
                    newest_without_artifact = Some(release.version);
                }
            }
        }
    }

    if let Some((release, artifact, offered)) = best {
        return UpdateDecision::Available(AvailableUpdate {
            version: release.version,
            channel: offered,
            target: key,
            artifact: artifact.clone(),
            notes_url: release.notes_url.clone(),
        });
    }

    if let Some(version) = newest_without_artifact {
        return UpdateDecision::Unsupported(UnsupportedReason::NoArtifactForTarget {
            target: key,
            version,
        });
    }

    if let Some((version, offered)) = newest_elsewhere {
        return UpdateDecision::UpToDate(UpToDateReason::NewerBuildOnAnotherChannel {
            version,
            channel: offered,
        });
    }

    match newest_acceptable {
        Some(newest) if newest < *installed => {
            UpdateDecision::UpToDate(UpToDateReason::AheadOfFeed { newest })
        }
        Some(_) => UpdateDecision::UpToDate(UpToDateReason::NothingNewer),
        None => UpdateDecision::UpToDate(UpToDateReason::FeedEmptyForChannel),
    }
}

pub fn decide_for_this_machine(
    installed: &Version,
    channel: ReleaseChannel,
    manifest: &UpdateManifest,
) -> UpdateDecision {
    match Target::current() {
        Some(target) => decide(installed, channel, target, manifest),
        None => UpdateDecision::Unsupported(UnsupportedReason::UnknownArchitecture),
    }
}

fn beats(candidate: (Version, ReleaseChannel), incumbent: (Version, ReleaseChannel)) -> bool {
    match candidate.0.cmp(&incumbent.0) {
        Ordering::Greater => true,
        Ordering::Less => false,
        Ordering::Equal => {
            candidate.1 == ReleaseChannel::Stable && incumbent.1 == ReleaseChannel::Beta
        }
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum IntegrityError {
    #[error("expected {expected} bytes, got {actual}")]
    Size { expected: u64, actual: u64 },
    #[error("the download does not match the checksum the manifest declared")]
    Checksum,
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";

    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();

    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        out.push(HEX[usize::from(byte >> 4)] as char);
        out.push(HEX[usize::from(byte & 0x0f)] as char);
    }
    out
}

/// checks the declared size before hashing so a truncated download fails without a full hash
pub fn verify(bytes: &[u8], artifact: &Artifact) -> Result<(), IntegrityError> {
    let actual = bytes.len() as u64;
    if actual != artifact.size {
        return Err(IntegrityError::Size {
            expected: artifact.size,
            actual,
        });
    }

    if digests_match(&sha256_hex(bytes), &artifact.sha256.to_ascii_lowercase()) {
        Ok(())
    } else {
        Err(IntegrityError::Checksum)
    }
}

fn digests_match(left: &str, right: &str) -> bool {
    let (left, right) = (left.as_bytes(), right.as_bytes());
    if left.len() != right.len() {
        return false;
    }

    let mut difference = 0_u8;
    for (a, b) in left.iter().zip(right) {
        difference |= a ^ b;
    }
    difference == 0
}

/// whether this build might run on a stranger machine, default is the strict one
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum BuildProfile {
    Development,
    #[default]
    Production,
}

/// what is known about where the feed came from, nothing makes Signed yet
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum FeedProvenance {
    #[default]
    Unsigned,
    Signed,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct ApplyPolicy {
    pub profile: BuildProfile,
    pub provenance: FeedProvenance,
    pub enabled: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ApplyPermission {
    Allowed,
    Disabled,
    UnsignedFeedInProduction,
}

impl ApplyPermission {
    pub const fn is_allowed(self) -> bool {
        matches!(self, Self::Allowed)
    }
}

impl fmt::Display for ApplyPermission {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Allowed => "allowed",
            Self::Disabled => "applying updates has not been turned on",
            Self::UnsignedFeedInProduction => {
                "a production build will not install from an unsigned feed"
            }
        })
    }
}

/// an unsigned feed in a production build is refused whatever the enabled flag says
pub const fn apply_permission(policy: ApplyPolicy) -> ApplyPermission {
    match (policy.profile, policy.provenance) {
        (BuildProfile::Production, FeedProvenance::Unsigned) => {
            ApplyPermission::UnsignedFeedInProduction
        }
        _ => {
            if policy.enabled {
                ApplyPermission::Allowed
            } else {
                ApplyPermission::Disabled
            }
        }
    }
}

/// how this copy was put on disk, the linux plan goes by this and not by the os
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum InstallShape {
    SelfContained,
    AppImage,
    PackageManaged,
}

const PACKAGE_OWNED_PREFIXES: [&str; 6] = ["/usr/", "/bin/", "/sbin/", "/opt/", "/snap/", "/app/"];

/// everything a plan needs to know about this install
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct InstallSite {
    pub host: HostOs,
    pub shape: InstallShape,
    pub installed: PathBuf,
    pub staging: PathBuf,
    pub helper: Option<PathBuf>,
}

impl InstallSite {
    /// app_path is the bundle on macos and the executable elsewhere, appimage is $APPIMAGE
    /// inside an appimage only $APPIMAGE names a file that outlives the run
    pub fn resolve(
        host: HostOs,
        app_path: &Path,
        appimage: Option<&Path>,
        staging: PathBuf,
        helper: Option<PathBuf>,
    ) -> Self {
        let (shape, installed) = match host {
            HostOs::MacOs | HostOs::Windows => {
                (InstallShape::SelfContained, app_path.to_path_buf())
            }
            HostOs::Linux => match appimage.filter(|path| !path.as_os_str().is_empty()) {
                Some(path) => (InstallShape::AppImage, path.to_path_buf()),
                None if is_package_owned(app_path) => {
                    (InstallShape::PackageManaged, app_path.to_path_buf())
                }
                None => (InstallShape::SelfContained, app_path.to_path_buf()),
            },
        };

        Self {
            host,
            shape,
            installed,
            staging,
            helper,
        }
    }
}

fn is_package_owned(path: &Path) -> bool {
    let text = path.to_string_lossy();
    PACKAGE_OWNED_PREFIXES
        .iter()
        .any(|prefix| text.starts_with(prefix))
}

pub const WINDOWS_HELPER_EXECUTABLE: &str = "riseonly-updater.exe";

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum InstallMethod {
    SwapBundle,
    HelperProcess { helper: PathBuf },
    HelperUnavailable,
    ReplaceFile,
    ReplaceTree,
    PackageManager,
}

impl InstallMethod {
    /// true when this process does the swap and so the relaunch too
    pub const fn replaces_in_place(&self) -> bool {
        matches!(
            self,
            Self::SwapBundle | Self::ReplaceFile | Self::ReplaceTree
        )
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct InstallPlan {
    pub method: InstallMethod,
    pub staged: PathBuf,
    pub installed: PathBuf,
    pub displaced: PathBuf,
}

pub fn plan_install(site: &InstallSite, update: &AvailableUpdate) -> InstallPlan {
    let method = match site.shape {
        InstallShape::PackageManaged => InstallMethod::PackageManager,
        InstallShape::AppImage => InstallMethod::ReplaceFile,
        InstallShape::SelfContained => match site.host {
            HostOs::MacOs => InstallMethod::SwapBundle,
            HostOs::Windows => match site.helper.clone() {
                Some(helper) => InstallMethod::HelperProcess { helper },
                None => InstallMethod::HelperUnavailable,
            },
            HostOs::Linux => InstallMethod::ReplaceTree,
        },
    };

    InstallPlan {
        method,
        staged: site
            .staging
            .join(staged_file_name(&update.artifact.url, &update.version)),
        installed: site.installed.clone(),
        displaced: displaced_path(&site.installed, &site.staging),
    }
}

/// staged file name taken from the url but never trusted, anything odd falls back
pub fn staged_file_name(url: &str, version: &Version) -> String {
    let without_query = url.split(['?', '#']).next().unwrap_or(url);
    let last = without_query.rsplit('/').next().unwrap_or("");

    if is_plain_file_name(last) {
        last.to_owned()
    } else {
        format!("riseonly-{version}.download")
    }
}

fn is_plain_file_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && !name.starts_with('.')
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_' | b'+'))
}

fn displaced_path(installed: &Path, staging: &Path) -> PathBuf {
    match installed.file_name().and_then(|name| name.to_str()) {
        Some(name) => installed.with_file_name(format!(".{name}.previous")),
        None => staging.join("previous"),
    }
}

/// what the windows helper is started with, an argument vector and never a command line
pub fn helper_arguments(plan: &InstallPlan, pid: u32) -> Vec<String> {
    vec![
        "--wait-for-pid".to_owned(),
        pid.to_string(),
        "--source".to_owned(),
        plan.staged.to_string_lossy().into_owned(),
        "--target".to_owned(),
        plan.installed.to_string_lossy().into_owned(),
        "--relaunch".to_owned(),
        plan.installed.to_string_lossy().into_owned(),
    ]
}

/// the helper next to executable or None, a bare name would let PATH resolve it
pub fn helper_beside_executable(executable: &Path, helper_name: &str) -> Option<PathBuf> {
    let directory = executable.parent()?;
    if directory.as_os_str().is_empty() {
        return None;
    }

    Some(directory.join(helper_name))
}

#[derive(Debug, Error)]
pub enum UpdateError {
    #[error("not applying: {0}")]
    Refused(ApplyPermission),
    #[error(transparent)]
    Integrity(#[from] IntegrityError),
    #[error("{WINDOWS_HELPER_EXECUTABLE} is not installed beside the application")]
    HelperMissing,
    #[error("could not locate this installation: {0}")]
    Unlocatable(String),
    #[error("{}: {reason}", .path.display())]
    Io { path: PathBuf, reason: String },
}

/// permission first then integrity then the plan
/// no plan is made for unchecked bytes
pub fn prepare(
    site: &InstallSite,
    policy: ApplyPolicy,
    update: &AvailableUpdate,
    bytes: &[u8],
) -> Result<InstallPlan, UpdateError> {
    let permission = apply_permission(policy);
    if !permission.is_allowed() {
        return Err(UpdateError::Refused(permission));
    }

    verify(bytes, &update.artifact)?;
    Ok(plan_install(site, update))
}

pub fn install(plan: &InstallPlan) -> Result<PlatformSupport, UpdateError> {
    match &plan.method {
        InstallMethod::SwapBundle | InstallMethod::ReplaceFile | InstallMethod::ReplaceTree => {
            swap_in_place(&plan.staged, &plan.installed, &plan.displaced)?;
            Ok(PlatformSupport::Performed)
        }
        InstallMethod::HelperProcess { helper } => {
            std::process::Command::new(helper)
                .args(helper_arguments(plan, std::process::id()))
                .spawn()
                .map_err(|error| UpdateError::Io {
                    path: helper.clone(),
                    reason: error.to_string(),
                })?;
            Ok(PlatformSupport::Performed)
        }
        InstallMethod::HelperUnavailable => Err(UpdateError::HelperMissing),
        InstallMethod::PackageManager => Ok(PlatformSupport::Unsupported),
    }
}

fn swap_in_place(staged: &Path, installed: &Path, displaced: &Path) -> Result<(), UpdateError> {
    let failure = |path: &Path, error: &std::io::Error| UpdateError::Io {
        path: path.to_path_buf(),
        reason: error.to_string(),
    };

    remove_any(displaced).map_err(|error| failure(displaced, &error))?;

    let had_previous = std::fs::symlink_metadata(installed).is_ok();
    if had_previous {
        std::fs::rename(installed, displaced).map_err(|error| failure(installed, &error))?;
    }

    match move_into_place(staged, installed) {
        Ok(()) => {
            let _ = remove_any(displaced);
            Ok(())
        }
        Err(error) => {
            if had_previous {
                let _ = std::fs::rename(displaced, installed);
            }
            Err(failure(staged, &error))
        }
    }
}

fn move_into_place(staged: &Path, installed: &Path) -> std::io::Result<()> {
    match std::fs::rename(staged, installed) {
        Err(error) if error.kind() == std::io::ErrorKind::CrossesDevices => {
            let name = installed.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
            let incoming = installed.with_file_name(format!(".{name}.incoming"));
            remove_any(&incoming)?;
            let copied = if std::fs::symlink_metadata(staged)?.is_dir() {
                copy_tree(staged, &incoming)
            } else {
                std::fs::copy(staged, &incoming).map(|_| ())
            };
            if let Err(error) = copied.and_then(|()| std::fs::rename(&incoming, installed)) {
                let _ = remove_any(&incoming);
                return Err(error);
            }
            let _ = remove_any(staged);
            Ok(())
        }
        other => other,
    }
}

fn remove_any(path: &Path) -> std::io::Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() => std::fs::remove_dir_all(path),
        Ok(_) => std::fs::remove_file(path),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

pub fn current_site(app: &gpui::App, staging: PathBuf) -> Result<InstallSite, UpdateError> {
    let installed = app
        .app_path()
        .map_err(|error| UpdateError::Unlocatable(error.to_string()))?;

    let appimage = std::env::var_os("APPIMAGE").map(PathBuf::from);

    let helper = app
        .path_for_auxiliary_executable(WINDOWS_HELPER_EXECUTABLE)
        .ok()
        .or_else(|| helper_beside_executable(&installed, WINDOWS_HELPER_EXECUTABLE))
        .filter(|path| path.is_file());

    Ok(InstallSite::resolve(
        HostOs::current(),
        &installed,
        appimage.as_deref(),
        staging,
        helper,
    ))
}

pub fn relaunch(app: &mut gpui::App, plan: &InstallPlan) -> PlatformSupport {
    if !plan.method.replaces_in_place() {
        return PlatformSupport::Unsupported;
    }

    app.restart();
    PlatformSupport::Performed
}

pub fn feed_signature_valid(feed: &[u8], signature: &str, public_key: &str) -> bool {
    use base64::Engine as _;

    let engine = base64::engine::general_purpose::STANDARD;
    let (Ok(signature), Ok(key)) = (engine.decode(signature.trim()), engine.decode(public_key.trim())) else {
        return false;
    };
    key.len() == 32
        && ring::signature::UnparsedPublicKey::new(&ring::signature::ED25519, key)
            .verify(feed, &signature)
            .is_ok()
}

pub fn newer_count(installed: &Version, channel: ReleaseChannel, target: Target, manifest: &UpdateManifest) -> usize {
    let key = target.key();
    let mut versions: Vec<Version> = manifest
        .releases
        .iter()
        .filter(|release| release.version > *installed)
        .filter(|release| release.channel.known().is_some_and(|offered| channel.accepts(offered)))
        .filter(|release| release.artifacts.contains_key(&key))
        .map(|release| release.version)
        .collect();
    versions.sort();
    versions.dedup();
    versions.len()
}

pub fn unpack(host: HostOs, archive: &Path, dest: &Path) -> Result<PathBuf, UpdateError> {
    let io = |path: &Path, reason: String| UpdateError::Io { path: path.to_path_buf(), reason };

    remove_any(dest).map_err(|error| io(dest, error.to_string()))?;
    std::fs::create_dir_all(dest).map_err(|error| io(dest, error.to_string()))?;

    let run = |program: &str, args: &[&std::ffi::OsStr]| -> Result<(), UpdateError> {
        let status = std::process::Command::new(program)
            .args(args)
            .status()
            .map_err(|error| io(archive, format!("{program}: {error}")))?;
        if status.success() {
            Ok(())
        } else {
            Err(io(archive, format!("{program} exited with {status}")))
        }
    };

    match host {
        HostOs::MacOs => {
            run("/usr/bin/ditto", &["-x".as_ref(), "-k".as_ref(), archive.as_os_str(), dest.as_os_str()])?;
            first_entry(dest, |path| path.extension().is_some_and(|extension| extension == "app"))
                .ok_or_else(|| io(archive, "no app bundle in the archive".to_owned()))
        }
        HostOs::Windows => {
            run("tar", &["-xf".as_ref(), archive.as_os_str(), "-C".as_ref(), dest.as_os_str()])?;
            first_entry(dest, |path| path.join("riseonly.exe").is_file())
                .ok_or_else(|| io(archive, "no riseonly.exe in the archive".to_owned()))
        }
        HostOs::Linux => {
            let file_name = archive.file_name().ok_or_else(|| io(archive, "no file name".to_owned()))?;
            let staged = dest.join(file_name);
            std::fs::rename(archive, &staged).map_err(|error| io(archive, error.to_string()))?;
            run("chmod", &["755".as_ref(), staged.as_os_str()])?;
            Ok(staged)
        }
    }
}

fn first_entry(dir: &Path, wanted: impl Fn(&Path) -> bool) -> Option<PathBuf> {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir).ok()?.filter_map(|entry| entry.ok().map(|entry| entry.path())).collect();
    entries.sort();
    entries.into_iter().find(|path| wanted(path))
}

pub fn replace_running_tree(new_root: &Path, install_dir: &Path, running: &Path) -> Result<(), UpdateError> {
    let io = |path: &Path, error: std::io::Error| UpdateError::Io { path: path.to_path_buf(), reason: error.to_string() };
    let aside = running.with_extension("exe.old");

    remove_any(&aside).map_err(|error| io(&aside, error))?;
    std::fs::rename(running, &aside).map_err(|error| io(running, error))?;
    if let Err(error) = copy_tree(new_root, install_dir) {
        if !running.exists() {
            let _ = std::fs::rename(&aside, running);
        }
        return Err(io(install_dir, error));
    }
    Ok(())
}

fn copy_tree(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

pub fn remove_leftovers(install_dir: &Path) {
    let Ok(entries) = std::fs::read_dir(install_dir) else {
        return;
    };
    for entry in entries.flatten() {
        if entry.file_name().to_string_lossy().ends_with(".exe.old") {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EMPTY_DIGEST: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

    fn version(raw: &str) -> Version {
        Version::parse(raw).unwrap()
    }

    fn artifact_for(url: &str, bytes: &[u8]) -> Artifact {
        Artifact {
            url: url.to_owned(),
            size: bytes.len() as u64,
            sha256: sha256_hex(bytes),
        }
    }

    fn release(raw_version: &str, channel: &str, targets: &[&str]) -> Release {
        Release {
            version: version(raw_version),
            channel: ChannelTag::new(channel),
            notes_url: None,
            artifacts: targets
                .iter()
                .map(|target| {
                    (
                        (*target).to_owned(),
                        artifact_for(
                            &format!("https://downloads.riseonly.net/{raw_version}/Riseonly.zip"),
                            raw_version.as_bytes(),
                        ),
                    )
                })
                .collect(),
        }
    }

    fn feed(releases: Vec<Release>) -> UpdateManifest {
        UpdateManifest { releases }
    }

    const MAC: Target = Target::new(HostOs::MacOs, Arch::Aarch64);

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("rise-updater-{name}"));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn update_named(raw_version: &str, url: &str) -> AvailableUpdate {
        AvailableUpdate {
            version: version(raw_version),
            channel: ReleaseChannel::Stable,
            target: MAC.key(),
            artifact: artifact_for(url, raw_version.as_bytes()),
            notes_url: None,
        }
    }

    #[test]
    fn ten_sorts_after_nine_rather_than_before_it() {
        assert!(
            version("1.10.0") > version("1.9.0"),
            "comparing versions as text is how a release strands everyone on 1.9"
        );
        assert!(version("2.0.0") > version("1.99.99"));
        assert!(version("1.2.10") > version("1.2.9"));
    }

    #[test]
    fn a_missing_component_is_zero_and_is_not_compared_as_text() {
        assert_eq!(version("1.2"), version("1.2.0"));
        assert_eq!(version("1"), version("1.0.0"));
        assert!(
            version("1.2.1") > version("1.2"),
            "1.2 in a feed means 1.2.0"
        );
    }

    #[test]
    fn a_leading_v_from_a_release_tag_is_accepted() {
        assert_eq!(version("v1.4.2"), Version::new(1, 4, 2));
        assert_eq!(version(" V1.4.2 "), Version::new(1, 4, 2));
    }

    #[test]
    fn a_version_round_trips_through_its_text() {
        for raw in ["0.0.0", "1.4.2", "10.0.1", "4294967295.0.0"] {
            assert_eq!(version(raw).to_string(), raw);
        }
    }

    #[test]
    fn malformed_versions_are_refused_rather_than_guessed() {
        assert_eq!(Version::parse(""), Err(VersionError::Empty));
        assert_eq!(Version::parse("v"), Err(VersionError::Empty));
        assert!(matches!(
            Version::parse("1.2.3.4"),
            Err(VersionError::TooManyComponents(_))
        ));
        for raw in ["1..2", "1.2.x", "-1.2.3", "1.2.3 beta", "latest"] {
            assert!(
                matches!(Version::parse(raw), Err(VersionError::NotNumeric(_))),
                "{raw} should not parse"
            );
        }
        assert!(matches!(
            Version::parse("4294967296.0.0"),
            Err(VersionError::OutOfRange(_))
        ));
    }

    #[test]
    fn a_prerelease_suffix_is_refused_rather_than_silently_dropped() {
        assert!(
            Version::parse("1.2.3-beta.1").is_err(),
            "dropping the suffix would make the beta compare equal to the release"
        );
    }

    #[test]
    fn the_compiled_in_version_of_this_build_parses() {
        assert!(Version::of_this_build().is_ok());
    }

    #[test]
    fn an_unset_channel_is_stable_not_beta() {
        assert_eq!(ReleaseChannel::default(), ReleaseChannel::Stable);
    }

    #[test]
    fn a_stable_install_is_never_offered_a_beta_build() {
        assert!(!ReleaseChannel::Stable.accepts(ReleaseChannel::Beta));
        assert!(ReleaseChannel::Stable.accepts(ReleaseChannel::Stable));

        let manifest = feed(vec![release("2.0.0", "beta", &["macos-aarch64"])]);
        let decision = decide(&version("1.0.0"), ReleaseChannel::Stable, MAC, &manifest);

        assert_eq!(decision.available(), None);
        assert_eq!(
            decision,
            UpdateDecision::UpToDate(UpToDateReason::NewerBuildOnAnotherChannel {
                version: version("2.0.0"),
                channel: ReleaseChannel::Beta,
            })
        );
    }

    #[test]
    fn a_beta_install_takes_whichever_channel_is_newer() {
        let stable_ahead = feed(vec![
            release("2.1.0", "stable", &["macos-aarch64"]),
            release("2.0.0", "beta", &["macos-aarch64"]),
        ]);
        let taken = decide(&version("1.0.0"), ReleaseChannel::Beta, MAC, &stable_ahead)
            .available()
            .unwrap()
            .clone();
        assert_eq!(taken.version, version("2.1.0"));
        assert_eq!(taken.channel, ReleaseChannel::Stable);

        let beta_ahead = feed(vec![
            release("2.1.0", "stable", &["macos-aarch64"]),
            release("2.2.0", "beta", &["macos-aarch64"]),
        ]);
        let taken = decide(&version("1.0.0"), ReleaseChannel::Beta, MAC, &beta_ahead)
            .available()
            .unwrap()
            .clone();
        assert_eq!(taken.version, version("2.2.0"));
        assert_eq!(taken.channel, ReleaseChannel::Beta);
    }

    #[test]
    fn the_same_version_on_both_channels_resolves_to_the_promoted_build() {
        let manifest = feed(vec![
            release("2.0.0", "beta", &["macos-aarch64"]),
            release("2.0.0", "stable", &["macos-aarch64"]),
        ]);

        for releases in [manifest.releases.clone(), {
            let mut reversed = manifest.releases.clone();
            reversed.reverse();
            reversed
        }] {
            let decision = decide(
                &version("1.0.0"),
                ReleaseChannel::Beta,
                MAC,
                &feed(releases),
            );
            assert_eq!(
                decision.available().unwrap().channel,
                ReleaseChannel::Stable
            );
        }
    }

    #[test]
    fn a_channel_the_feed_invents_does_not_stop_this_client_updating() {
        let manifest = feed(vec![
            release("3.0.0", "nightly", &["macos-aarch64"]),
            release("2.0.0", "stable", &["macos-aarch64"]),
        ]);

        let taken = decide(&version("1.0.0"), ReleaseChannel::Stable, MAC, &manifest)
            .available()
            .unwrap()
            .clone();

        assert_eq!(
            taken.version,
            version("2.0.0"),
            "an unknown channel is not for us, but it must not hide the one that is"
        );
    }

    #[test]
    fn every_target_key_round_trips() {
        for host in HostOs::ALL {
            for arch in [Arch::X86_64, Arch::Aarch64] {
                let target = Target::new(host, arch);
                assert_eq!(Target::parse(&target.key()), Some(target));
            }
        }
    }

    #[test]
    fn two_targets_never_share_a_key() {
        let mut keys: Vec<String> = HostOs::ALL
            .iter()
            .flat_map(|host| {
                [Arch::X86_64, Arch::Aarch64]
                    .into_iter()
                    .map(move |arch| Target::new(*host, arch).key())
            })
            .collect();
        let total = keys.len();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), total);
    }

    #[test]
    fn this_build_knows_which_artifact_it_would_need() {
        assert!(
            Target::current().is_some(),
            "the test suite only runs on architectures we publish for"
        );
    }

    #[test]
    fn a_release_with_no_checksum_is_refused_outright() {
        let omitted = r#"{"releases":[{"version":"1.4.0","channel":"stable","artifacts":{"macos-aarch64":{"url":"https://x/y","size":10}}}]}"#;
        assert!(matches!(
            UpdateManifest::parse(omitted),
            Err(ManifestError::Malformed(_))
        ));

        for bad in ["", "deadbeef", &EMPTY_DIGEST[1..]] {
            let json = format!(
                r#"{{"releases":[{{"version":"1.4.0","channel":"stable","artifacts":{{"macos-aarch64":{{"url":"https://x/y","size":10,"sha256":"{bad}"}}}}}}]}}"#
            );
            assert_eq!(
                UpdateManifest::parse(&json),
                Err(ManifestError::Unverifiable {
                    version: version("1.4.0"),
                    target: "macos-aarch64".to_owned(),
                }),
                "an artifact nobody can verify must never reach a decision"
            );
        }
    }

    #[test]
    fn an_artifact_for_an_unknown_platform_is_nothing_for_us_not_an_error() {
        let json = format!(
            r#"{{"releases":[{{"version":"1.4.0","channel":"stable","artifacts":{{"plan9-mips":{{"url":"https://x/y","size":10,"sha256":"{EMPTY_DIGEST}"}}}}}}]}}"#
        );

        let manifest = UpdateManifest::parse(&json).unwrap();
        let decision = decide(&version("1.0.0"), ReleaseChannel::Stable, MAC, &manifest);

        assert_eq!(
            decision,
            UpdateDecision::Unsupported(UnsupportedReason::NoArtifactForTarget {
                target: "macos-aarch64".to_owned(),
                version: version("1.4.0"),
            })
        );
    }

    #[test]
    fn a_release_with_no_artifacts_at_all_still_parses() {
        let manifest =
            UpdateManifest::parse(r#"{"releases":[{"version":"1.4.0","channel":"stable"}]}"#)
                .unwrap();
        assert!(manifest.releases[0].artifacts.is_empty());
    }

    #[test]
    fn a_field_this_build_has_never_heard_of_does_not_break_the_feed() {
        let json = format!(
            r#"{{"generated_at":"tomorrow","releases":[{{"version":"1.4.0","channel":"stable","severity":"critical","artifacts":{{"macos-aarch64":{{"url":"https://x/y","size":0,"sha256":"{EMPTY_DIGEST}","signature":"…"}}}}}}]}}"#
        );
        assert!(UpdateManifest::parse(&json).is_ok());
    }

    #[test]
    fn an_empty_document_is_an_empty_feed_rather_than_a_failure() {
        assert_eq!(
            UpdateManifest::parse("{}").unwrap(),
            UpdateManifest::default()
        );
    }

    #[test]
    fn something_that_is_not_a_manifest_is_a_typed_error() {
        for raw in ["", "not json", "[]", r#"{"releases":{}}"#] {
            assert!(matches!(
                UpdateManifest::parse(raw),
                Err(ManifestError::Malformed(_))
            ));
        }
    }

    #[test]
    fn a_manifest_survives_a_round_trip_through_json() {
        let manifest = feed(vec![
            release("1.4.0", "stable", &["macos-aarch64", "windows-x86_64"]),
            release("1.5.0", "beta", &["linux-x86_64"]),
        ]);
        assert_eq!(
            UpdateManifest::parse(&manifest.to_json()).unwrap(),
            manifest
        );
    }

    #[test]
    fn a_newer_build_is_offered_with_the_checksum_it_will_be_held_to() {
        let manifest = feed(vec![release("1.4.0", "stable", &["macos-aarch64"])]);
        let update = decide(&version("1.3.9"), ReleaseChannel::Stable, MAC, &manifest)
            .available()
            .unwrap()
            .clone();

        assert_eq!(update.version, version("1.4.0"));
        assert_eq!(update.target, "macos-aarch64");
        assert!(is_sha256_hex(&update.artifact.sha256));
    }

    #[test]
    fn an_identical_version_is_not_an_update() {
        let manifest = feed(vec![release("1.4.0", "stable", &["macos-aarch64"])]);
        assert_eq!(
            decide(&version("1.4.0"), ReleaseChannel::Stable, MAC, &manifest),
            UpdateDecision::UpToDate(UpToDateReason::NothingNewer)
        );
    }

    #[test]
    fn a_build_newer_than_the_feed_is_never_told_to_downgrade() {
        let manifest = feed(vec![
            release("1.4.0", "stable", &["macos-aarch64"]),
            release("1.3.0", "stable", &["macos-aarch64"]),
        ]);
        let decision = decide(&version("2.0.0"), ReleaseChannel::Stable, MAC, &manifest);

        assert_eq!(decision.available(), None, "that would be a downgrade");
        assert_eq!(
            decision,
            UpdateDecision::UpToDate(UpToDateReason::AheadOfFeed {
                newest: version("1.4.0"),
            })
        );
    }

    #[test]
    fn an_empty_feed_is_up_to_date_rather_than_broken() {
        let decision = decide(
            &version("1.0.0"),
            ReleaseChannel::Stable,
            MAC,
            &UpdateManifest::default(),
        );
        assert_eq!(
            decision,
            UpdateDecision::UpToDate(UpToDateReason::FeedEmptyForChannel)
        );
        assert!(decision.is_up_to_date());
    }

    #[test]
    fn the_newest_acceptable_release_wins_whatever_order_the_feed_is_in() {
        let versions = ["1.4.0", "1.10.0", "1.9.0"];
        let forwards: Vec<Release> = versions
            .iter()
            .map(|raw| release(raw, "stable", &["macos-aarch64"]))
            .collect();
        let mut backwards = forwards.clone();
        backwards.reverse();

        for releases in [forwards, backwards] {
            let decision = decide(
                &version("1.0.0"),
                ReleaseChannel::Stable,
                MAC,
                &feed(releases),
            );
            assert_eq!(decision.available().unwrap().version, version("1.10.0"));
        }
    }

    #[test]
    fn an_older_release_still_serves_a_platform_the_newest_one_dropped() {
        let manifest = feed(vec![
            release("2.0.0", "stable", &["macos-aarch64"]),
            release("1.9.0", "stable", &["macos-aarch64", "macos-x86_64"]),
        ]);

        let intel = Target::new(HostOs::MacOs, Arch::X86_64);
        let decision = decide(&version("1.0.0"), ReleaseChannel::Stable, intel, &manifest);
        assert_eq!(decision.available().unwrap().version, version("1.9.0"));
    }

    #[test]
    fn a_platform_that_stopped_being_published_is_unsupported_not_current() {
        let manifest = feed(vec![release("2.0.0", "stable", &["macos-aarch64"])]);
        let windows = Target::new(HostOs::Windows, Arch::X86_64);

        assert_eq!(
            decide(
                &version("1.0.0"),
                ReleaseChannel::Stable,
                windows,
                &manifest
            ),
            UpdateDecision::Unsupported(UnsupportedReason::NoArtifactForTarget {
                target: "windows-x86_64".to_owned(),
                version: version("2.0.0"),
            })
        );
    }

    #[test]
    fn the_decision_for_this_machine_matches_the_one_for_its_target() {
        let manifest = feed(vec![release(
            "9.9.9",
            "stable",
            &[
                "macos-aarch64",
                "macos-x86_64",
                "linux-x86_64",
                "linux-aarch64",
                "windows-x86_64",
                "windows-aarch64",
            ],
        )]);

        let target = Target::current().unwrap();
        assert_eq!(
            decide_for_this_machine(&version("1.0.0"), ReleaseChannel::Stable, &manifest),
            decide(&version("1.0.0"), ReleaseChannel::Stable, target, &manifest)
        );
    }

    #[test]
    fn the_digest_matches_a_known_answer() {
        assert_eq!(sha256_hex(b""), EMPTY_DIGEST);
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn bytes_that_match_the_manifest_verify() {
        let bytes = b"the whole application";
        assert_eq!(verify(bytes, &artifact_for("https://x/y", bytes)), Ok(()));
    }

    #[test]
    fn a_single_flipped_byte_fails_verification() {
        let bytes = b"the whole application".to_vec();
        let declared = artifact_for("https://x/y", &bytes);

        for index in [0, bytes.len() / 2, bytes.len() - 1] {
            let mut tampered = bytes.clone();
            tampered[index] ^= 0x01;
            assert_eq!(
                verify(&tampered, &declared),
                Err(IntegrityError::Checksum),
                "a flip at byte {index} went unnoticed"
            );
        }
    }

    #[test]
    fn a_truncated_download_fails_on_its_length() {
        let bytes = b"the whole application";
        let declared = artifact_for("https://x/y", bytes);

        assert_eq!(
            verify(&bytes[..bytes.len() - 1], &declared),
            Err(IntegrityError::Size {
                expected: bytes.len() as u64,
                actual: (bytes.len() - 1) as u64,
            })
        );
    }

    #[test]
    fn a_checksum_written_in_upper_case_still_matches() {
        let bytes = b"the whole application";
        let mut declared = artifact_for("https://x/y", bytes);
        declared.sha256 = declared.sha256.to_ascii_uppercase();

        assert_eq!(verify(bytes, &declared), Ok(()));
    }

    #[test]
    fn digests_of_different_lengths_never_match() {
        assert!(!digests_match(EMPTY_DIGEST, &EMPTY_DIGEST[1..]));
        assert!(digests_match(EMPTY_DIGEST, EMPTY_DIGEST));
    }

    #[test]
    fn an_unstated_build_profile_is_the_strict_one() {
        assert_eq!(BuildProfile::default(), BuildProfile::Production);
        assert_eq!(FeedProvenance::default(), FeedProvenance::Unsigned);
        assert_eq!(
            apply_permission(ApplyPolicy::default()),
            ApplyPermission::UnsignedFeedInProduction
        );
    }

    #[test]
    fn a_production_build_refuses_an_unsigned_feed_even_with_the_flag_on() {
        let policy = ApplyPolicy {
            profile: BuildProfile::Production,
            provenance: FeedProvenance::Unsigned,
            enabled: true,
        };
        assert_eq!(
            apply_permission(policy),
            ApplyPermission::UnsignedFeedInProduction,
            "the flag must not be a way around the missing signature"
        );
    }

    #[test]
    fn applying_stays_off_until_something_turns_it_on() {
        let policy = ApplyPolicy {
            profile: BuildProfile::Development,
            provenance: FeedProvenance::Unsigned,
            enabled: false,
        };
        assert_eq!(apply_permission(policy), ApplyPermission::Disabled);
    }

    #[test]
    fn a_development_build_may_apply_and_a_signed_feed_unblocks_production() {
        assert_eq!(
            apply_permission(ApplyPolicy {
                profile: BuildProfile::Development,
                provenance: FeedProvenance::Unsigned,
                enabled: true,
            }),
            ApplyPermission::Allowed
        );
        assert_eq!(
            apply_permission(ApplyPolicy {
                profile: BuildProfile::Production,
                provenance: FeedProvenance::Signed,
                enabled: true,
            }),
            ApplyPermission::Allowed
        );
    }

    fn site(host: HostOs, shape: InstallShape, helper: Option<PathBuf>) -> InstallSite {
        InstallSite {
            host,
            shape,
            installed: PathBuf::from("/opt/riseonly/Riseonly"),
            staging: PathBuf::from("/tmp/rise-staging"),
            helper,
        }
    }

    #[test]
    fn each_host_installs_the_only_way_its_os_allows() {
        let update = update_named("1.4.0", "https://downloads.riseonly.net/1.4.0/Riseonly.zip");

        let macos = plan_install(
            &site(HostOs::MacOs, InstallShape::SelfContained, None),
            &update,
        );
        assert_eq!(macos.method, InstallMethod::SwapBundle);

        let windows = plan_install(
            &site(
                HostOs::Windows,
                InstallShape::SelfContained,
                Some(PathBuf::from(
                    "C:\\Program Files\\Riseonly\\riseonly-updater.exe",
                )),
            ),
            &update,
        );
        assert_eq!(
            windows.method,
            InstallMethod::HelperProcess {
                helper: PathBuf::from("C:\\Program Files\\Riseonly\\riseonly-updater.exe"),
            }
        );

        let linux = plan_install(
            &site(HostOs::Linux, InstallShape::SelfContained, None),
            &update,
        );
        assert_eq!(linux.method, InstallMethod::ReplaceTree);
    }

    #[test]
    fn windows_never_replaces_its_own_running_image() {
        let update = update_named("1.4.0", "https://downloads.riseonly.net/1.4.0/Riseonly.zip");

        for helper in [Some(PathBuf::from("C:\\App\\riseonly-updater.exe")), None] {
            let plan = plan_install(
                &site(HostOs::Windows, InstallShape::SelfContained, helper),
                &update,
            );
            assert!(
                !plan.method.replaces_in_place(),
                "the OS holds the lock; this process cannot swap itself"
            );
        }
    }

    #[test]
    fn a_windows_install_without_its_helper_says_so_in_the_plan() {
        let update = update_named("1.4.0", "https://downloads.riseonly.net/1.4.0/Riseonly.zip");
        let plan = plan_install(
            &site(HostOs::Windows, InstallShape::SelfContained, None),
            &update,
        );

        assert_eq!(plan.method, InstallMethod::HelperUnavailable);
        assert!(matches!(install(&plan), Err(UpdateError::HelperMissing)));
    }

    #[test]
    fn the_linux_plan_follows_the_install_shape_and_not_just_the_os() {
        let update = update_named(
            "1.4.0",
            "https://downloads.riseonly.net/1.4.0/Riseonly.AppImage",
        );

        let appimage = plan_install(&site(HostOs::Linux, InstallShape::AppImage, None), &update);
        assert_eq!(appimage.method, InstallMethod::ReplaceFile);
        assert!(appimage.method.replaces_in_place());

        let packaged = plan_install(
            &site(HostOs::Linux, InstallShape::PackageManaged, None),
            &update,
        );
        assert_eq!(packaged.method, InstallMethod::PackageManager);
        assert!(
            !packaged.method.replaces_in_place(),
            "overwriting a deb's files corrupts the package database"
        );
    }

    #[test]
    fn a_package_managed_install_reports_that_it_did_not_update_itself() {
        let update = update_named("1.4.0", "https://downloads.riseonly.net/1.4.0/Riseonly.deb");
        let plan = plan_install(
            &site(HostOs::Linux, InstallShape::PackageManaged, None),
            &update,
        );

        assert_eq!(install(&plan).unwrap(), PlatformSupport::Unsupported);
    }

    #[test]
    fn only_a_swap_this_process_performed_relaunches_through_gpui() {
        let methods = [
            (InstallMethod::SwapBundle, true),
            (InstallMethod::ReplaceFile, true),
            (InstallMethod::ReplaceTree, true),
            (
                InstallMethod::HelperProcess {
                    helper: PathBuf::from("C:\\App\\riseonly-updater.exe"),
                },
                false,
            ),
            (InstallMethod::HelperUnavailable, false),
            (InstallMethod::PackageManager, false),
        ];

        for (method, expected) in methods {
            assert_eq!(
                method.replaces_in_place(),
                expected,
                "{method:?} relaunching is what races the helper"
            );
        }
    }

    #[test]
    fn an_appimage_replaces_the_file_the_user_launched_not_the_mount_point() {
        let resolved = InstallSite::resolve(
            HostOs::Linux,
            Path::new("/tmp/.mount_Riseon1a2b3c/AppRun"),
            Some(Path::new("/home/u/Applications/Riseonly.AppImage")),
            PathBuf::from("/home/u/.cache/riseonly"),
            None,
        );

        assert_eq!(resolved.shape, InstallShape::AppImage);
        assert_eq!(
            resolved.installed,
            Path::new("/home/u/Applications/Riseonly.AppImage"),
            "the mount disappears with the process; only $APPIMAGE names the real file"
        );
    }

    #[test]
    fn a_distribution_package_is_recognised_from_where_it_was_installed() {
        for path in [
            "/usr/bin/riseonly",
            "/usr/local/bin/riseonly",
            "/opt/riseonly/riseonly",
            "/snap/riseonly/current/bin/riseonly",
            "/app/bin/riseonly",
        ] {
            let resolved = InstallSite::resolve(
                HostOs::Linux,
                Path::new(path),
                None,
                PathBuf::from("/tmp"),
                None,
            );
            assert_eq!(
                resolved.shape,
                InstallShape::PackageManaged,
                "{path} belongs to the system, not to us"
            );
        }
    }

    #[test]
    fn an_unpacked_tarball_in_a_home_directory_may_replace_itself() {
        let resolved = InstallSite::resolve(
            HostOs::Linux,
            Path::new("/home/u/riseonly/riseonly"),
            None,
            PathBuf::from("/tmp"),
            None,
        );
        assert_eq!(resolved.shape, InstallShape::SelfContained);
    }

    #[test]
    fn macos_and_windows_replace_what_gpui_called_the_app_path() {
        for host in [HostOs::MacOs, HostOs::Windows] {
            let resolved = InstallSite::resolve(
                host,
                Path::new("/Applications/Riseonly.app"),
                Some(Path::new("/home/u/Riseonly.AppImage")),
                PathBuf::from("/tmp"),
                None,
            );
            assert_eq!(resolved.shape, InstallShape::SelfContained);
            assert_eq!(
                resolved.installed,
                Path::new("/Applications/Riseonly.app"),
                "a stray $APPIMAGE in the environment must not redirect the swap"
            );
        }
    }

    #[test]
    fn a_feed_cannot_choose_a_path_outside_the_staging_directory() {
        let version = version("1.4.0");
        for url in [
            "https://downloads.riseonly.net/1.4.0/../../../Applications/Evil.app",
            "https://downloads.riseonly.net/1.4.0/..",
            "https://downloads.riseonly.net/1.4.0/",
            "https://downloads.riseonly.net/1.4.0/.bashrc",
            "https://downloads.riseonly.net/1.4.0/a/b%2f..%2fc",
            "",
        ] {
            let name = staged_file_name(url, &version);
            assert_eq!(
                Path::new(&name).components().count(),
                1,
                "{url} produced {name}"
            );
            assert!(!name.starts_with('.'), "{url} produced {name}");
            assert!(!name.contains(".."), "{url} produced {name}");
        }
    }

    #[test]
    fn an_ordinary_artifact_keeps_the_name_it_was_published_under() {
        let version = version("1.4.0");
        assert_eq!(
            staged_file_name(
                "https://downloads.riseonly.net/1.4.0/Riseonly-1.4.0.dmg",
                &version
            ),
            "Riseonly-1.4.0.dmg"
        );
        assert_eq!(
            staged_file_name(
                "https://downloads.riseonly.net/Riseonly.AppImage?token=abc#frag",
                &version
            ),
            "Riseonly.AppImage"
        );
    }

    #[test]
    fn the_displaced_copy_stays_in_the_directory_it_is_renamed_from() {
        let update = update_named("1.4.0", "https://downloads.riseonly.net/1.4.0/Riseonly.zip");
        let mut where_it_lives = site(HostOs::MacOs, InstallShape::SelfContained, None);
        where_it_lives.installed = PathBuf::from("/Applications/Riseonly.app");

        let plan = plan_install(&where_it_lives, &update);
        assert_eq!(
            plan.displaced.parent(),
            Path::new("/Applications/Riseonly.app").parent(),
            "a rename across volumes is a copy, and a copy can be interrupted"
        );
        assert_ne!(plan.displaced, plan.installed);
    }

    #[test]
    fn the_helper_is_handed_separate_arguments_including_the_pid_to_wait_for() {
        let plan = InstallPlan {
            method: InstallMethod::HelperProcess {
                helper: PathBuf::from("C:\\Program Files\\Riseonly\\riseonly-updater.exe"),
            },
            staged: PathBuf::from("C:\\Users\\u\\AppData\\Local\\Rise Staging\\Riseonly.zip"),
            installed: PathBuf::from("C:\\Program Files\\Riseonly\\riseonly.exe"),
            displaced: PathBuf::from("C:\\Program Files\\Riseonly\\.riseonly.exe.previous"),
        };

        let arguments = helper_arguments(&plan, 4321);

        assert!(arguments.contains(&"4321".to_owned()));
        assert!(
            arguments
                .contains(&"C:\\Users\\u\\AppData\\Local\\Rise Staging\\Riseonly.zip".to_owned()),
            "a path with a space must survive as one argument, not two"
        );
        assert!(arguments.contains(&"C:\\Program Files\\Riseonly\\riseonly.exe".to_owned()));
    }

    #[test]
    fn the_helper_is_looked_for_beside_the_executable() {
        let executable = Path::new("/Applications/Riseonly/riseonly.exe");

        assert_eq!(
            helper_beside_executable(executable, WINDOWS_HELPER_EXECUTABLE),
            Some(PathBuf::from("/Applications/Riseonly/riseonly-updater.exe"))
        );
    }

    #[test]
    fn an_executable_with_no_directory_yields_no_helper_rather_than_a_bare_name() {
        assert_eq!(
            helper_beside_executable(Path::new("riseonly-updater.exe"), WINDOWS_HELPER_EXECUTABLE),
            None,
            "a bare name would be resolved from the working directory and PATH"
        );
    }

    #[test]
    fn preparing_refuses_before_it_looks_at_the_bytes() {
        let update = update_named("1.4.0", "https://downloads.riseonly.net/1.4.0/Riseonly.zip");
        let refused = prepare(
            &site(HostOs::MacOs, InstallShape::SelfContained, None),
            ApplyPolicy::default(),
            &update,
            b"not even the right bytes",
        );

        assert!(
            matches!(
                refused,
                Err(UpdateError::Refused(
                    ApplyPermission::UnsignedFeedInProduction
                ))
            ),
            "a build that may not install anything must not hash the download first"
        );
    }

    #[test]
    fn preparing_produces_a_plan_only_for_bytes_that_match() {
        let update = update_named("1.4.0", "https://downloads.riseonly.net/1.4.0/Riseonly.zip");
        let policy = ApplyPolicy {
            profile: BuildProfile::Development,
            provenance: FeedProvenance::Unsigned,
            enabled: true,
        };
        let where_it_lives = site(HostOs::MacOs, InstallShape::SelfContained, None);

        assert!(matches!(
            prepare(&where_it_lives, policy, &update, b"tampered"),
            Err(UpdateError::Integrity(_))
        ));

        let plan = prepare(&where_it_lives, policy, &update, b"1.4.0").unwrap();
        assert_eq!(plan.method, InstallMethod::SwapBundle);
        assert_eq!(plan.staged, Path::new("/tmp/rise-staging/Riseonly.zip"));
    }

    #[test]
    fn a_swap_replaces_the_installed_tree_and_leaves_nothing_behind() {
        let dir = scratch("swap");
        let installed = dir.join("Riseonly.app");
        let staging = dir.join("stage");

        std::fs::create_dir_all(installed.join("Contents")).unwrap();
        std::fs::write(installed.join("Contents/version"), "1.0.0").unwrap();
        std::fs::create_dir_all(staging.join("Riseonly.app/Contents")).unwrap();
        std::fs::write(staging.join("Riseonly.app/Contents/version"), "1.4.0").unwrap();

        let where_it_lives = InstallSite {
            host: HostOs::MacOs,
            shape: InstallShape::SelfContained,
            installed: installed.clone(),
            staging,
            helper: None,
        };
        let plan = plan_install(
            &where_it_lives,
            &update_named("1.4.0", "https://downloads.riseonly.net/1.4.0/Riseonly.app"),
        );

        assert_eq!(install(&plan).unwrap(), PlatformSupport::Performed);
        assert_eq!(
            std::fs::read_to_string(installed.join("Contents/version")).unwrap(),
            "1.4.0"
        );
        assert!(
            std::fs::symlink_metadata(&plan.displaced).is_err(),
            "the displaced copy must not be left sitting next to the application"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_swap_that_cannot_complete_puts_the_running_copy_back() {
        let dir = scratch("rollback");
        let installed = dir.join("Riseonly.app");
        let staging = dir.join("stage");

        std::fs::create_dir_all(installed.join("Contents")).unwrap();
        std::fs::write(installed.join("Contents/version"), "1.0.0").unwrap();
        std::fs::create_dir_all(&staging).unwrap();

        let plan = InstallPlan {
            method: InstallMethod::SwapBundle,
            staged: staging.join("Riseonly.app"),
            installed: installed.clone(),
            displaced: displaced_path(&installed, &staging),
        };

        assert!(
            install(&plan).is_err(),
            "nothing was staged, so there is nothing to swap in"
        );
        assert_eq!(
            std::fs::read_to_string(installed.join("Contents/version")).unwrap(),
            "1.0.0",
            "a failed swap must never leave the user without an application"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn an_appimage_swap_replaces_one_file_with_another() {
        let dir = scratch("appimage");
        let installed = dir.join("Riseonly.AppImage");
        let staging = dir.join("stage");

        std::fs::write(&installed, "old").unwrap();
        std::fs::create_dir_all(&staging).unwrap();
        std::fs::write(staging.join("Riseonly.AppImage"), "new").unwrap();

        let where_it_lives = InstallSite {
            host: HostOs::Linux,
            shape: InstallShape::AppImage,
            installed: installed.clone(),
            staging,
            helper: None,
        };
        let plan = plan_install(
            &where_it_lives,
            &update_named(
                "1.4.0",
                "https://downloads.riseonly.net/1.4.0/Riseonly.AppImage",
            ),
        );

        assert_eq!(install(&plan).unwrap(), PlatformSupport::Performed);
        assert_eq!(std::fs::read_to_string(&installed).unwrap(), "new");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn an_appimage_swap_never_opens_the_installed_file_for_writing() {
        let dir = scratch("appimage-read-only");
        let installed = dir.join("Riseonly.AppImage");
        let staging = dir.join("stage");
        std::fs::write(&installed, "old").unwrap();
        let mut permissions = std::fs::metadata(&installed).unwrap().permissions();
        permissions.set_readonly(true);
        std::fs::set_permissions(&installed, permissions).unwrap();
        std::fs::create_dir_all(&staging).unwrap();
        std::fs::write(staging.join("Riseonly.AppImage"), "new").unwrap();

        let plan = InstallPlan {
            method: InstallMethod::ReplaceFile,
            staged: staging.join("Riseonly.AppImage"),
            installed: installed.clone(),
            displaced: displaced_path(&installed, &staging),
        };

        assert_eq!(install(&plan).unwrap(), PlatformSupport::Performed);
        assert_eq!(std::fs::read_to_string(&installed).unwrap(), "new");
        std::fs::remove_dir_all(&dir).ok();
    }

    const RFC8032_KEY: &str = "11qYAYKxCrfVS/7TyWQHOg7hcvPapiMlrwIaaPcHURo=";
    const RFC8032_EMPTY_SIGNATURE: &str =
        "5VZDAMNgrHKQhuLMgG6CioSHfx645dl02HPgZSJJAVVfuIIVkKM7rMYeOXAc+bRr0lv18FlbviRlUUFDjnoQCw==";

    #[test]
    fn a_feed_signature_checks_against_the_rfc_vector() {
        assert!(feed_signature_valid(b"", RFC8032_EMPTY_SIGNATURE, RFC8032_KEY));
        assert!(!feed_signature_valid(b"x", RFC8032_EMPTY_SIGNATURE, RFC8032_KEY));
        assert!(!feed_signature_valid(b"", RFC8032_EMPTY_SIGNATURE, "AAAA"));
        assert!(!feed_signature_valid(b"", "not base64", RFC8032_KEY));
    }

    #[test]
    fn newer_count_counts_versions_this_target_can_take() {
        let manifest = feed(vec![
            release("1.3.0", "stable", &["macos-aarch64"]),
            release("1.4.0", "stable", &["macos-aarch64"]),
            release("1.5.0", "stable", &["macos-aarch64", "linux-x86_64"]),
            release("1.5.0", "stable", &["macos-aarch64"]),
            release("1.6.0", "beta", &["macos-aarch64"]),
            release("1.7.0", "stable", &["linux-x86_64"]),
        ]);
        assert_eq!(newer_count(&version("1.3.0"), ReleaseChannel::Stable, MAC, &manifest), 2);
        assert_eq!(newer_count(&version("1.3.0"), ReleaseChannel::Beta, MAC, &manifest), 3);
        assert_eq!(newer_count(&version("1.5.0"), ReleaseChannel::Stable, MAC, &manifest), 0);
    }

    #[test]
    fn unpack_finds_the_bundle_in_a_mac_zip() {
        if HostOs::current() != HostOs::MacOs {
            return;
        }
        let dir = scratch("unpack-mac");
        let bundle = dir.join("Riseonly.app/Contents");
        std::fs::create_dir_all(&bundle).unwrap();
        std::fs::write(bundle.join("Info.plist"), "plist").unwrap();
        let archive = dir.join("Riseonly.zip");
        let status = std::process::Command::new("/usr/bin/ditto")
            .args(["-c", "-k", "--keepParent"])
            .arg(dir.join("Riseonly.app"))
            .arg(&archive)
            .status()
            .unwrap();
        assert!(status.success());

        let unpacked = unpack(HostOs::MacOs, &archive, &dir.join("unpacked")).unwrap();
        assert_eq!(unpacked, dir.join("unpacked/Riseonly.app"));
        assert_eq!(std::fs::read_to_string(unpacked.join("Contents/Info.plist")).unwrap(), "plist");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn replace_running_tree_steps_the_exe_aside_and_copies_the_rest() {
        let dir = scratch("replace-tree");
        let install = dir.join("install");
        let fresh = dir.join("fresh");
        std::fs::create_dir_all(install.join("assets")).unwrap();
        std::fs::create_dir_all(fresh.join("assets")).unwrap();
        std::fs::write(install.join("riseonly.exe"), "old exe").unwrap();
        std::fs::write(install.join("assets/a.json"), "old").unwrap();
        std::fs::write(fresh.join("riseonly.exe"), "new exe").unwrap();
        std::fs::write(fresh.join("assets/a.json"), "new").unwrap();

        replace_running_tree(&fresh, &install, &install.join("riseonly.exe")).unwrap();
        assert_eq!(std::fs::read_to_string(install.join("riseonly.exe")).unwrap(), "new exe");
        assert_eq!(std::fs::read_to_string(install.join("assets/a.json")).unwrap(), "new");
        assert_eq!(std::fs::read_to_string(install.join("riseonly.exe.old")).unwrap(), "old exe");

        remove_leftovers(&install);
        assert!(!install.join("riseonly.exe.old").exists());
        std::fs::remove_dir_all(&dir).ok();
    }
}
