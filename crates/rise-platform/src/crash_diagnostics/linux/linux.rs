use std::cell::UnsafeCell;
use std::ffi::{CString, c_char, c_int, c_long, c_uint, c_void};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::ptr;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use crate::crash_diagnostics::*;

const SC_CLK_TCK: c_int = 2;
const SC_PAGESIZE: c_int = 30;

/// statvfs of glibc and musl on 64 bit linux (x86_64 and aarch64), the only targets
/// every unsigned long and fsblkcnt_t member is 64 bits there
#[repr(C)]
#[derive(Default)]
struct StatVfs {
    f_bsize: u64,
    f_frsize: u64,
    _f_blocks: u64,
    _f_bfree: u64,
    f_bavail: u64,
    _f_files: u64,
    _f_ffree: u64,
    _f_favail: u64,
    _f_fsid: u64,
    _f_flag: u64,
    _f_namemax: u64,
    _spare: [c_int; 6],
}

const _: () = assert!(size_of::<StatVfs>() == 112, "struct statvfs layout of 64-bit Linux");

unsafe extern "C" {
    fn sysconf(name: c_int) -> c_long;
    fn statvfs(path: *const c_char, buf: *mut StatVfs) -> c_int;
    fn sigaction(signal: c_int, action: *const SigAction, previous: *mut SigAction) -> c_int;
    fn raise(signal: c_int) -> c_int;
    fn open(path: *const c_char, flags: c_int, ...) -> c_int;
    fn read(fd: c_int, buffer: *mut c_void, count: usize) -> isize;
    fn write(fd: c_int, buffer: *const c_void, count: usize) -> isize;
    fn close(fd: c_int) -> c_int;
    fn getpid() -> c_int;
    fn clock_gettime(clock: c_int, time: *mut TimeSpec) -> c_int;
    fn backtrace(buffer: *mut *mut c_void, size: c_int) -> c_int;
    fn prctl(option: c_int, ...) -> c_int;
    fn pthread_self() -> usize;
    fn pthread_kill(thread: usize, signal: c_int) -> c_int;
    fn __libc_current_sigrtmin() -> c_int;
    fn sigaltstack(stack: *const SignalStack, previous: *mut SignalStack) -> c_int;
}

const SIGILL: c_int = 4;
const SIGTRAP: c_int = 5;
const SIGABRT: c_int = 6;
const SIGBUS: c_int = 7;
const SIGFPE: c_int = 8;
const SIGSEGV: c_int = 11;
const CRASH_SIGNALS: [c_int; 6] = [SIGSEGV, SIGBUS, SIGILL, SIGFPE, SIGABRT, SIGTRAP];
const SA_SIGINFO: c_int = 4;
const SA_ONSTACK: c_int = 0x0800_0000;
const O_RDONLY: c_int = 0;
const O_WRONLY: c_int = 0o1;
const O_CREAT: c_int = 0o100;
const O_EXCL: c_int = 0o200;
const O_CLOEXEC: c_int = 0o2_000_000;
const RECORD_MODE: c_uint = 0o600;
const CLOCK_REALTIME: c_int = 0;
const PR_GET_NAME: c_int = 16;

/// struct sigaction of glibc on x86_64 and aarch64
#[repr(C)]
#[derive(Clone, Copy)]
struct SigAction {
    handler: usize,
    mask: [u64; 16],
    flags: c_int,
    restorer: usize,
}

const _: () = assert!(
    size_of::<SigAction>() == 152,
    "struct sigaction layout of 64-bit glibc"
);

impl SigAction {
    const fn default_action() -> Self {
        Self {
            handler: 0,
            mask: [0; 16],
            flags: 0,
            restorer: 0,
        }
    }
}

/// head of siginfo_t on 64 bit linux, si_addr sits after three ints and padding
#[repr(C)]
struct SigInfo {
    _signo: c_int,
    _errno: c_int,
    code: c_int,
    _padding: c_int,
    address: usize,
}

#[repr(C)]
#[derive(Default)]
struct TimeSpec {
    seconds: i64,
    nanos: i64,
}

static MARKER_PREFIX: OnceLock<Box<[u8]>> = OnceLock::new();

static PREVIOUS_ACTIONS: OnceLock<[SigAction; CRASH_SIGNALS.len()]> = OnceLock::new();

const SIGNAL_STACK_BYTES: usize = 64 * 1024;
const SS_DISABLE: c_int = 2;

#[repr(C)]
struct SignalStack {
    base: *mut c_void,
    flags: c_int,
    size: usize,
}

#[repr(C, align(16))]
struct SignalStackMemory(UnsafeCell<[u8; SIGNAL_STACK_BYTES]>);

unsafe impl Sync for SignalStackMemory {}

static MAIN_SIGNAL_STACK: SignalStackMemory = SignalStackMemory(UnsafeCell::new([0; SIGNAL_STACK_BYTES]));

struct RecordBuffers {
    path: CrashText<1024>,
    line: CrashText<256>,
    frames: [usize; NATIVE_CRASH_FRAMES],
    chunk: [u8; 4096],
    map_line: CrashText<1024>,
}

struct RecordSlot(UnsafeCell<RecordBuffers>);

unsafe impl Sync for RecordSlot {}

static RECORD_BUFFERS: RecordSlot = RecordSlot(UnsafeCell::new(RecordBuffers {
    path: CrashText::new(),
    line: CrashText::new(),
    frames: [0; NATIVE_CRASH_FRAMES],
    chunk: [0; 4096],
    map_line: CrashText::new(),
}));

static RECORD_BUFFERS_BUSY: AtomicBool = AtomicBool::new(false);

struct RecordBuffersTaken;

impl RecordBuffersTaken {
    fn take() -> Option<(Self, &'static mut RecordBuffers)> {
        if RECORD_BUFFERS_BUSY.swap(true, Ordering::SeqCst) {
            return None;
        }
        Some((Self, unsafe { &mut *RECORD_BUFFERS.0.get() }))
    }
}

impl Drop for RecordBuffersTaken {
    fn drop(&mut self) {
        RECORD_BUFFERS_BUSY.store(false, Ordering::SeqCst);
    }
}

fn widen_main_signal_stack() {
    let mut current = SignalStack { base: ptr::null_mut(), flags: 0, size: 0 };
    if unsafe { sigaltstack(ptr::null(), &raw mut current) } != 0 {
        return;
    }
    if current.flags & SS_DISABLE == 0 && current.size >= SIGNAL_STACK_BYTES {
        return;
    }
    let ours = SignalStack { base: MAIN_SIGNAL_STACK.0.get().cast(), flags: 0, size: SIGNAL_STACK_BYTES };
    unsafe { sigaltstack(&raw const ours, ptr::null_mut()) };
}

pub(crate) struct NativeCrashDiagnostics;

fn read_proc(path: &str, what: DiagnosticsReading) -> Result<String, CrashDiagnosticsUnavailable> {
    std::fs::read_to_string(path).map_err(|error| CrashDiagnosticsUnavailable::SystemRefused {
        what,
        detail: format!("{path}: {error}"),
    })
}

fn sysconf_value(
    name: c_int,
    label: &str,
    what: DiagnosticsReading,
) -> Result<u64, CrashDiagnosticsUnavailable> {
    let value = unsafe { sysconf(name) };
    u64::try_from(value)
        .ok()
        .filter(|value| *value > 0)
        .ok_or_else(|| CrashDiagnosticsUnavailable::SystemRefused {
            what,
            detail: format!("sysconf({label}) returned {value}"),
        })
}

impl CrashDiagnosticsSource for NativeCrashDiagnostics {
    fn resident_memory_bytes(&self) -> Result<u64, CrashDiagnosticsUnavailable> {
        let what = DiagnosticsReading::ResidentMemory;
        let page_size = sysconf_value(SC_PAGESIZE, "_SC_PAGESIZE", what)?;
        resident_bytes_from_statm(&read_proc("/proc/self/statm", what)?, page_size)
    }

    fn process_start_seconds_since_1970(&self) -> Result<f64, CrashDiagnosticsUnavailable> {
        let what = DiagnosticsReading::ProcessStart;
        let ticks = start_ticks_from_proc_pid_stat(&read_proc("/proc/self/stat", what)?)?;
        let boot = boot_seconds_from_proc_stat(&read_proc("/proc/stat", what)?)?;
        let ticks_per_second = sysconf_value(SC_CLK_TCK, "_SC_CLK_TCK", what)?;
        process_start_from_boot(boot, ticks, ticks_per_second)
    }

    fn available_capacity_for_important_usage(
        &self,
        path: &Path,
    ) -> Result<u64, CrashDiagnosticsUnavailable> {
        let what = DiagnosticsReading::AvailableCapacity;
        let c_path = CString::new(path.as_os_str().as_bytes()).map_err(|_| {
            CrashDiagnosticsUnavailable::SystemRefused {
                what,
                detail: format!("{} holds a NUL byte", path.display()),
            }
        })?;
        let mut info = StatVfs::default();
        let result = unsafe { statvfs(c_path.as_ptr(), &raw mut info) };
        if result != 0 {
            return Err(CrashDiagnosticsUnavailable::SystemRefused {
                what,
                detail: format!("statvfs {}: {}", path.display(), std::io::Error::last_os_error()),
            });
        }
        available_bytes_from_statvfs(info.f_bavail, info.f_frsize, info.f_bsize)
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
        let mut prefix = marker_dir.as_os_str().as_bytes().to_vec();
        prefix.push(b'/');
        if MARKER_PREFIX.set(prefix.into_boxed_slice()).is_err() {
            return Ok(());
        }
        let mut warm = [ptr::null_mut::<c_void>(); 4];
        unsafe { backtrace(warm.as_mut_ptr(), warm.len() as c_int) };
        widen_main_signal_stack();
        let mut previous = [SigAction::default_action(); CRASH_SIGNALS.len()];
        for (slot, signal) in previous.iter_mut().zip(CRASH_SIGNALS) {
            if unsafe { sigaction(signal, ptr::null(), slot) } != 0 {
                return Err(CrashDiagnosticsUnavailable::SystemRefused {
                    what,
                    detail: format!("sigaction({signal}): {}", std::io::Error::last_os_error()),
                });
            }
        }
        let _ = PREVIOUS_ACTIONS.set(previous);
        let handler: unsafe extern "C" fn(c_int, *mut SigInfo, *mut c_void) = on_crash_signal;
        let action = SigAction {
            handler: handler as usize,
            mask: [0; 16],
            flags: SA_SIGINFO | SA_ONSTACK,
            restorer: 0,
        };
        for signal in CRASH_SIGNALS {
            if unsafe { sigaction(signal, &action, ptr::null_mut()) } != 0 {
                return Err(CrashDiagnosticsUnavailable::SystemRefused {
                    what,
                    detail: format!("sigaction({signal}): {}", std::io::Error::last_os_error()),
                });
            }
        }
        Ok(())
    }

    fn install_hang_watch(&self, marker_dir: &Path) -> Result<(), CrashDiagnosticsUnavailable> {
        let what = DiagnosticsReading::HangReports;
        if MAIN_THREAD.get().is_some() {
            return Ok(());
        }
        if MARKER_PREFIX.get().is_none() {
            let mut prefix = marker_dir.as_os_str().as_bytes().to_vec();
            prefix.push(b'/');
            let _ = MARKER_PREFIX.set(prefix.into_boxed_slice());
        }
        let signal = unsafe { __libc_current_sigrtmin() } + HANG_SIGNAL_FROM_RTMIN;
        let handler: unsafe extern "C" fn(c_int, *mut SigInfo, *mut c_void) = on_hang_signal;
        let action = SigAction {
            handler: handler as usize,
            mask: [0; 16],
            flags: SA_SIGINFO | SA_RESTART,
            restorer: 0,
        };
        if unsafe { sigaction(signal, &action, ptr::null_mut()) } != 0 {
            return Err(CrashDiagnosticsUnavailable::SystemRefused {
                what,
                detail: format!("sigaction({signal}): {}", std::io::Error::last_os_error()),
            });
        }
        let _ = MAIN_THREAD.set(unsafe { pthread_self() });
        std::thread::Builder::new()
            .name("rise-hang-watch".into())
            .spawn(move || watch_main_thread(signal))
            .map(|_| ())
            .map_err(|error| CrashDiagnosticsUnavailable::Io { what, detail: error.to_string() })
    }
}

const SA_RESTART: c_int = 0x1000_0000;
const HANG_SIGNAL_FROM_RTMIN: c_int = 6;

static MAIN_THREAD: OnceLock<usize> = OnceLock::new();

static HANG_CAPTURE_WANTED: AtomicBool = AtomicBool::new(false);

/// the app beats on the main thread, a silence of the canon threshold asks that thread for its stack
fn watch_main_thread(signal: c_int) {
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
            if let Some(main) = MAIN_THREAD.get() {
                HANG_CAPTURE_WANTED.store(true, Ordering::SeqCst);
                unsafe { pthread_kill(*main, signal) };
            }
        }
        if !hung {
            reported = false;
        }
    }
}

unsafe extern "C" fn on_hang_signal(signal: c_int, info: *mut SigInfo, context: *mut c_void) {
    if HANG_CAPTURE_WANTED.swap(false, Ordering::SeqCst) {
        unsafe { record(KIND_HANG, signal, info, context) };
    }
}

unsafe extern "C" fn on_crash_signal(signal: c_int, info: *mut SigInfo, context: *mut c_void) {
    if take_native_crash_slot() {
        unsafe { record_signal(signal, info, context) };
    }
    let previous = PREVIOUS_ACTIONS
        .get()
        .and_then(|actions| {
            CRASH_SIGNALS
                .iter()
                .position(|known| *known == signal)
                .map(|index| actions[index])
        })
        .unwrap_or(SigAction::default_action());
    unsafe { sigaction(signal, &previous, ptr::null_mut()) };
    let sent = info.is_null() || unsafe { (*info).code } <= 0;
    if sent {
        unsafe { raise(signal) };
    }
}

fn emit(fd: c_int, bytes: &[u8]) {
    let mut rest = bytes;
    while !rest.is_empty() {
        let written = unsafe { write(fd, rest.as_ptr().cast(), rest.len()) };
        if written <= 0 {
            return;
        }
        rest = &rest[written as usize..];
    }
}

unsafe fn record_signal(signal: c_int, info: *mut SigInfo, context: *mut c_void) {
    unsafe { record(KIND_SIGNAL, signal, info, context) };
}

unsafe fn record(kind: &str, signal: c_int, info: *mut SigInfo, context: *mut c_void) {
    let Some(prefix) = MARKER_PREFIX.get() else {
        return;
    };
    let mut now = TimeSpec::default();
    unsafe { clock_gettime(CLOCK_REALTIME, &raw mut now) };
    let millis = now.seconds.max(0) as u64 * 1000 + now.nanos.max(0) as u64 / 1_000_000;
    let pid = unsafe { getpid() } as u32;
    let Some((_taken, buffers)) = RecordBuffersTaken::take() else {
        return;
    };
    let path = &mut buffers.path;
    path.clear();
    path.push_bytes(prefix);
    push_native_crash_name(path, millis, pid);
    path.push_bytes(&[0]);
    if path.as_bytes().last() != Some(&0) {
        return;
    }
    let flags = O_WRONLY | O_CREAT | O_EXCL | O_CLOEXEC;
    let fd = unsafe { open(path.as_bytes().as_ptr().cast(), flags, RECORD_MODE) };
    if fd < 0 {
        return;
    }
    let line = &mut buffers.line;
    emit(
        fd,
        line.clear().push_str(NATIVE_CRASH_HEADER).push_str("\n").as_bytes(),
    );
    emit(
        fd,
        line.clear()
            .push_str("kind ")
            .push_str(kind)
            .push_str("\n")
            .as_bytes(),
    );
    let crashed = kind != KIND_HANG;
    if crashed {
        emit(
            fd,
            line.clear()
                .push_str("code ")
                .push_signed(i64::from(signal))
                .push_str("\n")
                .as_bytes(),
        );
    }
    if crashed && !info.is_null() {
        let (code, address) = unsafe { ((*info).code, (*info).address) };
        emit(
            fd,
            line.clear()
                .push_str("subcode ")
                .push_signed(i64::from(code))
                .push_str("\n")
                .as_bytes(),
        );
        emit(
            fd,
            line.clear()
                .push_str("address ")
                .push_hex(address as u64)
                .push_str("\n")
                .as_bytes(),
        );
    }
    let architecture = std::env::consts::ARCH;
    if let Some(offset) = signal_context_pc_offset(architecture).filter(|_| !context.is_null()) {
        let pc = unsafe {
            context
                .cast::<u8>()
                .add(offset)
                .cast::<u64>()
                .read_unaligned()
        };
        emit(
            fd,
            line.clear()
                .push_str("pc ")
                .push_hex(pc)
                .push_str("\n")
                .as_bytes(),
        );
    }
    emit(
        fd,
        line.clear()
            .push_str("pid ")
            .push_decimal(u64::from(pid))
            .push_str("\n")
            .as_bytes(),
    );
    let mut name = [0u8; 17];
    if unsafe { prctl(PR_GET_NAME, name.as_mut_ptr()) } == 0 {
        let length = name.iter().position(|byte| *byte == 0).unwrap_or(16);
        emit(
            fd,
            line.clear()
                .push_str("thread ")
                .push_bytes(&name[..length])
                .push_str("\n")
                .as_bytes(),
        );
    }
    emit(
        fd,
        line.clear()
            .push_str("at ")
            .push_decimal(millis)
            .push_str("\n")
            .as_bytes(),
    );
    emit(
        fd,
        line.clear()
            .push_str("arch ")
            .push_str(architecture)
            .push_str("\n")
            .as_bytes(),
    );
    let frames = &mut buffers.frames;
    let count = unsafe { backtrace(frames.as_mut_ptr().cast(), frames.len() as c_int) }.max(0) as usize;
    for frame in &frames[..count.min(frames.len())] {
        emit(
            fd,
            line.clear()
                .push_str("frame ")
                .push_hex(*frame as u64)
                .push_str("\n")
                .as_bytes(),
        );
    }
    copy_kept_maps(fd, &mut buffers.chunk, &mut buffers.map_line);
    unsafe { close(fd) };
}

fn copy_kept_maps(fd: c_int, chunk: &mut [u8; 4096], line: &mut CrashText<1024>) {
    let maps = unsafe { open(c"/proc/self/maps".as_ptr(), O_RDONLY | O_CLOEXEC) };
    if maps < 0 {
        return;
    }
    line.clear();
    let mut overflowed = false;
    loop {
        let read_bytes = unsafe { read(maps, chunk.as_mut_ptr().cast(), chunk.len()) };
        if read_bytes <= 0 {
            break;
        }
        for byte in &chunk[..read_bytes as usize] {
            if *byte == b'\n' {
                if !overflowed && keep_map_line(line.as_bytes()) {
                    emit(fd, b"map ");
                    emit(fd, line.as_bytes());
                    emit(fd, b"\n");
                }
                line.clear();
                overflowed = false;
            } else if line.is_full() {
                overflowed = true;
            } else {
                line.push_bytes(std::slice::from_ref(byte));
            }
        }
    }
    unsafe { close(maps) };
}
