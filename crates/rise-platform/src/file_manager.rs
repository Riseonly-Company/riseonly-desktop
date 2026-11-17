use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

/// raw values of the cocoa error codes for the file errors the app inspects
/// so comparing codes gives the same number as on ios
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CocoaErrorCode {
    FileNoSuchFile,
    FileLocking,
    FileReadUnknown,
    FileReadNoPermission,
    FileReadInvalidFileName,
    FileReadCorruptFile,
    FileReadNoSuchFile,
    FileWriteUnknown,
    FileWriteNoPermission,
    FileWriteInvalidFileName,
    FileWriteFileExists,
    FileWriteOutOfSpace,
    FileWriteVolumeReadOnly,
    CoderReadCorrupt,
}

impl CocoaErrorCode {
    pub fn raw_value(&self) -> i64 {
        match self {
            Self::FileNoSuchFile => 4,
            Self::FileLocking => 255,
            Self::FileReadUnknown => 256,
            Self::FileReadNoPermission => 257,
            Self::FileReadInvalidFileName => 258,
            Self::FileReadCorruptFile => 259,
            Self::FileReadNoSuchFile => 260,
            Self::FileWriteUnknown => 512,
            Self::FileWriteNoPermission => 513,
            Self::FileWriteInvalidFileName => 514,
            Self::FileWriteFileExists => 516,
            Self::FileWriteOutOfSpace => 640,
            Self::FileWriteVolumeReadOnly => 642,
            Self::CoderReadCorrupt => 4864,
        }
    }

    pub fn from_raw(raw: i64) -> Option<Self> {
        let code = match raw {
            4 => Self::FileNoSuchFile,
            255 => Self::FileLocking,
            256 => Self::FileReadUnknown,
            257 => Self::FileReadNoPermission,
            258 => Self::FileReadInvalidFileName,
            259 => Self::FileReadCorruptFile,
            260 => Self::FileReadNoSuchFile,
            512 => Self::FileWriteUnknown,
            513 => Self::FileWriteNoPermission,
            514 => Self::FileWriteInvalidFileName,
            516 => Self::FileWriteFileExists,
            640 => Self::FileWriteOutOfSpace,
            642 => Self::FileWriteVolumeReadOnly,
            4864 => Self::CoderReadCorrupt,
            _ => return None,
        };
        Some(code)
    }
}

/// twin of the foundation cocoa error, what the file manager throws and what app code throws itself
#[derive(Clone, Debug)]
pub struct CocoaError {
    pub code: CocoaErrorCode,
    pub path: Option<PathBuf>,
    pub underlying: Option<Arc<io::Error>>,
}

impl CocoaError {
    pub fn new(code: CocoaErrorCode) -> Self {
        Self {
            code,
            path: None,
            underlying: None,
        }
    }

    fn at(code: CocoaErrorCode, path: &Path) -> Self {
        Self {
            code,
            path: Some(path.to_path_buf()),
            underlying: None,
        }
    }

    fn from_io(error: io::Error, path: &Path, operation: FileOperation) -> Self {
        let code = match (error.kind(), operation) {
            (io::ErrorKind::NotFound, FileOperation::Read) => CocoaErrorCode::FileReadNoSuchFile,
            (io::ErrorKind::NotFound, FileOperation::Write) => CocoaErrorCode::FileNoSuchFile,
            (io::ErrorKind::PermissionDenied, FileOperation::Read) => {
                CocoaErrorCode::FileReadNoPermission
            }
            (io::ErrorKind::PermissionDenied, FileOperation::Write) => {
                CocoaErrorCode::FileWriteNoPermission
            }
            (io::ErrorKind::AlreadyExists, _) => CocoaErrorCode::FileWriteFileExists,
            (io::ErrorKind::StorageFull, _) => CocoaErrorCode::FileWriteOutOfSpace,
            (io::ErrorKind::ReadOnlyFilesystem, _) => CocoaErrorCode::FileWriteVolumeReadOnly,
            (io::ErrorKind::InvalidFilename, FileOperation::Read) => {
                CocoaErrorCode::FileReadInvalidFileName
            }
            (io::ErrorKind::InvalidFilename, FileOperation::Write) => {
                CocoaErrorCode::FileWriteInvalidFileName
            }
            (_, FileOperation::Read) => CocoaErrorCode::FileReadUnknown,
            (_, FileOperation::Write) => CocoaErrorCode::FileWriteUnknown,
        };
        Self {
            code,
            path: Some(path.to_path_buf()),
            underlying: Some(Arc::new(error)),
        }
    }
}

impl fmt::Display for CocoaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "CocoaError {} ({:?})", self.code.raw_value(), self.code)?;
        if let Some(path) = &self.path {
            write!(f, " at {}", path.display())?;
        }
        if let Some(underlying) = &self.underlying {
            write!(f, ": {underlying}")?;
        }
        Ok(())
    }
}

impl std::error::Error for CocoaError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.underlying
            .as_deref()
            .map(|error| error as &(dyn std::error::Error + 'static))
    }
}

#[derive(Clone, Copy)]
enum FileOperation {
    Read,
    Write,
}

/// twin of the file attribute type for the values the app compares
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FileAttributeType {
    TypeRegular,
    TypeDirectory,
    TypeSymbolicLink,
    TypeUnknown,
}

/// twin of the attributes dictionary typed for the keys the app reads (size
/// modification date, creation date and type)
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileAttributes {
    pub size: i64,
    pub modification_date: Option<SystemTime>,
    pub creation_date: Option<SystemTime>,
    pub r#type: FileAttributeType,
}

/// twin of the directory enumeration options for the option the app passes
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DirectoryEnumerationOptions {
    pub skips_hidden_files: bool,
}

impl DirectoryEnumerationOptions {
    pub const SKIPS_HIDDEN_FILES: Self = Self {
        skips_hidden_files: true,
    };
}

/// twin of the default file manager, synchronous so callers keep it off the gpui main thread
/// directories come from crate paths, never from here
#[derive(Clone, Copy, Debug, Default)]
pub struct FileManager;

impl FileManager {
    /// follows symlinks, a dangling link answers false
    pub fn file_exists(&self, at_path: &Path) -> bool {
        std::fs::metadata(at_path).is_ok()
    }

    pub fn file_exists_is_directory(&self, at_path: &Path, is_directory: &mut bool) -> bool {
        match std::fs::metadata(at_path) {
            Ok(metadata) => {
                *is_directory = metadata.is_dir();
                true
            }
            Err(_) => false,
        }
    }

    /// an existing directory is success only with intermediate directories
    /// an existing file at the path is always file write file exists
    pub fn create_directory(
        &self,
        at: &Path,
        with_intermediate_directories: bool,
    ) -> Result<(), CocoaError> {
        if let Ok(metadata) = std::fs::metadata(at) {
            if metadata.is_dir() && with_intermediate_directories {
                return Ok(());
            }
            return Err(CocoaError::at(CocoaErrorCode::FileWriteFileExists, at));
        }
        let result = if with_intermediate_directories {
            std::fs::create_dir_all(at)
        } else {
            std::fs::create_dir(at)
        };
        result.map_err(|error| CocoaError::from_io(error, at, FileOperation::Write))
    }

    /// removes a file, a symlink itself or a directory with everything in it
    /// a missing item is an error, callers that dont care use ok
    pub fn remove_item(&self, at: &Path) -> Result<(), CocoaError> {
        let metadata = std::fs::symlink_metadata(at)
            .map_err(|error| CocoaError::from_io(error, at, FileOperation::Write))?;
        let result = retrying_share_conflict(|| {
            if metadata.is_dir() {
                std::fs::remove_dir_all(at)
            } else {
                std::fs::remove_file(at).or_else(|error| {
                    if metadata.file_type().is_symlink() {
                        std::fs::remove_dir(at)
                    } else {
                        Err(error)
                    }
                })
            }
        });
        result.map_err(|error| CocoaError::from_io(error, at, FileOperation::Write))
    }

    /// fails with file write file exists when the destination exists
    /// unix rename would silently replace it, this doesnt
    /// across volumes it copies then removes
    pub fn move_item(&self, at: &Path, to: &Path) -> Result<(), CocoaError> {
        if std::fs::symlink_metadata(at).is_err() {
            return Err(CocoaError::at(CocoaErrorCode::FileNoSuchFile, at));
        }
        if std::fs::symlink_metadata(to).is_ok() {
            return Err(CocoaError::at(CocoaErrorCode::FileWriteFileExists, to));
        }
        match retrying_share_conflict(|| std::fs::rename(at, to)) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::CrossesDevices => {
                self.copy_item(at, to)?;
                self.remove_item(at)
            }
            Err(error) => Err(CocoaError::from_io(error, to, FileOperation::Write)),
        }
    }

    /// copies a file or a whole directory tree, the destination must not exist
    pub fn copy_item(&self, at: &Path, to: &Path) -> Result<(), CocoaError> {
        let metadata = std::fs::symlink_metadata(at)
            .map_err(|error| CocoaError::from_io(error, at, FileOperation::Read))?;
        if std::fs::symlink_metadata(to).is_ok() {
            return Err(CocoaError::at(CocoaErrorCode::FileWriteFileExists, to));
        }
        if metadata.is_dir() {
            copy_directory(at, to)
        } else {
            std::fs::copy(at, to)
                .map(|_| ())
                .map_err(|error| CocoaError::from_io(error, to, FileOperation::Write))
        }
    }

    /// writes contents (or an empty file) replacing anything there
    /// answers success as a bool instead of throwing
    pub fn create_file(&self, at_path: &Path, contents: Option<&[u8]>) -> bool {
        std::fs::write(at_path, contents.unwrap_or_default()).is_ok()
    }

    /// names of the items in the directory, hidden ones included and in no promised order
    pub fn contents_of_directory_at_path(&self, at_path: &Path) -> Result<Vec<String>, CocoaError> {
        let entries = std::fs::read_dir(at_path)
            .map_err(|error| CocoaError::from_io(error, at_path, FileOperation::Read))?;
        let mut names = Vec::new();
        for entry in entries {
            let entry =
                entry.map_err(|error| CocoaError::from_io(error, at_path, FileOperation::Read))?;
            names.push(entry.file_name().to_string_lossy().into_owned());
        }
        Ok(names)
    }

    /// full paths of the items in the directory, skips_hidden_files drops dot named items
    pub fn contents_of_directory_at(
        &self,
        at: &Path,
        options: DirectoryEnumerationOptions,
    ) -> Result<Vec<PathBuf>, CocoaError> {
        let entries = std::fs::read_dir(at)
            .map_err(|error| CocoaError::from_io(error, at, FileOperation::Read))?;
        let mut paths = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|error| CocoaError::from_io(error, at, FileOperation::Read))?;
            if options.skips_hidden_files && entry.file_name().to_string_lossy().starts_with('.') {
                continue;
            }
            paths.push(entry.path());
        }
        Ok(paths)
    }

    /// attributes of the item itself, a symlink reports its own type
    pub fn attributes_of_item(&self, at_path: &Path) -> Result<FileAttributes, CocoaError> {
        let metadata = std::fs::symlink_metadata(at_path)
            .map_err(|error| CocoaError::from_io(error, at_path, FileOperation::Read))?;
        let file_type = metadata.file_type();
        let r#type = if file_type.is_symlink() {
            FileAttributeType::TypeSymbolicLink
        } else if file_type.is_dir() {
            FileAttributeType::TypeDirectory
        } else if file_type.is_file() {
            FileAttributeType::TypeRegular
        } else {
            FileAttributeType::TypeUnknown
        };
        Ok(FileAttributes {
            size: i64::try_from(metadata.len()).unwrap_or(i64::MAX),
            modification_date: metadata.modified().ok(),
            creation_date: metadata.created().ok(),
            r#type,
        })
    }
}

const SHARE_CONFLICT_OS_ERRORS: [i32; 2] = [32, 33];
const SHARE_CONFLICT_RETRIES: u32 = 8;
const SHARE_CONFLICT_PAUSE: Duration = Duration::from_millis(50);

/// windows refuses to rename or delete a file another handle holds, unix doesnt care (i hate it)
/// a player stopped a moment ago closes its source on its own thread
/// so moving a voice draft right away would fail, this retries and waits only on windows
pub(crate) fn retrying_share_conflict<T>(mut operation: impl FnMut() -> io::Result<T>) -> io::Result<T> {
    let mut retries_left = SHARE_CONFLICT_RETRIES;
    loop {
        match operation() {
            Err(error)
                if retries_left > 0
                    && error.raw_os_error().is_some_and(|code| SHARE_CONFLICT_OS_ERRORS.contains(&code)) =>
            {
                retries_left -= 1;
                std::thread::sleep(SHARE_CONFLICT_PAUSE);
            }
            result => return result,
        }
    }
}

fn copy_directory(from: &Path, to: &Path) -> Result<(), CocoaError> {
    std::fs::create_dir(to).map_err(|error| CocoaError::from_io(error, to, FileOperation::Write))?;
    let entries =
        std::fs::read_dir(from).map_err(|error| CocoaError::from_io(error, from, FileOperation::Read))?;
    for entry in entries {
        let entry = entry.map_err(|error| CocoaError::from_io(error, from, FileOperation::Read))?;
        let source = entry.path();
        let target = to.join(entry.file_name());
        let file_type = entry
            .file_type()
            .map_err(|error| CocoaError::from_io(error, &source, FileOperation::Read))?;
        if file_type.is_dir() {
            copy_directory(&source, &target)?;
        } else {
            std::fs::copy(&source, &target)
                .map_err(|error| CocoaError::from_io(error, &target, FileOperation::Write))?;
        }
    }
    Ok(())
}
