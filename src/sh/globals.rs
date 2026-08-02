use std::collections::VecDeque;
use std::env;
use std::ffi::{CStr, CString};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{LazyLock, Mutex};

// ANSI escape sequences
pub const ANSI_CYAN: &str = "\x1b[36m";
pub const ANSI_GREEN: &str = "\x1b[32m";
pub const ANSI_RED: &str = "\x1b[31m";
pub const ANSI_RESET: &str = "\x1b[0m";

// Limits
pub const DEFAULT_HIST_LIMIT: usize = 500;
pub const MAX_FUNCTION_DEPTH: usize = 1000;
pub const MAX_SUBSTITUTION_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_CHILD_PROCESSES: usize = 256;
pub const MAX_GLOB_ITERATIONS: usize = 1_000_000;
pub const MAX_GLOB_RESULTS: usize = 65_536;
pub const SUBSTITUTION_READ_TIMEOUT_USEC: i64 = 30_000_000;
pub const MAX_SCRIPT_BYTES: usize = 1_048_576;
pub const MAX_HISTORY_BYTES: usize = 1_048_576;
pub const MAX_HISTORY_LINE_BYTES: usize = 1024 * 1024;

// Global flags
pub static SIGINT_RECEIVED: AtomicBool = AtomicBool::new(false);
pub static SIGTSTP_RECEIVED: AtomicBool = AtomicBool::new(false);
pub static ACTIVE_CHILDREN: AtomicUsize = AtomicUsize::new(0);

// Job control
pub struct Job {
    pub pid: i32,
    pub pgid: i32,
    pub command: String,
    pub state: JobState,
}

#[derive(Clone, PartialEq)]
pub enum JobState {
    Running,
    Stopped,
}

pub static JOBS: LazyLock<Mutex<Vec<Job>>> = LazyLock::new(|| Mutex::new(Vec::new()));
pub static PROC_SUB_PIDS: LazyLock<Mutex<Vec<i32>>> = LazyLock::new(|| Mutex::new(Vec::new()));

// Shell state
pub struct ShellState {
    pub last_status: u8,
    pub script_name: String,
    pub positional_params: Vec<String>,
    pub pipefail: bool,
    pub exit_requested: bool,
    pub exit_code: u8,
    pub last_bg_pid: Option<i32>,
}

impl ShellState {
    pub fn new() -> Self {
        ShellState {
            last_status: 0,
            script_name: "sh".to_string(),
            positional_params: Vec::new(),
            pipefail: env::var("SHELLOPTS").unwrap_or_default().contains("pipefail"),
            exit_requested: false,
            exit_code: 0,
            last_bg_pid: None,
        }
    }
}

// Utilities
pub fn safe_cstring(s: &str) -> Option<CString> {
    CString::new(s.as_bytes()).ok()
}

pub fn build_argv(tokens: &[String]) -> Option<(Vec<CString>, Vec<*const libc::c_char>)> {
    let cstrings: Vec<CString> = tokens.iter().map(|s| safe_cstring(s)).collect::<Option<Vec<_>>>()?;
    let ptrs: Vec<*const libc::c_char> = cstrings
        .iter()
        .map(|s| s.as_ptr())
        .chain(std::iter::once(std::ptr::null()))
        .collect();
    Some((cstrings, ptrs))
}

#[inline]
pub fn push_char_at(s: &str, buf: &mut String, i: &mut usize) {
    let c = s[*i..].chars().next().unwrap();
    buf.push(c);
    *i += c.len_utf8();
}

pub fn is_valid_identifier(s: &str) -> bool {
    if s.is_empty() { return false; }
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}