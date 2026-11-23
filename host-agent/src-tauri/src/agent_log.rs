//! The agent's own log file, on every platform.
//!
//! A Windows release build has no console, so whatever the agent printed to
//! stderr was lost and a silent stop could not be explained afterwards
//! (issue 33). Every line now also goes to `host-agent.log` in the app-config
//! directory, with a UTC timestamp. The file is rotated once it passes
//! [`MAX_LOG_BYTES`]: the previous one is kept as `host-agent.log.1`.
//! A panic is written there too before the process aborts.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

pub const LOG_FILE_NAME: &str = "host-agent.log";
pub const MAX_LOG_BYTES: u64 = 5 * 1024 * 1024;

struct Sink {
    path: PathBuf,
    file: File,
    written: u64,
}

static SINK: Mutex<Option<Sink>> = Mutex::new(None);

/// Opens the log in `dir` and logs panics there. Safe to call more than once;
/// the last directory wins. Failing to open the file only loses the file copy.
pub fn init(dir: &Path) {
    let path = dir.join(LOG_FILE_NAME);
    match open(&path) {
        Ok(sink) => {
            if let Ok(mut guard) = SINK.lock() {
                *guard = Some(sink);
            }
        }
        Err(error) => eprintln!("host-agent log: could not open {}: {error}", path.display()),
    }
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let thread = std::thread::current();
        let name = thread.name().unwrap_or("unnamed");
        line(&format!("host-agent PANIC in thread {name}: {info}"));
        previous(info);
    }));
}

fn open(path: &Path) -> std::io::Result<Sink> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    rotate_if_full(path, MAX_LOG_BYTES)?;
    let file = OpenOptions::new().create(true).append(true).open(path)?;
    let written = file.metadata().map(|meta| meta.len()).unwrap_or(0);
    Ok(Sink {
        path: path.to_path_buf(),
        file,
        written,
    })
}

fn rotate_if_full(path: &Path, max_bytes: u64) -> std::io::Result<bool> {
    let len = match std::fs::metadata(path) {
        Ok(meta) => meta.len(),
        Err(_) => return Ok(false),
    };
    if len < max_bytes {
        return Ok(false);
    }
    let old = rotated_path(path);
    let _ = std::fs::remove_file(&old);
    std::fs::rename(path, old)?;
    Ok(true)
}

fn rotated_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(".1");
    PathBuf::from(name)
}

/// Writes one line to stderr and, once [`init`] ran, to the log file.
pub fn line(message: &str) {
    let stamped = format!("{} {message}\n", utc_timestamp(SystemTime::now()));
    eprint!("{stamped}");
    let Ok(mut guard) = SINK.lock() else {
        return;
    };
    let Some(sink) = guard.as_mut() else {
        return;
    };
    if sink.written + stamped.len() as u64 > MAX_LOG_BYTES {
        if let Ok(fresh) = open_after_rotate(&sink.path) {
            *sink = fresh;
        }
    }
    if sink.file.write_all(stamped.as_bytes()).is_ok() {
        let _ = sink.file.flush();
        sink.written += stamped.len() as u64;
    }
}

fn open_after_rotate(path: &Path) -> std::io::Result<Sink> {
    let old = rotated_path(path);
    let _ = std::fs::remove_file(&old);
    std::fs::rename(path, old)?;
    open(path)
}

/// `2026-11-18T20:46:35Z`, without a date crate.
pub fn utc_timestamp(time: SystemTime) -> String {
    let secs = time
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

// Howard Hinnant's days-to-civil algorithm.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

/// `eprintln!` that also lands in the agent's log file.
#[macro_export]
macro_rules! agent_log {
    ($($arg:tt)*) => {
        $crate::agent_log::line(&format!($($arg)*))
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn timestamps_are_utc_iso() {
        let t = UNIX_EPOCH + Duration::from_secs(1_795_034_795); // 2026-11-18T20:46:35Z
        assert_eq!(utc_timestamp(t), "2026-11-18T20:46:35Z");
        assert_eq!(utc_timestamp(UNIX_EPOCH), "1970-01-01T00:00:00Z");
        let leap = UNIX_EPOCH + Duration::from_secs(951_782_400); // 2000-02-29
        assert_eq!(utc_timestamp(leap), "2000-02-29T00:00:00Z");
    }

    #[test]
    fn a_full_log_is_kept_as_dot_one() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(LOG_FILE_NAME);
        std::fs::write(&path, vec![b'x'; 20]).unwrap();
        assert!(!rotate_if_full(&path, 100).unwrap());
        assert!(rotate_if_full(&path, 10).unwrap());
        assert!(!path.exists());
        assert_eq!(std::fs::read(rotated_path(&path)).unwrap().len(), 20);
    }

    #[test]
    fn opening_appends_to_an_existing_log() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(LOG_FILE_NAME);
        std::fs::write(&path, b"before\n").unwrap();
        let mut sink = open(&path).unwrap();
        assert_eq!(sink.written, 7);
        sink.file.write_all(b"after\n").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "before\nafter\n");
    }
}
