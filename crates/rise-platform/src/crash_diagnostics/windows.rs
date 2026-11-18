use std::ffi::c_void;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use std::ptr;
use std::sync::OnceLock;
use std::time::Instant;

use windows::Win32::Foundation::FILETIME;
use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
use windows::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};
use windows::core::PCWSTR;

use super::crash_diagnostics::*;

pub(super) struct NativeCrashDiagnostics;

const EXCEPTION_CONTINUE_SEARCH: i32 = 0;
const CRT_SIGABRT: i32 = 22;
const CRT_SIG_ERR: usize = usize::MAX;
const GENERIC_WRITE: u32 = 0x4000_0000;
const CREATE_NEW: u32 = 1;
const FILE_ATTRIBUTE_NORMAL: u32 = 0x80;
const MODULE_FROM_ADDRESS: u32 = 0x4 | 0x2;
const PATH_UNITS: usize = 600;
const MODULE_NAME_UNITS: usize = 520;

#[repr(C)]
struct ExceptionRecord {
    code: u32,
    _flags: u32,
    _chained: *const ExceptionRecord,
    address: *const c_void,
    parameter_count: u32,
    information: [usize; 15],
}

#[repr(C)]
struct ExceptionPointers {
    record: *const ExceptionRecord,
    _context: *const c_void,
}

type ExceptionFilter = unsafe extern "system" fn(*const ExceptionPointers) -> i32;

#[repr(C)]
#[derive(Default)]
struct FileTimeHalves {
    low: u32,
    high: u32,
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn SetUnhandledExceptionFilter(filter: Option<ExceptionFilter>) -> Option<ExceptionFilter>;
    fn RtlCaptureStackBackTrace(
        skip: u32,
        count: u32,
        frames: *mut *mut c_void,
        hash: *mut u32,
    ) -> u16;
    fn GetModuleHandleExW(flags: u32, name: *const u16, module: *mut *mut c_void) -> i32;
    fn GetModuleFileNameW(module: *mut c_void, name: *mut u16, size: u32) -> u32;
    fn CreateFileW(
        name: *const u16,
        access: u32,
        share: u32,
        security: *const c_void,
        disposition: u32,
        flags: u32,
        template: *mut c_void,
    ) -> *mut c_void;
    fn WriteFile(
        file: *mut c_void,
        buffer: *const u8,
        length: u32,
        written: *mut u32,
        overlapped: *mut c_void,
    ) -> i32;
    fn CloseHandle(handle: *mut c_void) -> i32;
    fn GetSystemTimeAsFileTime(time: *mut FileTimeHalves);
    fn GetCurrentProcessId() -> u32;
    fn GetCurrentThreadId() -> u32;
}

unsafe extern "C" {
    fn signal(signal: i32, handler: usize) -> usize;
}

static MARKER_PREFIX: OnceLock<Box<[u16]>> = OnceLock::new();

static PREVIOUS_FILTER: OnceLock<Option<ExceptionFilter>> = OnceLock::new();

fn refused(what: DiagnosticsReading, error: &windows::core::Error) -> CrashDiagnosticsUnavailable {
    CrashDiagnosticsUnavailable::SystemRefused {
        what,
        detail: error.to_string(),
    }
}

impl CrashDiagnosticsSource for NativeCrashDiagnostics {
    fn resident_memory_bytes(&self) -> Result<u64, CrashDiagnosticsUnavailable> {
        let mut counters = PROCESS_MEMORY_COUNTERS {
            cb: size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
            ..Default::default()
        };
        unsafe {
            GetProcessMemoryInfo(
                GetCurrentProcess(),
                &mut counters,
                size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
            )
        }
        .map_err(|error| refused(DiagnosticsReading::ResidentMemory, &error))?;
        Ok(counters.WorkingSetSize as u64)
    }

    fn process_start_seconds_since_1970(&self) -> Result<f64, CrashDiagnosticsUnavailable> {
        let mut creation = FILETIME::default();
        let mut exit = FILETIME::default();
        let mut kernel = FILETIME::default();
        let mut user = FILETIME::default();
        unsafe {
            GetProcessTimes(
                GetCurrentProcess(),
                &mut creation,
                &mut exit,
                &mut kernel,
                &mut user,
            )
        }
        .map_err(|error| refused(DiagnosticsReading::ProcessStart, &error))?;
        process_start_from_filetime(creation.dwHighDateTime, creation.dwLowDateTime)
    }

    fn available_capacity_for_important_usage(
        &self,
        path: &Path,
    ) -> Result<u64, CrashDiagnosticsUnavailable> {
        let wide: Vec<u16> = path
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let mut available: u64 = 0;
        unsafe {
            GetDiskFreeSpaceExW(
                PCWSTR(wide.as_ptr()),
                Some(&mut available as *mut u64),
                None,
                None,
            )
        }
        .map_err(|error| refused(DiagnosticsReading::AvailableCapacity, &error))?;
        Ok(available)
    }

    fn install_native_crash_handler(
        &self,
        marker_dir: &Path,
    ) -> Result<(), CrashDiagnosticsUnavailable> {
        let what = DiagnosticsReading::NativeCrashes;
        std::fs::create_dir_all(marker_dir).map_err(|error| CrashDiagnosticsUnavailable::Io {
            what,
            detail: format!("{}: {error}", marker_dir.display()),
        })?;
        let mut prefix: Vec<u16> = marker_dir.as_os_str().encode_wide().collect();
        prefix.push(u16::from(b'\\'));
        if MARKER_PREFIX.set(prefix.into_boxed_slice()).is_err() {
            return Ok(());
        }
        harvest_wer_reports(marker_dir);
        let previous = unsafe { SetUnhandledExceptionFilter(Some(on_unhandled_exception)) };
        let _ = PREVIOUS_FILTER.set(previous);
        let abort_handler: extern "C" fn(i32) = on_abort_signal;
        if unsafe { signal(CRT_SIGABRT, abort_handler as usize) } == CRT_SIG_ERR {
            return Err(CrashDiagnosticsUnavailable::SystemRefused {
                what,
                detail: "the C runtime refused a SIGABRT handler".to_owned(),
            });
        }
        Ok(())
    }

    fn install_hang_watch(&self, marker_dir: &Path) -> Result<(), CrashDiagnosticsUnavailable> {
        static STARTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
        if STARTED.swap(true, std::sync::atomic::Ordering::SeqCst) {
            return Ok(());
        }
        Self::start_hang_watch(marker_dir)
    }
}

fn harvest_wer_reports(marker_dir: &Path) {
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let Some(exe_name) = exe.file_name().map(|name| name.to_string_lossy().to_ascii_lowercase()) else {
        return;
    };
    let prefix = format!("appcrash_{exe_name}_");
    let mut reports = Vec::new();
    for root in ["ProgramData", "LOCALAPPDATA"].into_iter().filter_map(std::env::var_os) {
        for folder in ["ReportArchive", "ReportQueue"] {
            let Ok(entries) = std::fs::read_dir(Path::new(&root).join("Microsoft\\Windows\\WER").join(folder)) else {
                continue;
            };
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.to_ascii_lowercase().starts_with(&prefix)
                    && let Some(text) = read_report_text(&entry.path().join("Report.wer"))
                {
                    reports.push((name, text));
                }
            }
        }
    }
    let seen_path = marker_dir.join(WER_SEEN_FILE);
    let seen: Vec<String> =
        std::fs::read_to_string(&seen_path).unwrap_or_default().lines().map(str::to_owned).collect();
    let recorded_pids: Vec<u32> = [pending_native_crash_records(marker_dir), pending_panic_markers(marker_dir)]
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|path| MarkerStamp::of(&path))
        .map(|stamp| stamp.pid)
        .collect();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| u64::try_from(since.as_millis()).unwrap_or(u64::MAX));
    let (chosen, still_seen) = select_wer_crashes(&reports, &seen, &recorded_pids, &exe.to_string_lossy(), now);
    for (_, crash) in &chosen {
        let mut name: CrashText<64> = CrashText::new();
        push_native_crash_name(&mut name, crash.at_millis.unwrap_or(now), crash.pid.unwrap_or(0));
        let path = marker_dir.join(String::from_utf8_lossy(name.as_bytes()).as_ref());
        if let Err(error) = std::fs::write(&path, crash.record_text(std::env::consts::ARCH)) {
            tracing::warn!(target: "riseonly", path = %path.display(), %error, "wer crash record not written");
        }
    }
    if still_seen != seen {
        let _ = std::fs::write(&seen_path, still_seen.join("\n"));
    }
}

fn read_report_text(path: &Path) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    match bytes.strip_prefix(&[0xFF, 0xFE]) {
        Some(wide) => {
            let units = wide.chunks_exact(2).map(|pair| u16::from_le_bytes([pair[0], pair[1]]));
            Some(char::decode_utf16(units).map(|unit| unit.unwrap_or(char::REPLACEMENT_CHARACTER)).collect())
        }
        None => Some(String::from_utf8_lossy(&bytes).into_owned()),
    }
}

unsafe extern "system" fn on_unhandled_exception(pointers: *const ExceptionPointers) -> i32 {
    if take_native_crash_slot() {
        unsafe { record_exception(pointers) };
    }
    match PREVIOUS_FILTER.get().copied().flatten() {
        Some(previous) => unsafe { previous(pointers) },
        None => EXCEPTION_CONTINUE_SEARCH,
    }
}

extern "C" fn on_abort_signal(_signal: i32) {
    if take_native_crash_slot() {
        unsafe { record_abort() };
    }
}

/// open crash record file, closed on drop
struct RecordFile {
    handle: *mut c_void,
    millis: u64,
    pid: u32,
}

impl RecordFile {
    unsafe fn create() -> Option<Self> {
        let prefix = MARKER_PREFIX.get()?;
        let mut time = FileTimeHalves::default();
        unsafe { GetSystemTimeAsFileTime(&raw mut time) };
        let ticks = (u64::from(time.high) << 32) | u64::from(time.low);
        let millis = ticks.saturating_sub(FILETIME_UNIX_EPOCH) / 10_000;
        let pid = unsafe { GetCurrentProcessId() };
        let mut name: CrashText<64> = CrashText::new();
        push_native_crash_name(&mut name, millis, pid);
        let name = name.as_bytes();
        let mut path = [0u16; PATH_UNITS];
        if prefix.len() + name.len() >= path.len() {
            return None;
        }
        path[..prefix.len()].copy_from_slice(prefix);
        for (unit, byte) in path[prefix.len()..].iter_mut().zip(name) {
            *unit = u16::from(*byte);
        }
        let handle = unsafe {
            CreateFileW(
                path.as_ptr(),
                GENERIC_WRITE,
                0,
                ptr::null(),
                CREATE_NEW,
                FILE_ATTRIBUTE_NORMAL,
                ptr::null_mut(),
            )
        };
        if handle.is_null() || handle as isize == -1 {
            return None;
        }
        Some(Self {
            handle,
            millis,
            pid,
        })
    }

    fn emit(&self, bytes: &[u8]) {
        let mut written = 0u32;
        let _ = unsafe {
            WriteFile(
                self.handle,
                bytes.as_ptr(),
                bytes.len() as u32,
                &raw mut written,
                ptr::null_mut(),
            )
        };
    }

    fn emit_head(&self, kind: &str, line: &mut CrashText<256>) {
        self.emit(
            line.clear()
                .push_str(NATIVE_CRASH_HEADER)
                .push_str("\n")
                .as_bytes(),
        );
        self.emit(
            line.clear()
                .push_str("kind ")
                .push_str(kind)
                .push_str("\n")
                .as_bytes(),
        );
    }

    fn emit_tail(&self, line: &mut CrashText<256>) {
        self.emit_tail_for(line, unsafe { GetCurrentThreadId() });
    }

    fn emit_tail_for(&self, line: &mut CrashText<256>, thread: u32) {
        self.emit(
            line.clear()
                .push_str("pid ")
                .push_decimal(u64::from(self.pid))
                .push_str("\n")
                .as_bytes(),
        );
        self.emit(
            line.clear()
                .push_str("thread ")
                .push_decimal(u64::from(thread))
                .push_str("\n")
                .as_bytes(),
        );
        self.emit(
            line.clear()
                .push_str("at ")
                .push_decimal(self.millis)
                .push_str("\n")
                .as_bytes(),
        );
        self.emit(
            line.clear()
                .push_str("arch ")
                .push_str(std::env::consts::ARCH)
                .push_str("\n")
                .as_bytes(),
        );
    }

    fn emit_stack(&self, line: &mut CrashText<256>, extra_base: usize) {
        let mut frames = [ptr::null_mut::<c_void>(); NATIVE_CRASH_FRAMES];
        let count = unsafe {
            RtlCaptureStackBackTrace(0, frames.len() as u32, frames.as_mut_ptr(), ptr::null_mut())
        };
        let mut addresses = [0u64; NATIVE_CRASH_FRAMES];
        for (address, frame) in addresses.iter_mut().zip(&frames[..usize::from(count).min(frames.len())]) {
            *address = *frame as u64;
        }
        self.emit_frames(line, &addresses[..usize::from(count).min(frames.len())], extra_base);
    }

    /// frames with the base of their module, then one line per module with its file name
    fn emit_frames(&self, line: &mut CrashText<256>, frames: &[u64], extra_base: usize) {
        let mut bases = [0usize; NATIVE_CRASH_FRAMES + 1];
        let mut known = 0;
        if extra_base != 0 {
            bases[0] = extra_base;
            known = 1;
        }
        for frame in frames {
            let base = module_base(*frame as usize);
            line.clear().push_str("frame ").push_hex(*frame);
            if base != 0 {
                line.push_str(" ").push_hex(base as u64);
                if !bases[..known].contains(&base) && known < bases.len() {
                    bases[known] = base;
                    known += 1;
                }
            }
            self.emit(line.push_str("\n").as_bytes());
        }
        let mut name = [0u16; MODULE_NAME_UNITS];
        let mut module_line: CrashText<1600> = CrashText::new();
        for base in &bases[..known] {
            let length = unsafe {
                GetModuleFileNameW(*base as *mut c_void, name.as_mut_ptr(), name.len() as u32)
            } as usize;
            module_line
                .clear()
                .push_str("module ")
                .push_hex(*base as u64)
                .push_str(" ");
            module_line
                .push_utf16(&name[..length.min(name.len())])
                .push_str("\n");
            self.emit(module_line.as_bytes());
        }
    }
}

impl Drop for RecordFile {
    fn drop(&mut self) {
        let _ = unsafe { CloseHandle(self.handle) };
    }
}

fn module_base(address: usize) -> usize {
    let mut module = ptr::null_mut::<c_void>();
    let found =
        unsafe { GetModuleHandleExW(MODULE_FROM_ADDRESS, address as *const u16, &raw mut module) };
    if found == 0 { 0 } else { module as usize }
}

unsafe fn record_exception(pointers: *const ExceptionPointers) {
    let Some(file) = (unsafe { RecordFile::create() }) else {
        return;
    };
    let mut line: CrashText<256> = CrashText::new();
    file.emit_head(KIND_EXCEPTION, &mut line);
    let record = if pointers.is_null() {
        ptr::null()
    } else {
        unsafe { (*pointers).record }
    };
    let mut pc_base = 0;
    if !record.is_null() {
        let record = unsafe { &*record };
        file.emit(
            line.clear()
                .push_str("code ")
                .push_hex(u64::from(record.code))
                .push_str("\n")
                .as_bytes(),
        );
        let parameters = (record.parameter_count as usize).min(record.information.len());
        if parameters >= 1 {
            let operation = record.information[0] as i64;
            file.emit(
                line.clear()
                    .push_str("subcode ")
                    .push_signed(operation)
                    .push_str("\n")
                    .as_bytes(),
            );
        }
        if parameters >= 2 {
            let address = record.information[1] as u64;
            file.emit(
                line.clear()
                    .push_str("address ")
                    .push_hex(address)
                    .push_str("\n")
                    .as_bytes(),
            );
        }
        let pc = record.address as usize;
        pc_base = module_base(pc);
        line.clear().push_str("pc ").push_hex(pc as u64);
        if pc_base != 0 {
            line.push_str(" ").push_hex(pc_base as u64);
        }
        file.emit(line.push_str("\n").as_bytes());
    }
    file.emit_tail(&mut line);
    file.emit_stack(&mut line, pc_base);
}

unsafe fn record_abort() {
    let Some(file) = (unsafe { RecordFile::create() }) else {
        return;
    };
    let mut line: CrashText<256> = CrashText::new();
    file.emit_head(KIND_ABORT, &mut line);
    file.emit(
        line.clear()
            .push_str("code ")
            .push_decimal(CRT_SIGABRT as u64)
            .push_str("\n")
            .as_bytes(),
    );
    file.emit_tail(&mut line);
    file.emit_stack(&mut line, 0);
}

const THREAD_SUSPEND_RESUME: u32 = 0x2;
const THREAD_GET_CONTEXT: u32 = 0x8;
const THREAD_QUERY_INFORMATION: u32 = 0x40;
const CONTEXT_FULL_AMD64: u32 = 0x0010_000B;
const CONTEXT_FLAGS_OFFSET: usize = 0x30;
const CONTEXT_RSP_OFFSET: usize = 0x98;
const CONTEXT_RIP_OFFSET: usize = 0xF8;

#[link(name = "kernel32")]
unsafe extern "system" {
    fn OpenThread(access: u32, inherit: i32, thread: u32) -> *mut c_void;
    fn SuspendThread(thread: *mut c_void) -> u32;
    fn ResumeThread(thread: *mut c_void) -> u32;
    fn GetThreadContext(thread: *mut c_void, context: *mut AmdContext) -> i32;
    fn RtlLookupFunctionEntry(pc: u64, image_base: *mut u64, history: *mut c_void) -> *mut c_void;
    fn RtlVirtualUnwind(
        handler_type: u32,
        image_base: u64,
        pc: u64,
        entry: *mut c_void,
        context: *mut AmdContext,
        handler_data: *mut *mut c_void,
        establisher: *mut u64,
        pointers: *mut c_void,
    ) -> *mut c_void;
}

/// x64 CONTEXT as raw bytes, only flags, rsp and rip are read or written by name
#[repr(C, align(16))]
struct AmdContext([u8; 1232]);

impl AmdContext {
    fn full() -> Self {
        let mut context = Self([0; 1232]);
        context.0[CONTEXT_FLAGS_OFFSET..CONTEXT_FLAGS_OFFSET + 4].copy_from_slice(&CONTEXT_FULL_AMD64.to_le_bytes());
        context
    }

    fn read(&self, offset: usize) -> u64 {
        let mut bytes = [0u8; 8];
        bytes.copy_from_slice(&self.0[offset..offset + 8]);
        u64::from_le_bytes(bytes)
    }

    fn write(&mut self, offset: usize, value: u64) {
        self.0[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
    }
}

struct MainThread {
    id: u32,
    handle: usize,
}

impl NativeCrashDiagnostics {
    fn start_hang_watch(marker_dir: &Path) -> Result<(), CrashDiagnosticsUnavailable> {
        let what = DiagnosticsReading::HangReports;
        if MARKER_PREFIX.get().is_none() {
            let mut prefix: Vec<u16> = marker_dir.as_os_str().encode_wide().collect();
            prefix.push(u16::from(b'\\'));
            let _ = MARKER_PREFIX.set(prefix.into_boxed_slice());
        }
        let id = unsafe { GetCurrentThreadId() };
        let handle = unsafe { OpenThread(THREAD_SUSPEND_RESUME | THREAD_GET_CONTEXT | THREAD_QUERY_INFORMATION, 0, id) };
        if handle.is_null() {
            return Err(CrashDiagnosticsUnavailable::SystemRefused {
                what,
                detail: format!("OpenThread: {}", std::io::Error::last_os_error()),
            });
        }
        let main = MainThread { id, handle: handle as usize };
        std::thread::Builder::new()
            .name("rise-hang-watch".into())
            .spawn(move || watch_main_thread(&main))
            .map(|_| ())
            .map_err(|error| CrashDiagnosticsUnavailable::Io { what, detail: error.to_string() })
    }
}

fn watch_main_thread(main: &MainThread) {
    let mut reported = false;
    let mut last_wake = Instant::now();
    let mut awake_since = last_wake;
    loop {
        std::thread::sleep(main_thread_beat_interval());
        let now = Instant::now();
        if is_wake_gap(now - last_wake) {
            awake_since = now;
        }
        last_wake = now;
        let hung = is_hang_while_awake(main_thread_silence(), now - awake_since);
        if hung && !reported {
            reported = true;
            record_hang(main);
        }
        if !hung {
            reported = false;
        }
    }
}

fn record_hang(main: &MainThread) {
    let handle = main.handle as *mut c_void;
    let mut frames = [0u64; NATIVE_CRASH_FRAMES];
    let mut count = 0;
    if std::env::consts::ARCH == "x86_64" && unsafe { SuspendThread(handle) } != u32::MAX {
        let mut context = AmdContext::full();
        if unsafe { GetThreadContext(handle, &raw mut context) } != 0 {
            count = unsafe { walk_suspended(&mut context, &mut frames) };
        }
        unsafe { ResumeThread(handle) };
    }
    let Some(file) = (unsafe { RecordFile::create() }) else {
        return;
    };
    let mut line: CrashText<256> = CrashText::new();
    file.emit_head(KIND_HANG, &mut line);
    if let Some(pc) = frames[..count].first() {
        let base = module_base(*pc as usize);
        line.clear().push_str("pc ").push_hex(*pc);
        if base != 0 {
            line.push_str(" ").push_hex(base as u64);
        }
        file.emit(line.push_str("\n").as_bytes());
    }
    file.emit_tail_for(&mut line, main.id);
    file.emit_frames(&mut line, &frames[..count], 0);
}

/// unwinds a suspended thread from its context without allocating, the thread may hold the heap lock
unsafe fn walk_suspended(context: &mut AmdContext, frames: &mut [u64]) -> usize {
    let mut count = 0;
    while count < frames.len() {
        let pc = context.read(CONTEXT_RIP_OFFSET);
        if pc == 0 {
            break;
        }
        frames[count] = pc;
        count += 1;
        let mut image_base = 0u64;
        let entry = unsafe { RtlLookupFunctionEntry(pc, &raw mut image_base, ptr::null_mut()) };
        if entry.is_null() {
            let rsp = context.read(CONTEXT_RSP_OFFSET);
            if rsp == 0 || rsp % 8 != 0 {
                break;
            }
            let return_address = unsafe { *(rsp as *const u64) };
            context.write(CONTEXT_RIP_OFFSET, return_address);
            context.write(CONTEXT_RSP_OFFSET, rsp + 8);
        } else {
            let mut handler_data = ptr::null_mut();
            let mut establisher = 0u64;
            unsafe {
                RtlVirtualUnwind(
                    0,
                    image_base,
                    pc,
                    entry,
                    context,
                    &raw mut handler_data,
                    &raw mut establisher,
                    ptr::null_mut(),
                )
            };
        }
    }
    count
}
