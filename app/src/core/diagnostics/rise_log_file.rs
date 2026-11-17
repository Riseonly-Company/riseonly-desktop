use std::fs::{File, OpenOptions};
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::time::Duration;

use tracing_subscriber::fmt::MakeWriter;

pub const LOG_FILE_STEM: &str = "riseonly";
pub const LOG_FILE_EXTENSION: &str = "log";
pub const LOG_FILE_MAX_BYTES: u64 = 20 * 1024 * 1024;
pub const LOG_FILE_COUNT: usize = 5;
pub const LOG_FILE_QUEUE_LINES: usize = 4096;
pub const LOG_FILE_FILTER: &str = "warn";
pub const LOG_FILE_REDACTED: &str = "<redacted>";
pub const LOG_FILE_PANIC_FLUSH: Duration = Duration::from_millis(500);

const BEARER_PREFIX: &str = "Bearer ";
const PRIVATE_FIELD_SUFFIXES: [&str; 6] = ["token", "jwt", "secret", "password", "cookie", "authorization"];
const PRIVATE_FIELD_WORDS: [&str; 8] = ["phone", "email", "otp", "text", "body", "content", "caption", "query"];

static LOG_FILE_PATH: OnceLock<PathBuf> = OnceLock::new();

enum LogFileMessage {
    Line(Vec<u8>),
    Flush(mpsc::Sender<()>),
}

/// tracing writer into the os log folder with size rotation, lines go to a writer thread
#[derive(Clone)]
pub struct RiseLogFile {
    sender: SyncSender<LogFileMessage>,
}

impl RiseLogFile {
    pub fn start() -> Option<Self> {
        let directory = rise_platform::paths::system_logs_directory()?;
        let log_file = Self::start_in(directory.clone(), LOG_FILE_MAX_BYTES, LOG_FILE_COUNT)?;
        let _ = LOG_FILE_PATH.set(log_file_path(&directory, 0));
        Some(log_file)
    }

    /// starts a writer for a given folder, size limit per file and number of kept files
    pub fn start_in(directory: PathBuf, max_bytes: u64, count: usize) -> Option<Self> {
        std::fs::create_dir_all(&directory).ok()?;
        let sink = RotatingLogFile::open(directory, max_bytes, count).ok()?;
        let (sender, receiver) = mpsc::sync_channel(LOG_FILE_QUEUE_LINES);
        std::thread::Builder::new()
            .name("riseonly-log-file".into())
            .spawn(move || sink.run(receiver))
            .ok()?;
        Some(Self { sender })
    }

    pub fn current_path() -> Option<&'static Path> {
        LOG_FILE_PATH.get().map(PathBuf::as_path)
    }

    /// waits until every line queued before the call is on disk, false on timeout
    pub fn flush(&self, timeout: Duration) -> bool {
        let (done, wait) = mpsc::channel();
        if self.sender.send(LogFileMessage::Flush(done)).is_err() {
            return false;
        }
        wait.recv_timeout(timeout).is_ok()
    }
}

/// one formatted event, handed to the writer thread on drop and dropped when the queue is full
pub struct RiseLogFileLine {
    sender: SyncSender<LogFileMessage>,
    buffer: Vec<u8>,
}

impl Write for RiseLogFileLine {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.buffer.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Drop for RiseLogFileLine {
    fn drop(&mut self) {
        if !self.buffer.is_empty() {
            let _ = self.sender.try_send(LogFileMessage::Line(std::mem::take(&mut self.buffer)));
        }
    }
}

impl<'a> MakeWriter<'a> for RiseLogFile {
    type Writer = RiseLogFileLine;

    fn make_writer(&'a self) -> Self::Writer {
        RiseLogFileLine {
            sender: self.sender.clone(),
            buffer: Vec::new(),
        }
    }
}

struct RotatingLogFile {
    directory: PathBuf,
    max_bytes: u64,
    count: usize,
    file: BufWriter<File>,
    size: u64,
}

impl RotatingLogFile {
    fn open(directory: PathBuf, max_bytes: u64, count: usize) -> io::Result<Self> {
        let file = open_append(&log_file_path(&directory, 0))?;
        let size = file.metadata().map_or(0, |metadata| metadata.len());
        Ok(Self {
            directory,
            max_bytes,
            count: count.max(1),
            file: BufWriter::new(file),
            size,
        })
    }

    fn run(mut self, receiver: Receiver<LogFileMessage>) {
        while let Ok(first) = receiver.recv() {
            let mut pending = Some(first);
            while let Some(message) = pending.take() {
                match message {
                    LogFileMessage::Line(line) => self.append(&line),
                    LogFileMessage::Flush(done) => {
                        let _ = self.file.flush();
                        let _ = done.send(());
                    }
                }
                pending = receiver.try_recv().ok();
            }
            let _ = self.file.flush();
        }
        let _ = self.file.flush();
    }

    fn append(&mut self, line: &[u8]) {
        let redacted = redact_private_fields(&String::from_utf8_lossy(line));
        let bytes = redacted.as_bytes();
        let length = bytes.len() as u64;
        if self.size > 0 && self.size + length > self.max_bytes {
            self.rotate();
        }
        if self.file.write_all(bytes).is_ok() {
            self.size += length;
        }
    }

    fn rotate(&mut self) {
        let _ = self.file.flush();
        if self.count == 1 {
            let _ = std::fs::remove_file(log_file_path(&self.directory, 0));
        }
        for index in (1..self.count).rev() {
            let from = log_file_path(&self.directory, index - 1);
            if from.exists() {
                let to = log_file_path(&self.directory, index);
                let _ = std::fs::remove_file(&to);
                let _ = std::fs::rename(&from, &to);
            }
        }
        if let Ok(file) = open_append(&log_file_path(&self.directory, 0)) {
            self.file = BufWriter::new(file);
            self.size = 0;
        }
    }
}

/// riseonly.log for index 0, riseonly.<index>.log for older rotated files
pub fn log_file_path(directory: &Path, index: usize) -> PathBuf {
    if index == 0 {
        directory.join(format!("{LOG_FILE_STEM}.{LOG_FILE_EXTENSION}"))
    } else {
        directory.join(format!("{LOG_FILE_STEM}.{index}.{LOG_FILE_EXTENSION}"))
    }
}

fn open_append(path: &Path) -> io::Result<File> {
    OpenOptions::new().create(true).append(true).open(path)
}

/// hides values of key=value fields that may carry tokens, contacts or message texts, and bearer tokens
pub fn redact_private_fields(line: &str) -> String {
    let mut redacted = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(equals) = rest.find('=') {
        let head = &rest[..equals];
        let name_start = head
            .bytes()
            .rposition(|byte| !is_field_name_byte(byte))
            .map_or(0, |index| index + 1);
        let value = &rest[equals + 1..];
        let value_length = field_value_length(value);
        redacted.push_str(head);
        redacted.push('=');
        if value_length > 0 && is_private_field(&head[name_start..]) {
            redacted.push_str(LOG_FILE_REDACTED);
        } else {
            redacted.push_str(&value[..value_length]);
        }
        rest = &value[value_length..];
    }
    redacted.push_str(rest);
    redact_bearer(&redacted)
}

fn is_field_name_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'.'
}

fn is_private_field(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    name.split(|character: char| character == '_' || character == '.')
        .filter(|word| !word.is_empty())
        .any(|word| {
            PRIVATE_FIELD_WORDS.contains(&word) || PRIVATE_FIELD_SUFFIXES.iter().any(|suffix| word.ends_with(suffix))
        })
}

fn field_value_length(value: &str) -> usize {
    let bytes = value.as_bytes();
    if bytes.first() == Some(&b'"') {
        let mut index = 1;
        while let Some(byte) = bytes.get(index) {
            match byte {
                b'\\' => index += 2,
                b'"' => return index + 1,
                _ => index += 1,
            }
        }
        return value.len();
    }
    value.find(char::is_whitespace).unwrap_or(value.len())
}

fn redact_bearer(line: &str) -> String {
    let mut redacted = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(start) = rest.find(BEARER_PREFIX) {
        let token_start = start + BEARER_PREFIX.len();
        redacted.push_str(&rest[..token_start]);
        let token = &rest[token_start..];
        let length = token
            .find(|character: char| character.is_whitespace() || character == '"')
            .unwrap_or(token.len());
        if length > 0 {
            redacted.push_str(LOG_FILE_REDACTED);
        }
        rest = &token[length..];
    }
    redacted.push_str(rest);
    redacted
}

#[cfg(test)]
#[path = "rise_log_file_tests.rs"]
mod tests;
