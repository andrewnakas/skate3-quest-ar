//! The engine reports startup progress and every fatal error with
//! `eprintln!`, which Android discards. Redirect stdout and stderr into a
//! pipe and forward each line to logcat under the tag `skate3`, and route
//! panics there too, so `adb logcat -s skate3` shows what the desktop console
//! would.
use std::ffi::CString;
use std::io::{BufRead, BufReader};
use std::os::fd::FromRawFd;

const ANDROID_LOG_INFO: libc::c_int = 4;
const ANDROID_LOG_ERROR: libc::c_int = 6;

#[link(name = "log")]
unsafe extern "C" {
    fn __android_log_write(
        priority: libc::c_int,
        tag: *const libc::c_char,
        text: *const libc::c_char,
    ) -> libc::c_int;
}

fn write(priority: libc::c_int, line: &str) {
    let text = CString::new(line.replace('\0', "\\0")).unwrap_or_default();
    // SAFETY: both strings are NUL-terminated and outlive the call.
    unsafe {
        __android_log_write(priority, c"skate3".as_ptr(), text.as_ptr());
    }
}

pub(crate) fn capture_stdio() {
    let mut fds = [0 as libc::c_int; 2];
    // SAFETY: `fds` is a valid two-element array; dup2 targets are the
    // process's own stdout/stderr descriptors.
    let ok = unsafe {
        libc::pipe(fds.as_mut_ptr()) == 0
            && libc::dup2(fds[1], libc::STDOUT_FILENO) >= 0
            && libc::dup2(fds[1], libc::STDERR_FILENO) >= 0
    };
    if !ok {
        write(ANDROID_LOG_ERROR, "could not redirect stdio to logcat");
        return;
    }
    // SAFETY: the read end is owned solely by this reader from here on.
    let reader = unsafe { std::fs::File::from_raw_fd(fds[0]) };
    std::thread::Builder::new()
        .name("logcat".into())
        .spawn(move || {
            for line in BufReader::new(reader).lines() {
                match line {
                    Ok(line) => write(ANDROID_LOG_INFO, &line),
                    Err(_) => break,
                }
            }
        })
        .ok();
    std::panic::set_hook(Box::new(|info| {
        write(ANDROID_LOG_ERROR, &format!("panic: {info}"));
        write(
            ANDROID_LOG_ERROR,
            &std::backtrace::Backtrace::force_capture().to_string(),
        );
    }));
}
