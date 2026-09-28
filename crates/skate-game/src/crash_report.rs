//! Process supervision uses only std; no renderer or game assets are initialized here.
use std::{
    collections::VecDeque,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const CHILD: &str = "SKATE_REPORT_CHILD";
const LINE_LIMIT: usize = 4096;
const LOG_LIMIT: usize = 256;
#[cfg(windows)]
#[path = "crash_native.rs"]
mod native;

#[derive(Default)]
struct Capture {
    logs: VecDeque<String>,
    transitions: VecDeque<String>,
    metadata: std::collections::BTreeMap<String, String>,
    panic: VecDeque<String>,
}
impl Capture {
    fn line(&mut self, stream: &str, line: &[u8], elapsed: f64) {
        let line = sanitize(&String::from_utf8_lossy(line));
        // Advanced capture status must reach the launch console even while the
        // supervisor keeps normal logs bounded and private. Sanitize first.
        if line.starts_with("TRACE ") {
            eprintln!("{line}");
        }
        let entry = format!("+{elapsed:.3}s {stream}: {line}");
        if let Some(value) = line.strip_prefix("REPORT_META ") {
            if let Some((key, _)) = value.split_once('=') {
                // A fixed key vocabulary prevents arbitrary diagnostic accumulation.
                if ["startup", "stage", "gpu", "state", "graphics", "network"].contains(&key) {
                    self.metadata.insert(key.into(), entry.clone());
                }
            }
        }
        if line.starts_with("REPORT_TRANSITION ") {
            push(&mut self.transitions, entry.clone(), 64);
        }
        if line.starts_with("REPORT_PANIC ") || line.starts_with("REPORT_NATIVE ") {
            push(&mut self.panic, entry.clone(), 128);
        }
        push(&mut self.logs, entry, LOG_LIMIT);
    }
}
fn push(queue: &mut VecDeque<String>, value: String, limit: usize) {
    if queue.len() == limit {
        queue.pop_front();
    }
    queue.push_back(value);
}

/// Fail closed for credential/network messages. Never collect args or environment dumps.
fn sanitize(value: &str) -> String {
    let lower = value.to_ascii_lowercase();
    if [
        "token",
        "ticket",
        "password",
        "secret",
        "authorization",
        "cookie",
        "session",
        "steam",
        "join_code",
        "host_code",
        "peer=",
        "address=",
        "lobby=",
        "owner=",
        "appearance",
        "unknown argument",
    ]
    .iter()
    .any(|key| lower.contains(key))
    {
        return "[sensitive-context line omitted]".into();
    }
    // Omit the entire line when it contains absolute paths (including paths with spaces).
    // Keep source locations portable by reporting relative paths from our panic hook.
    if value
        .as_bytes()
        .windows(3)
        .any(|w| w[0].is_ascii_alphabetic() && w[1] == b':' && (w[2] == b'\\' || w[2] == b'/'))
        || value.contains("\\\\")
        || value
            .split_whitespace()
            .any(|s| s.trim_start_matches(['\'', '"', '(']).starts_with('/'))
    {
        return "[absolute-path line omitted]".into();
    }
    value
        .chars()
        .filter(|c| !c.is_control() || *c == '\t')
        .take(LINE_LIMIT)
        .collect()
}

fn reader(
    mut input: impl Read + Send + 'static,
    stream: &'static str,
    capture: Arc<Mutex<Capture>>,
    start: Instant,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let mut buffer = [0; 4096];
        let mut line = Vec::with_capacity(LINE_LIMIT);
        let mut truncated = false;
        loop {
            let count = match input.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(n) => n,
            };
            for &byte in &buffer[..count] {
                if byte == b'\n' {
                    if truncated {
                        line.clear();
                        line.extend_from_slice(b"[oversized line omitted]");
                    }
                    capture.lock().unwrap_or_else(|e| e.into_inner()).line(
                        stream,
                        &line,
                        start.elapsed().as_secs_f64(),
                    );
                    line.clear();
                    truncated = false;
                } else if line.len() < LINE_LIMIT {
                    line.push(byte);
                } else {
                    truncated = true;
                }
            }
            // Preserve existing launcher logs. Capture happens before forwarding.
            if stream == "stderr" {
                let _ = std::io::stderr().write_all(&buffer[..count]);
            } else {
                let _ = std::io::stdout().write_all(&buffer[..count]);
            }
        }
        if truncated {
            line.clear();
            line.extend_from_slice(b"[oversized line omitted]");
        }
        if !line.is_empty() {
            capture.lock().unwrap_or_else(|e| e.into_inner()).line(
                stream,
                &line,
                start.elapsed().as_secs_f64(),
            );
        }
    })
}

pub(crate) fn entry() -> Option<i32> {
    if std::env::var_os(CHILD).is_some() {
        // Entry runs before game threads. Do not let setup/relay descendants
        // accidentally pass the supervisor bypass marker to a later game launch.
        unsafe {
            std::env::remove_var(CHILD);
        }
        #[cfg(windows)]
        native::install();
        install_panic_hook();
        return None;
    }
    if std::env::args_os().nth(1).as_deref() == Some(std::ffi::OsStr::new("--crash-report-preview"))
    {
        present("Skate 3 diagnostic UI preview\nSynthetic report only; no game was started.\n");
        return Some(0);
    }
    let capture = Arc::new(Mutex::new(Capture::default()));
    let start = Instant::now();
    let result = std::env::current_exe().and_then(|exe| {
        Command::new(exe)
            .args(std::env::args_os().skip(1))
            .env(CHILD, "1")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
    });
    let mut child = match result {
        Ok(child) => child,
        Err(error) => {
            present(&report(
                &Capture::default(),
                &format!("Could not start game: {}", sanitize(&error.to_string())),
                0.,
            ));
            return Some(1);
        }
    };
    let out = reader(
        child.stdout.take().unwrap(),
        "stdout",
        capture.clone(),
        start,
    );
    let err = reader(
        child.stderr.take().unwrap(),
        "stderr",
        capture.clone(),
        start,
    );
    let status = child.wait();
    // A relay may inherit a pipe. Never wait indefinitely for its EOF.
    let deadline = Instant::now() + Duration::from_secs(2);
    while !(out.is_finished() && err.is_finished()) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    let code = match &status {
        Ok(s) => s.code().unwrap_or(1),
        Err(_) => 1,
    };
    if !status.as_ref().is_ok_and(|s| s.success()) {
        let outcome = match status {
            Ok(s) => format!(
                "Process status: {s}; exit code hex: 0x{:08X}. Native exception interpretation is platform-dependent.",
                code as u32
            ),
            Err(e) => format!("Process wait failed: {}", sanitize(&e.to_string())),
        };
        let text = report(
            &capture.lock().unwrap_or_else(|e| e.into_inner()),
            &outcome,
            start.elapsed().as_secs_f64(),
        );
        present(&text);
    }
    Some(code)
}

fn install_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        let payload = info
            .payload()
            .downcast_ref::<String>()
            .map(String::as_str)
            .or_else(|| info.payload().downcast_ref::<&str>().copied())
            .unwrap_or("non-string panic payload");
        eprintln!("REPORT_PANIC {}", sanitize(payload));
        if let Some(location) = info.location() {
            let file = location.file().replace('\\', "/");
            let portable = file
                .find("crates/")
                .map(|i| &file[i..])
                .unwrap_or_else(|| file.rsplit('/').next().unwrap_or("unknown"));
            eprintln!(
                "REPORT_PANIC at {portable}:{}:{}",
                location.line(),
                location.column()
            );
        }
        // Only walk stacks on a panic. The independent supervisor owns the popup.
        for line in std::backtrace::Backtrace::force_capture()
            .to_string()
            .lines()
            .take(120)
        {
            eprintln!("REPORT_PANIC {}", sanitize(line));
        }
    }));
}

fn report(capture: &Capture, outcome: &str, elapsed: f64) -> String {
    #[cfg(windows)]
    let os_version = native::os_version();
    #[cfg(not(windows))]
    let os_version = "OS version unavailable";
    let mut text = format!(
        "Skate 3 Rust Engine diagnostic report v1\nBuild: {}\nPlatform: {} / {}\nUTC Unix seconds: {}\nRuntime seconds: {elapsed:.3}\n{outcome}\n\nNo automatic upload. Review before sharing. Paths and sensitive-context log lines are omitted.\nNative fault stack/registers: unavailable (no memory dump collected).\nGPU/driver, map and settings: available only if initialized and recorded below.\nMods: no authoritative mod inventory; modified asset contents are not collected.\nLogs: last 256 bounded lines; transitions: last 64; panic: last 128 lines.\nState is sampled every second; brief transitions can be missed. Abrupt exits can lose pending pipe data.\n",
        env!("SKATE_BUILD_ID"),
        std::env::consts::OS,
        std::env::consts::ARCH,
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    );
    text.push_str(&format!("OS version: {os_version}\n"));
    for (name, entries) in [
        (
            "Latest metadata",
            capture.metadata.values().collect::<Vec<_>>(),
        ),
        (
            "Recent state transitions",
            capture.transitions.iter().collect(),
        ),
        (
            "Panic, native exception and stack",
            capture.panic.iter().collect(),
        ),
        ("Recent logs", capture.logs.iter().collect()),
    ] {
        text.push_str(&format!("\n{name}\n"));
        if entries.is_empty() {
            text.push_str("Unavailable / not recorded\n");
        }
        for entry in entries {
            text.push_str(entry);
            text.push('\n');
        }
    }
    text
}

fn save(text: &str) -> std::io::Result<PathBuf> {
    let roots = [
        std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir),
        std::env::temp_dir(),
    ];
    let mut error = None;
    for root in roots {
        let result = (|| {
            let folder = root.join("Skate3RustEngine/CrashReports");
            std::fs::create_dir_all(&folder)?;
            let stamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();
            let path = folder.join(format!("report-{stamp}-{}.txt", std::process::id()));
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)?;
            file.write_all(text.as_bytes())?;
            Ok(path)
        })();
        match result {
            Ok(path) => return Ok(path),
            Err(e) => error = Some(e),
        }
    }
    Err(error.unwrap())
}
fn present(text: &str) {
    match save(text) {
        Ok(path) => {
            eprintln!("Crash report saved: {}", path.display());
            if let Err(error) = popup(&path) {
                eprintln!(
                    "Report popup unavailable: {error}. Open the saved text report manually."
                );
                #[cfg(windows)]
                native::fallback_notice(&format!(
                    "The game stopped. The report popup could not start.\nSaved report: {}",
                    path.display()
                ));
            }
        }
        Err(error) => {
            eprintln!("Could not save crash report: {error}\n{text}");
            #[cfg(windows)]
            native::fallback_notice(
                "The game stopped, but its diagnostic report could not be saved. Check free disk space and directory permissions. Details were sent to the launcher log.",
            );
        }
    }
}
fn popup(path: &Path) -> std::io::Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let status = Command::new("powershell.exe")
            .args([
                "-NoProfile",
                "-STA",
                "-NonInteractive",
                "-WindowStyle",
                "Hidden",
                "-Command",
                include_str!("crash_report_ui.ps1"),
            ])
            .env("SKATE_REPORT_PATH", path)
            .creation_flags(0x08000000)
            .status()?;
        if !status.success() {
            return Err(std::io::Error::other("PowerShell UI failed"));
        }
    }
    #[cfg(not(windows))]
    {
        let _ = path;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_and_private() {
        let mut c = Capture::default();
        for i in 0..10000 {
            c.line("stderr", format!("REPORT_TRANSITION {i}").as_bytes(), 0.);
        }
        assert_eq!(c.logs.len(), LOG_LIMIT);
        assert_eq!(c.transitions.len(), 64);
        for secret in [
            "ticket abc",
            "SESSION=123",
            "C:\\Users\\Some Person\\file",
            "/home/name/a",
            "Unknown argument abc",
        ] {
            assert!(sanitize(secret).contains("omitted"));
        }
        assert_eq!(
            sanitize("panic at crates/skate-game/src/main.rs:12"),
            "panic at crates/skate-game/src/main.rs:12"
        );
        assert!(sanitize(&"x".repeat(100000)).len() <= LINE_LIMIT);
    }

    #[test]
    fn failure_evidence_survives_log_flood_and_missing_data_is_explicit() {
        let mut capture = Capture::default();
        capture.line("stderr", b"REPORT_META stage=stock_graphs", 1.);
        capture.line("stderr", b"REPORT_PANIC bad index", 2.);
        capture.line(
            "stderr",
            b"REPORT_NATIVE code=0xC0000005 fault_pc=0x1234",
            2.,
        );
        for _ in 0..1000 {
            capture.line("stdout", b"ordinary log", 3.);
        }
        let text = report(&capture, "exit=1", 4.);
        assert!(text.contains("stage=stock_graphs"));
        assert!(text.contains("bad index"));
        assert!(text.contains("fault_pc=0x1234"));
        assert!(text.contains("Unavailable / not recorded"));
        assert!(!report(&Capture::default(), "exit=1", 0.).contains("stage=stock_graphs"));
    }
}
