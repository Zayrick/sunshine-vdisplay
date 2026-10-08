//! Append-only log file shared by all invocations; also echoed to stdout
//! (which Sunshine redirects into the app's output log, if configured).

use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

use windows::Win32::System::SystemInformation::GetLocalTime;

const MAX_LOG_SIZE: u64 = 1024 * 1024;

static ECHO: AtomicBool = AtomicBool::new(true);

pub fn set_echo(echo: bool) {
    ECHO.store(echo, Ordering::Relaxed);
}

/// `vdisplay.log` next to the executable if writable, otherwise in %TEMP%.
pub fn path() -> &'static PathBuf {
    static PATH: OnceLock<PathBuf> = OnceLock::new();
    PATH.get_or_init(|| {
        let beside_exe = std::env::current_exe().ok().and_then(|exe| {
            let p = exe.with_file_name("vdisplay.log");
            OpenOptions::new()
                .create(true)
                .append(true)
                .open(&p)
                .ok()
                .map(|_| p)
        });
        beside_exe.unwrap_or_else(|| std::env::temp_dir().join("vdisplay.log"))
    })
}

pub fn write(msg: &str) {
    let t = unsafe { GetLocalTime() };
    let line = format!(
        "[{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:03}] [{}] {msg}\n",
        t.wYear,
        t.wMonth,
        t.wDay,
        t.wHour,
        t.wMinute,
        t.wSecond,
        t.wMilliseconds,
        std::process::id()
    );
    if ECHO.load(Ordering::Relaxed) {
        let _ = std::io::stdout().write_all(line.as_bytes());
    }
    let path = path();
    if std::fs::metadata(path).is_ok_and(|m| m.len() > MAX_LOG_SIZE) {
        let _ = std::fs::rename(path, path.with_extension("log.old"));
    }
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = f.write_all(line.as_bytes());
    }
}

#[macro_export]
macro_rules! log {
    ($($arg:tt)*) => {
        $crate::logging::write(&format!($($arg)*))
    };
}
