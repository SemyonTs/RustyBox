// =============================================================================
// sh — POSIX-compliant command interpreter.
// =============================================================================
// Copyright (c) 2026 Semyon Tsarev
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// project, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Supported features:
//   - Command execution with PATH resolution.
//   - Builtin commands: cd, exit, exec, export, pwd, alias, jobs, fg, bg,
//     source (`.`), eval, set, unset.
//   - I/O redirection: <, >, >>.
//   - Pipelines: |.
//   - Conditional execution: &&, ||.
//   - Background execution: &.
//   - Command substitution: $(...) and `...`.
//   - Globbing (wildcards): *, ?, [...].
//   - Signal handling: SIGINT forwards to foreground child; shell ignores
//     SIGQUIT; SIGTSTP stops child and (optionally) the shell itself.
//   - Exit status: $? expands to the last command's exit code.
//   - Line comments: # outside quotes.
//   - Backslash escaping outside quotes.
//   - Tilde expansion: ~ and ~user.
//   - Readline support (rustyline): line editing, history, file completion,
//     command completion (first word), history hints.
//   - Dynamic prompt: ~/..../last_component (intermediate dirs collapsed).
//   - Function definitions: name() { body; }.
//   - Process substitution: <(cmd) and >(cmd).
//   - Shell options: pipefail (set -o pipefail / +o pipefail).
//   - HISTCONTROL support (ignoredups).
//   - History saved to ~/.rbsh_history (max 500 entries; commands
//     starting with space are not recorded).
//   - Script execution: sh script.sh [args...] or sh -c "command".
// =============================================================================

use crate::context::Context;
use crate::flags::CommandFlags;
use crate::registry;

use rustyline::completion::{Completer, FilenameCompleter, Pair};
use rustyline::error::ReadlineError;
use rustyline::highlight::Highlighter;
use rustyline::hint::{Hinter, HistoryHinter};
use rustyline::validate::{ValidationResult, Validator};
use rustyline::{CompletionType, Config, EditMode, Editor, Helper};

use std::collections::{HashMap, HashSet, VecDeque};
use std::env;
use std::ffi::{CStr, CString};
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::io::{IntoRawFd, RawFd};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{LazyLock, Mutex};

// -----------------------------------------------------------------------------
// ANSI escape sequences for prompt coloring.
// -----------------------------------------------------------------------------
const ANSI_CYAN: &str = "\x1b[36m";
const ANSI_GREEN: &str = "\x1b[32m";
const ANSI_RED: &str = "\x1b[31m";
const ANSI_RESET: &str = "\x1b[0m";

// -----------------------------------------------------------------------------
// Default history size limit and maximum function call depth.
// -----------------------------------------------------------------------------
const DEFAULT_HIST_LIMIT: usize = 500;
const MAX_FUNCTION_DEPTH: usize = 1000;

// -----------------------------------------------------------------------------
// Security limits.
// -----------------------------------------------------------------------------
/// Maximum bytes read from a command substitution before truncation.
const MAX_SUBSTITUTION_BYTES: usize = 16 * 1024 * 1024; // 16 MiB
/// Maximum number of concurrent child processes per parent (secondary
/// fork-bomb mitigation; primary defense is RLIMIT_NPROC).
const MAX_CHILD_PROCESSES: usize = 256;
/// Maximum iterations in glob pattern matching (ReDoS mitigation).
const MAX_GLOB_ITERATIONS: usize = 1_000_000;
/// Maximum number of glob results per pattern to prevent memory exhaustion.
const MAX_GLOB_RESULTS: usize = 65_536;
/// Timeout (in microseconds) for command substitution reads to prevent
/// indefinite blocking on a child that never closes its stdout.
const SUBSTITUTION_READ_TIMEOUT_USEC: i64 = 30_000_000; // 30 seconds

/// Maximum size of a script or sourced file (1 MiB) to prevent DoS.
const MAX_SCRIPT_BYTES: usize = 1_048_576;
/// Maximum size of the history file (1 MiB) to prevent DoS.
const MAX_HISTORY_BYTES: usize = 1_048_576;
/// Maximum length of a single history line to prevent memory exhaustion.
const MAX_HISTORY_LINE_BYTES: usize = 1024 * 1024; // 1 MiB

// -----------------------------------------------------------------------------
// Global flags set by signal handlers.
// -----------------------------------------------------------------------------
static SIGINT_RECEIVED: AtomicBool = AtomicBool::new(false);
static SIGTSTP_RECEIVED: AtomicBool = AtomicBool::new(false);

// -----------------------------------------------------------------------------
// Global counter of live child processes (per-process fork-bomb mitigation).
// Each process tracks its own direct children after fork.
// -----------------------------------------------------------------------------
static ACTIVE_CHILDREN: AtomicUsize = AtomicUsize::new(0);

// -----------------------------------------------------------------------------
// Job control structures and global job table.
// -----------------------------------------------------------------------------
struct Job {
    pid: i32,
    /// Process group ID (set via setpgid at fork time).
    pgid: i32,
    command: String,
    state: JobState,
}

#[derive(Clone, PartialEq)]
enum JobState {
    Running,
    Stopped,
}

static JOBS: LazyLock<Mutex<Vec<Job>>> = LazyLock::new(|| Mutex::new(Vec::new()));

// -----------------------------------------------------------------------------
// Process substitution PIDs (tracked separately to avoid printing "Done"
// and to ensure they are reaped without polluting the job table).
// -----------------------------------------------------------------------------
static PROC_SUB_PIDS: LazyLock<Mutex<Vec<i32>>> = LazyLock::new(|| Mutex::new(Vec::new()));

// -----------------------------------------------------------------------------
// Shell state that holds transient information (last status, positional
// parameters, etc.).  Environment variables for special parameters are **not**
// exported to child processes; they live only inside this structure.
// -----------------------------------------------------------------------------
struct ShellState {
    last_status: u8,
    /// $0  – script / shell name
    script_name: String,
    /// $1, $2, ... $9
    positional_params: Vec<String>,
    /// Whether `pipefail` is enabled
    pipefail: bool,
    /// Set to true when the `exit` builtin is invoked; the main loop checks
    /// this flag to perform orderly cleanup instead of calling
    /// `std::process::exit` directly.
    exit_requested: bool,
    exit_code: u8,
    /// PID of the most recently launched background job ($!).
    last_bg_pid: Option<i32>,
}

impl ShellState {
    fn new() -> Self {
        ShellState {
            last_status: 0,
            script_name: "sh".to_string(),
            positional_params: Vec::new(),
            pipefail: env::var("SHELLOPTS")
                .unwrap_or_default()
                .contains("pipefail"),
            exit_requested: false,
            exit_code: 0,
            last_bg_pid: None,
        }
    }
}

// -----------------------------------------------------------------------------
// Signal handlers (POSIX sigaction).
// -----------------------------------------------------------------------------
extern "C" fn sigint_handler(_: libc::c_int) {
    SIGINT_RECEIVED.store(true, Ordering::SeqCst);
}

extern "C" fn sigtstp_handler(_: libc::c_int) {
    SIGTSTP_RECEIVED.store(true, Ordering::SeqCst);
}

fn setup_signal_handlers() {
    unsafe {
        // SIGINT – forward to foreground child.
        let mut sa: libc::sigaction = std::mem::zeroed();
        sa.sa_sigaction = sigint_handler as *const () as usize;
        sa.sa_flags = 0; // no SA_RESTART – allow waitpid to return EINTR
        libc::sigemptyset(&mut sa.sa_mask);
        libc::sigaction(libc::SIGINT, &sa, std::ptr::null_mut());

        // SIGQUIT – ignore (POSIX requires shells to be immune).
        let mut sa_ignore: libc::sigaction = std::mem::zeroed();
        sa_ignore.sa_sigaction = libc::SIG_IGN as usize;
        sa_ignore.sa_flags = 0;
        libc::sigemptyset(&mut sa_ignore.sa_mask);
        libc::sigaction(libc::SIGQUIT, &sa_ignore, std::ptr::null_mut());

        // SIGTSTP – record the event; the wait loop forwards it to the child.
        let mut sa_tstp: libc::sigaction = std::mem::zeroed();
        sa_tstp.sa_sigaction = sigtstp_handler as *const () as usize;
        sa_tstp.sa_flags = 0;
        libc::sigemptyset(&mut sa_tstp.sa_mask);
        libc::sigaction(libc::SIGTSTP, &sa_tstp, std::ptr::null_mut());

        // SIGCHLD – default disposition (we poll via waitpid).
        libc::signal(libc::SIGCHLD, libc::SIG_DFL);
    }
}

// -----------------------------------------------------------------------------
// Set RLIMIT_NPROC as the primary fork-bomb mitigation.  This is enforced
// by the kernel and cannot be bypassed by child processes resetting a
// userspace counter.
// -----------------------------------------------------------------------------
fn setup_nproc_limit() {
    unsafe {
        let mut rlim: libc::rlimit = std::mem::zeroed();
        if libc::getrlimit(libc::RLIMIT_NPROC, &mut rlim) == 0 {
            // Only lower the limit; never raise it above the current hard max.
            let desired: libc::rlim_t = 4096;
            if rlim.rlim_max == libc::RLIM_INFINITY || rlim.rlim_max > desired {
                rlim.rlim_cur = desired;
                // Do not touch rlim_max – we only set the soft limit.
                libc::setrlimit(libc::RLIMIT_NPROC, &rlim);
            }
        }
    }
}

// -----------------------------------------------------------------------------
// Safe CString construction: returns None if the input contains interior
// null bytes, preventing panics that would crash the shell.
// -----------------------------------------------------------------------------
fn safe_cstring(s: &str) -> Option<CString> {
    CString::new(s.as_bytes()).ok()
}

/// Build an argv array from tokens.  Returns None if any token contains a
/// null byte (which would be an injection / corruption indicator).
fn build_argv(tokens: &[String]) -> Option<(Vec<CString>, Vec<*const libc::c_char>)> {
    let cstrings: Vec<CString> = tokens
        .iter()
        .map(|s| safe_cstring(s))
        .collect::<Option<Vec<_>>>()?;
    let ptrs: Vec<*const libc::c_char> = cstrings
        .iter()
        .map(|s| s.as_ptr())
        .chain(std::iter::once(std::ptr::null()))
        .collect();
    Some((cstrings, ptrs))
}

// -----------------------------------------------------------------------------
// Fork-bomb mitigation: acquire / release a child-process slot.
// This is a per-process secondary defense; the primary defense is
// RLIMIT_NPROC set at shell startup.
// -----------------------------------------------------------------------------
fn acquire_child_slot() -> bool {
    loop {
        let current = ACTIVE_CHILDREN.load(Ordering::SeqCst);
        if current >= MAX_CHILD_PROCESSES {
            return false;
        }
        if ACTIVE_CHILDREN
            .compare_exchange(current, current + 1, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            return true;
        }
    }
}

/// Release a child-process slot.  Uses a compare-exchange loop to prevent
/// underflow (which would disable the fork-bomb mitigation entirely).
fn release_child_slot() {
    loop {
        let current = ACTIVE_CHILDREN.load(Ordering::SeqCst);
        if current == 0 {
            return; // already zero; do not underflow
        }
        if ACTIVE_CHILDREN
            .compare_exchange(current, current - 1, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            return;
        }
    }
}

// -----------------------------------------------------------------------------
// UTF-8 aware byte push helper.  Since all shell metacharacters are ASCII
// (< 0x80) and Rust &str is guaranteed valid UTF-8, we can safely scan
// bytes for operators while correctly pushing multi-byte characters.
// -----------------------------------------------------------------------------
#[inline]
fn push_char_at(s: &str, buf: &mut String, i: &mut usize) {
    let c = s[*i..].chars().next().unwrap();
    buf.push(c);
    *i += c.len_utf8();
}

// -----------------------------------------------------------------------------
// Path cache for faster command lookup during completion.
// -----------------------------------------------------------------------------
#[derive(Clone, Default)]
struct PathCache {
    entries: Vec<String>,
    last_path: String,
}

impl PathCache {
    fn new() -> Self {
        Self {
            entries: Vec::new(),
            last_path: String::new(),
        }
    }

    fn refresh(&mut self) {
        let path = env::var("PATH").unwrap_or_default();
        if path == self.last_path {
            return;
        }
        self.last_path = path.clone();
        self.entries.clear();
        for dir in path.split(':') {
            if let Ok(entries) = fs::read_dir(dir) {
                for entry in entries.flatten() {
                    let name = entry.file_name().to_string_lossy().into_owned();
                    self.entries.push(name);
                }
            }
        }
        self.entries.sort();
        self.entries.dedup();
    }

    fn all_commands(&mut self) -> Vec<String> {
        self.refresh();
        self.entries.clone()
    }
}

// -----------------------------------------------------------------------------
// Static completions for common command options.
// -----------------------------------------------------------------------------
fn static_options(cmd: &str) -> Vec<&'static str> {
    match cmd {
        "ls" => vec![
            "-l", "-a", "-h", "-r", "-t", "-S", "-R", "-1", "-F", "--color",
        ],
        "grep" => vec!["-i", "-v", "-r", "-n", "-c", "-l", "-E", "-F", "-w", "-x"],
        "cat" => vec!["-n", "-b", "-s", "-v", "-E", "-T", "-A"],
        "echo" => vec!["-n", "-e", "-E"],
        "head" => vec!["-n", "-c"],
        "tail" => vec!["-n", "-c", "-f"],
        "wc" => vec!["-l", "-w", "-c", "-m"],
        "sort" => vec!["-r", "-n", "-u", "-k"],
        "find" => vec!["-name", "-type", "-size", "-mtime", "-exec"],
        "sh" => vec!["-c"],
        _ => vec![],
    }
}

// -----------------------------------------------------------------------------
// Sensitive environment variable prefixes that should not appear in
// tab-completion candidates to prevent shoulder-surfing information leaks.
// -----------------------------------------------------------------------------
const SENSITIVE_VAR_PREFIXES: &[&str] = &[
    "AWS_SECRET",
    "AWS_SESSION_TOKEN",
    "DATABASE_URL",
    "DB_PASSWORD",
    "SECRET",
    "TOKEN",
    "PRIVATE_KEY",
    "API_KEY",
    "PASSWORD",
    "PASSWD",
    "CREDENTIALS",
    "AUTH",
    "ENCRYPTION_KEY",
    "SIGNING_KEY",
];

fn is_sensitive_var(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    SENSITIVE_VAR_PREFIXES
        .iter()
        .any(|prefix| upper.contains(prefix))
}

// -----------------------------------------------------------------------------
// Custom rustyline Helper: command/option/variable completion, history hints,
// syntax highlighting.
// -----------------------------------------------------------------------------
struct ShHelper {
    file_completer: FilenameCompleter,
    history_hinter: HistoryHinter,
    builtins: Vec<String>,
    path_cache: PathCache,
    opts: ShellOpts,
}

#[derive(Clone)]
struct ShellOpts {
    pipefail: bool,
    hist_control: Option<String>,
}

impl Completer for ShHelper {
    type Candidate = Pair;

    fn complete(
        &self,
        line: &str,
        pos: usize,
        _ctx: &rustyline::Context<'_>,
    ) -> rustyline::Result<(usize, Vec<Pair>)> {
        let (word_start, word) = extract_word(line, pos);

        // Variable name completion after '$' – filter out sensitive variables.
        if word.starts_with('$') {
            let varname = &word[1..];
            let mut matches: Vec<Pair> = env::vars()
                .filter(|(k, _)| k.starts_with(varname) && !is_sensitive_var(k))
                .map(|(k, _)| Pair {
                    display: format!("${}", k),
                    replacement: format!("${}", k),
                })
                .collect();
            matches.sort_by(|a, b| a.display.cmp(&b.display));
            if !matches.is_empty() {
                return Ok((word_start, matches));
            }
        }

        // Option completion after a command
        if !is_first_word(line, word_start) && word.starts_with('-') {
            if let Some(cmd) = get_first_command(line) {
                let opts = static_options(&cmd);
                let filtered: Vec<Pair> = opts
                    .iter()
                    .filter(|o| o.starts_with(word))
                    .map(|&o| Pair {
                        display: o.to_string(),
                        replacement: o.to_string(),
                    })
                    .collect();
                if !filtered.is_empty() {
                    return Ok((word_start, filtered));
                }
            }
        }

        // First word: command + builtins
        if is_first_word(line, word_start) {
            let mut candidates = self.builtins.clone();
            let mut cache = self.path_cache.clone();
            candidates.extend(cache.all_commands());
            candidates.sort();
            candidates.dedup();

            let matches: Vec<Pair> = candidates
                .into_iter()
                .filter(|c| c.starts_with(word))
                .map(|c| Pair {
                    display: c.clone(),
                    replacement: c.clone(),
                })
                .collect();
            return Ok((word_start, matches));
        }

        // Otherwise: file completion with tilde expansion
        let expanded = expand_tilde_one(word);
        let file_word: &str = if expanded != *word { &expanded } else { word };
        let new_line = format!("{}{}", &line[..word_start], file_word);
        let res = self
            .file_completer
            .complete(&new_line, word_start + file_word.len(), _ctx);
        res.map(|(pos, pairs)| {
            let pairs = pairs
                .into_iter()
                .map(|p| {
                    let replacement =
                        if word.starts_with('~') && p.replacement.starts_with(file_word) {
                            let rel = p
                                .replacement
                                .strip_prefix(file_word)
                                .unwrap_or(&p.replacement);
                            format!("~{}", rel)
                        } else {
                            p.replacement
                        };
                    Pair {
                        display: p.display,
                        replacement,
                    }
                })
                .collect();
            (pos, pairs)
        })
    }
}

impl Hinter for ShHelper {
    type Hint = String;

    fn hint(&self, line: &str, pos: usize, ctx: &rustyline::Context<'_>) -> Option<String> {
        self.history_hinter.hint(line, pos, ctx)
    }
}

impl Highlighter for ShHelper {
    fn highlight<'l>(&self, line: &'l str, _pos: usize) -> std::borrow::Cow<'l, str> {
        let single_count = line.chars().filter(|&c| c == '\'').count();
        let double_count = line.chars().filter(|&c| c == '"').count();
        if single_count % 2 != 0 || double_count % 2 != 0 {
            return std::borrow::Cow::Owned(format!("{}{}{}", ANSI_RED, line, ANSI_RESET));
        }
        std::borrow::Cow::Borrowed(line)
    }
}

impl Validator for ShHelper {
    fn validate(
        &self,
        _ctx: &mut rustyline::validate::ValidationContext,
    ) -> Result<ValidationResult, ReadlineError> {
        Ok(ValidationResult::Valid(None))
    }
}

impl Helper for ShHelper {}

// -----------------------------------------------------------------------------
// Word extraction and first-word detection.
// -----------------------------------------------------------------------------
fn extract_word(line: &str, pos: usize) -> (usize, &str) {
    let bytes = line.as_bytes();
    let mut start = pos;
    while start > 0 && !bytes[start - 1].is_ascii_whitespace() {
        start -= 1;
    }
    (start, &line[start..pos])
}

fn is_first_word(line: &str, word_start: usize) -> bool {
    line[..word_start].trim().is_empty()
}

fn get_first_command(line: &str) -> Option<String> {
    let tokens = tokenize_to_strings(line);
    tokens.into_iter().next()
}

// -----------------------------------------------------------------------------
// Entry point.
// -----------------------------------------------------------------------------
fn sh_main(ctx: &mut Context) -> u8 {
    let _opts = match crate::args::parse(ctx, "") {
        Ok(o) => o,
        Err(e) => {
            eprintln!("sh: {e}");
            return 1;
        }
    };

    let rustybox_mode = ctx
        .optargs
        .first()
        .map(|s| s == "rustybox")
        .unwrap_or(false);

    setup_signal_handlers();
    setup_nproc_limit();

    let mut shell_args = ctx.optargs.clone();
    if shell_args.first().map(|s| s.as_str()) == Some("rustybox") {
        shell_args.remove(0);
    }

    // Check for non-interactive mode: sh -c "cmd" or sh script [args...]
    if !shell_args.is_empty() {
        let (cmd, script_file, script_args) = parse_shell_arguments(&shell_args);

        if cmd.is_some() || script_file.is_some() {
            let mut aliases: HashMap<String, String> = HashMap::new();
            let mut functions: HashMap<String, String> = HashMap::new();

            let mut state = ShellState::new();

            let status = if let Some(cmd_str) = cmd {
                // For `sh -c "cmd" name arg1 arg2`, `name` becomes $0, and `arg1 arg2` become $1, $2...
                let mut pos_params = script_args.clone();
                state.script_name = if pos_params.is_empty() {
                    "sh".to_string()
                } else {
                    pos_params.remove(0)
                };
                state.positional_params = pos_params;

                run_command_list(
                    &cmd_str,
                    &mut state,
                    0,
                    rustybox_mode,
                    &mut aliases,
                    &mut functions,
                )
            } else if let Some(script) = script_file {
                state.script_name = script.clone();
                state.positional_params = script_args.clone();

                execute_script(
                    &script,
                    &script_args,
                    &mut state,
                    rustybox_mode,
                    &mut aliases,
                    &mut functions,
                )
            } else {
                0
            };

            return status;
        }
    }

    // Interactive mode setup
    let builtins = {
        let mut names = vec![
            "cd", "exit", "exec", "export", "pwd", "alias", "history", "jobs", "fg", "bg",
            "source", "eval", "set", "unset",
        ]
        .into_iter()
        .map(String::from)
        .collect::<Vec<_>>();
        let common = vec![
            "ls", "cat", "echo", "grep", "head", "tail", "wc", "sh", "sort", "find",
        ];
        names.extend(common.into_iter().map(String::from));
        names.sort();
        names.dedup();
        names
    };

    let shell_opts = ShellOpts {
        pipefail: env::var("SHELLOPTS")
            .unwrap_or_default()
            .contains("pipefail"),
        hist_control: env::var("HISTCONTROL").ok(),
    };

    let helper = ShHelper {
        file_completer: FilenameCompleter::new(),
        history_hinter: HistoryHinter {},
        builtins,
        path_cache: PathCache::new(),
        opts: shell_opts,
    };

    let config = Config::builder()
        .completion_type(CompletionType::List)
        .edit_mode(EditMode::Emacs)
        .build();
    let mut rl = Editor::with_config(config).expect("Failed to create line editor");
    rl.set_helper(Some(helper));

    // History file: defaults to ~/.rbsh_history.
    let hist_file = env::var("HISTFILE").unwrap_or_else(|_| {
        let home = env::var("HOME").unwrap_or_else(|_| ".".into());
        format!("{}/.rbsh_history", home)
    });

    // Load history safely: read at most DEFAULT_HIST_LIMIT entries with a total
    // byte limit to prevent DoS, avoiding TOCTOU by reading in a bounded manner.
    if let Ok(file) = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&hist_file)
    {
        // Read at most MAX_HISTORY_BYTES bytes, then split into lines.
        let mut reader = BufReader::new(file.take(MAX_HISTORY_BYTES as u64));
        let mut lines: VecDeque<String> = VecDeque::with_capacity(DEFAULT_HIST_LIMIT);
        for line_res in reader.lines() {
            if let Ok(line) = line_res {
                // Skip overly long lines to prevent memory exhaustion.
                if line.len() > MAX_HISTORY_LINE_BYTES {
                    continue;
                }
                if lines.len() >= DEFAULT_HIST_LIMIT {
                    lines.pop_front();
                }
                lines.push_back(line);
            }
        }
        for line in &lines {
            rl.add_history_entry(line);
        }
    }

    let mut state = ShellState::new();
    let mut aliases: HashMap<String, String> = HashMap::new();
    let mut functions: HashMap<String, String> = HashMap::new();

    // Source ~/.shrc on startup if it exists.
    // SECURITY: Use O_NOFOLLOW to prevent symlink attacks and TOCTOU races.
    if let Ok(home) = env::var("HOME") {
        let rc_file = format!("{}/.shrc", home);
        if let Ok(file) = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&rc_file)
        {
            // Verify it's a regular file (not a directory, device, etc.)
            if let Ok(meta) = file.metadata() {
                if meta.is_file() {
                    // Read with a hard byte limit to prevent DoS
                    let mut reader = BufReader::new(file.take(MAX_SCRIPT_BYTES as u64));
                    let mut content = String::new();
                    if reader.read_to_string(&mut content).is_ok() {
                        let _ = run_command_list(
                            &content,
                            &mut state,
                            0,
                            rustybox_mode,
                            &mut aliases,
                            &mut functions,
                        );
                    }
                }
            }
        }
    }

    loop {
        // Check if `exit` was requested by a builtin.
        if state.exit_requested {
            break;
        }

        // Reap finished background jobs to prevent zombie accumulation
        // while the user is at the prompt.
        reap_background();

        let prompt = make_prompt();
        let readline = rl.readline(&prompt);
        match readline {
            Ok(line) => {
                let input = line.trim().to_string();
                if input.is_empty() {
                    continue;
                }

                // Commands starting with a space are not saved to history.
                if !line.starts_with(' ') {
                    let should_add = match rl.helper().map(|h| &h.opts.hist_control) {
                        Some(Some(control)) if control == "ignoredups" => {
                            let prev = rl.history().iter().last().map(|s| s.clone());
                            prev != Some(input.clone())
                        }
                        _ => true,
                    };
                    if should_add {
                        rl.add_history_entry(&input).ok();
                    }
                }

                SIGINT_RECEIVED.store(false, Ordering::SeqCst);
                SIGTSTP_RECEIVED.store(false, Ordering::SeqCst);
                state.last_status = run_command_list(
                    &input,
                    &mut state,
                    0,
                    rustybox_mode,
                    &mut aliases,
                    &mut functions,
                );
            }
            Err(ReadlineError::Interrupted) => {
                eprintln!("^C");
                continue;
            }
            Err(ReadlineError::Eof) => {
                println!("exit");
                break;
            }
            Err(err) => {
                eprintln!("sh: read error: {err}");
                break;
            }
        }
    }

    // Save trimmed history to file with restrictive permissions (0600).
    // Use O_NOFOLLOW to prevent symlink attacks on the history file.
    {
        let hist = rl.history();
        let entries: Vec<String> = hist.iter().map(|s| s.to_string()).collect();
        let total = entries.len();
        let skip = if total > DEFAULT_HIST_LIMIT {
            total - DEFAULT_HIST_LIMIT
        } else {
            0
        };

        // Remove existing file first (which may be a symlink) and create
        // a fresh regular file with O_NOFOLLOW | O_CREAT | O_EXCL.
        let _ = fs::remove_file(&hist_file);
        if let Ok(mut file) = OpenOptions::new()
            .write(true)
            .create_new(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&hist_file)
        {
            // Restrict history file to owner-only read/write.
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let perms = fs::Permissions::from_mode(0o600);
                let _ = file.set_permissions(perms);
            }
            for entry in &entries[skip..] {
                let _ = writeln!(file, "{}", entry);
            }
        }
    }

    if state.exit_requested {
        state.exit_code
    } else {
        state.last_status
    }
}

// -----------------------------------------------------------------------------
// Shell argument parsing for script mode.
// -----------------------------------------------------------------------------
fn parse_shell_arguments(args: &[String]) -> (Option<String>, Option<String>, Vec<String>) {
    if args.is_empty() {
        return (None, None, vec![]);
    }

    if args[0] == "-c" {
        if args.len() < 2 {
            eprintln!("sh: -c requires an argument");
            return (Some(String::new()), None, vec![]);
        }
        let cmd = args[1].clone();
        let rest = if args.len() > 2 {
            args[2..].to_vec()
        } else {
            vec![]
        };
        return (Some(cmd), None, rest);
    }

    let script = args[0].clone();
    let script_args = if args.len() > 1 {
        args[1..].to_vec()
    } else {
        vec![]
    };
    (None, Some(script), script_args)
}

// -----------------------------------------------------------------------------
// Script execution with shebang support.
// -----------------------------------------------------------------------------
fn execute_script(
    script_path: &str,
    script_args: &[String],
    state: &mut ShellState,
    rustybox_mode: bool,
    aliases: &mut HashMap<String, String>,
    functions: &mut HashMap<String, String>,
) -> u8 {
    let file = match OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(script_path)
    {
        Ok(f) => f,
        Err(e) => {
            eprintln!("sh: {}: {}", script_path, e);
            return if e.kind() == io::ErrorKind::NotFound {
                127
            } else {
                126
            };
        }
    };

    let meta = match file.metadata() {
        Ok(m) => m,
        Err(e) => {
            eprintln!("sh: {}: {}", script_path, e);
            return 126;
        }
    };
    if !meta.is_file() {
        eprintln!("sh: {}: not a regular file", script_path);
        return 126;
    }

    let mut content = String::with_capacity(4096);
    {
        let mut reader = BufReader::new(file.take(MAX_SCRIPT_BYTES as u64));
        if let Err(e) = reader.read_to_string(&mut content) {
            eprintln!("sh: {}: {}", script_path, e);
            return 126;
        }
    }

    if content.starts_with("#!") {
        let shebang_line = content.lines().next().unwrap_or("");
        let interpreter = shebang_line.trim_start_matches("#!").trim();

        if !interpreter.is_empty() {
            let parts: Vec<&str> = interpreter.split_whitespace().collect();
            let interpreter_path = parts[0];
            let interpreter_arg = if parts.len() > 1 { parts[1] } else { "" };

            let candidates: Vec<String> = if interpreter_path.starts_with('/') {
                if let Ok(canon) = Path::new(interpreter_path).canonicalize() {
                    let trusted = [
                        Path::new("/bin"),
                        Path::new("/usr/bin"),
                        Path::new("/usr/local/bin"),
                    ];
                    if trusted.iter().any(|t| canon.starts_with(t)) {
                        vec![canon.to_string_lossy().into_owned()]
                    } else {
                        eprintln!(
                            "sh: {}: interpreter '{}' not in trusted directories",
                            script_path, interpreter_path
                        );
                        return 126;
                    }
                } else {
                    eprintln!(
                        "sh: {}: cannot resolve interpreter path '{}'",
                        script_path, interpreter_path
                    );
                    return 126;
                }
            } else {
                if interpreter_path.contains('/') || interpreter_path.contains("..") {
                    eprintln!(
                        "sh: {}: insecure interpreter name '{}'",
                        script_path, interpreter_path
                    );
                    return 126;
                }
                ["/bin", "/usr/bin", "/usr/local/bin"]
                    .iter()
                    .map(|dir| format!("{}/{}", dir, interpreter_path))
                    .collect()
            };

            let mut argv_tokens: Vec<String> = Vec::new();
            if !interpreter_arg.is_empty() {
                argv_tokens.push(interpreter_arg.to_string());
            }
            argv_tokens.push(script_path.to_string());
            argv_tokens.extend_from_slice(script_args);

            for interp in &candidates {
                let mut full_tokens = vec![interp.clone()];
                full_tokens.extend_from_slice(&argv_tokens);

                let (argv_cstrings, argv_ptrs) = match build_argv(&full_tokens) {
                    Some(v) => v,
                    None => {
                        eprintln!("sh: invalid argument (null byte) in shebang invocation");
                        return 1;
                    }
                };

                if !acquire_child_slot() {
                    eprintln!("sh: too many child processes");
                    return 1;
                }

                let pid = unsafe { libc::fork() };
                if pid < 0 {
                    eprintln!("sh: fork failed");
                    release_child_slot();
                    return 1;
                }
                if pid == 0 {
                    unsafe {
                        ACTIVE_CHILDREN.store(0, Ordering::SeqCst);
                        libc::execv(argv_ptrs[0], argv_ptrs.as_ptr());
                        libc::_exit(127);
                    }
                }
                drop(argv_cstrings);
                let status = wait_for_child(pid);
                release_child_slot();
                if status != 127 {
                    return status;
                }
            }

            eprintln!(
                "sh: {}: shebang interpreter '{}' not found in trusted directories",
                script_path, interpreter_path
            );
            return 126;
        }
    }

    run_command_list(&content, state, 0, rustybox_mode, aliases, functions)
}
// -----------------------------------------------------------------------------
// Prompt generation.
// -----------------------------------------------------------------------------
fn make_prompt() -> String {
    let home = env::var("HOME").unwrap_or_default();
    let current = env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));

    let (root, relative): (String, PathBuf) = if !home.is_empty() && current.starts_with(&home) {
        (
            "~".to_string(),
            current.strip_prefix(&home).unwrap().to_path_buf(),
        )
    } else {
        (
            "/".to_string(),
            current.strip_prefix("/").unwrap_or(&current).to_path_buf(),
        )
    };

    let components: Vec<&str> = relative
        .iter()
        .map(|c| c.to_str().unwrap_or(""))
        .filter(|s| !s.is_empty())
        .collect();

    let sep = if root == "~" { "/" } else { "" };

    let path_part = if components.is_empty() {
        root.to_string()
    } else if components.len() == 1 {
        format!("{}{}{}", root, sep, components[0])
    } else {
        let last = components.last().unwrap();
        format!("{}{}..../{}", root, sep, last)
    };

    let user = env::var("USER").unwrap_or_else(|_| "user".into());
    let host = env::var("HOSTNAME").unwrap_or_else(|_| {
        // Use a 257-byte buffer: 256 for the hostname + guaranteed NUL.
        let mut buf = [0u8; 257];
        unsafe {
            if libc::gethostname(buf.as_mut_ptr() as *mut libc::c_char, buf.len()) == 0 {
                // Ensure NUL termination even if gethostname did not provide one.
                buf[256] = 0;
                CStr::from_ptr(buf.as_ptr() as *const libc::c_char)
                    .to_string_lossy()
                    .into_owned()
            } else {
                "localhost".to_string()
            }
        }
    });

    format!(
        "{}{}@{} {} {} {}{} $ ",
        ANSI_CYAN, user, host, ANSI_RESET, ANSI_GREEN, path_part, ANSI_RESET
    )
}

// -----------------------------------------------------------------------------
// Validate that a string is a valid POSIX shell identifier.
// -----------------------------------------------------------------------------
fn is_valid_identifier(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

// -----------------------------------------------------------------------------
// Command execution – recursive depth is limited to prevent stack overflow
// from infinite function loops or deeply nested substitutions.
// -----------------------------------------------------------------------------
fn run_command_list(
    input: &str,
    state: &mut ShellState,
    depth: usize,
    rustybox_mode: bool,
    aliases: &mut HashMap<String, String>,
    functions: &mut HashMap<String, String>,
) -> u8 {
    if depth > MAX_FUNCTION_DEPTH {
        eprintln!("sh: maximum function call depth exceeded");
        return 1;
    }

    // Quote-aware parsing of function definitions.
    if let Some((name, body)) = try_parse_function_def(input) {
        functions.insert(name, body);
        return state.last_status;
    }

    // Split the input on top-level operators (;, &&, ||, &) respecting
    // quotes, $(...), `...`, and parenthesised subshells.
    let commands = split_command_list(input);

    for (pos, (cmd, _sep)) in commands.iter().enumerate() {
        if cmd.is_empty() {
            continue;
        }

        if state.exit_requested {
            break;
        }

        if pos > 0 {
            let prev_sep = commands[pos - 1].1;
            match prev_sep {
                Some('&') if state.last_status != 0 => continue,
                Some('|') if state.last_status == 0 => continue,
                _ => {}
            }
        }

        let is_background = commands[pos].1 == Some('b');
        state.last_status = run_single_command_list(
            cmd,
            state,
            depth + 1,
            rustybox_mode,
            aliases,
            functions,
            is_background,
        );
    }

    reap_background();
    state.last_status
}

/// Split input on top-level shell operators (;, &&, ||, &) while respecting
/// single quotes, double quotes, $(...), `...`, and (...) subshells.
/// Returns a vector of (command_string, separator) pairs.  The separator
/// indicates what followed the command:
///   Some('&')  – followed by &&
///   Some('|')  – followed by ||
///   Some(';')  – followed by ;
///   Some('b')  – followed by & (background)
///   None       – last command (no trailing operator)
fn split_command_list(input: &str) -> Vec<(String, Option<char>)> {
    let bytes = input.as_bytes();
    let mut commands: Vec<(String, Option<char>)> = Vec::new();
    let mut current = String::with_capacity(64);
    let mut in_single = false;
    let mut in_double = false;
    let mut in_backtick = false;
    let mut paren_depth: usize = 0; // tracks $( ... ) and ( ... )
    let mut i = 0;

    while i < bytes.len() {
        let b = bytes[i];

        if in_single {
            push_char_at(input, &mut current, &mut i);
            if b == b'\'' {
                in_single = false;
            }
            continue;
        }

        if in_double {
            if b == b'"' {
                current.push('"');
                in_double = false;
                i += 1;
            } else if b == b'\\' && i + 1 < bytes.len() {
                i += 1;
                push_char_at(input, &mut current, &mut i);
            } else if b == b'$' && i + 1 < bytes.len() && bytes[i + 1] == b'(' {
                current.push('(');
                paren_depth += 1;
                i += 2;
            } else if b == b'(' && paren_depth > 0 {
                current.push('(');
                paren_depth += 1;
                i += 1;
            } else if b == b')' && paren_depth > 0 {
                current.push(')');
                paren_depth -= 1;
                i += 1;
            } else {
                push_char_at(input, &mut current, &mut i);
            }
            continue;
        }

        if in_backtick {
            if b == b'\\' && i + 1 < bytes.len() {
                current.push('\\');
                i += 1;
                push_char_at(input, &mut current, &mut i);
            } else if b == b'`' {
                current.push('`');
                in_backtick = false;
                i += 1;
            } else {
                push_char_at(input, &mut current, &mut i);
            }
            continue;
        }

        // Outside any quoting context.
        if b == b'\'' {
            in_single = true;
            current.push('\'');
            i += 1;
        } else if b == b'"' {
            in_double = true;
            current.push('"');
            i += 1;
        } else if b == b'`' {
            in_backtick = true;
            current.push('`');
            i += 1;
        } else if b == b'$' && i + 1 < bytes.len() && bytes[i + 1] == b'(' {
            current.push('$');
            current.push('(');
            paren_depth += 1;
            i += 2;
        } else if b == b'(' {
            paren_depth += 1;
            current.push('(');
            i += 1;
        } else if b == b')' {
            if paren_depth > 0 {
                paren_depth -= 1;
            }
            current.push(')');
            i += 1;
        } else if b == b'\\' && i + 1 < bytes.len() {
            current.push('\\');
            i += 1;
            push_char_at(input, &mut current, &mut i);
        } else if paren_depth == 0 && b == b'&' && i + 1 < bytes.len() && bytes[i + 1] == b'&' {
            commands.push((current.trim().to_string(), Some('&')));
            current.clear();
            i += 2;
        } else if paren_depth == 0 && b == b'|' && i + 1 < bytes.len() && bytes[i + 1] == b'|' {
            commands.push((current.trim().to_string(), Some('|')));
            current.clear();
            i += 2;
        } else if paren_depth == 0 && b == b';' {
            commands.push((current.trim().to_string(), Some(';')));
            current.clear();
            i += 1;
        } else if paren_depth == 0 && b == b'&' {
            commands.push((current.trim().to_string(), Some('b')));
            current.clear();
            i += 1;
        } else {
            push_char_at(input, &mut current, &mut i);
        }
    }

    if !current.trim().is_empty() {
        commands.push((current.trim().to_string(), None));
    }

    commands
}

/// Find the matching closing brace for a function definition, respecting
/// quotes, $(...), `...`, and nested braces.
fn find_matching_brace(input: &str, start: usize) -> Option<usize> {
    let bytes = input.as_bytes();
    let mut depth = 1;
    let mut i = start + 1;
    let mut in_single = false;
    let mut in_double = false;
    let mut in_backtick = false;
    let mut paren_depth: usize = 0; // tracks $(...) nesting

    while i < bytes.len() {
        let b = bytes[i];

        if in_single {
            if b == b'\'' {
                in_single = false;
            }
        } else if in_double {
            if b == b'"' {
                in_double = false;
            } else if b == b'\\' && i + 1 < bytes.len() {
                i += 1; // skip escaped char
            } else if b == b'$' && i + 1 < bytes.len() && bytes[i + 1] == b'(' {
                paren_depth += 1;
                i += 2; // skip BOTH '$' and '(' to avoid double counting
                continue;
            } else if b == b'(' && paren_depth > 0 {
                paren_depth += 1;
            } else if b == b')' && paren_depth > 0 {
                paren_depth -= 1;
            }
        } else if in_backtick {
            if b == b'\\' && i + 1 < bytes.len() {
                i += 1;
            } else if b == b'`' {
                in_backtick = false;
            }
        } else {
            match b {
                b'\'' => in_single = true,
                b'"' => in_double = true,
                b'`' => in_backtick = true,
                b'$' if i + 1 < bytes.len() && bytes[i + 1] == b'(' => {
                    paren_depth += 1;
                    i += 2; // skip BOTH '$' and '(' to avoid double counting
                    continue;
                }
                b'(' if paren_depth > 0 => paren_depth += 1,
                b')' if paren_depth > 0 => paren_depth -= 1,
                b'{' if paren_depth == 0 => depth += 1,
                b'}' if paren_depth == 0 => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(i);
                    }
                }
                _ => {}
            }
        }
        i += 1;
    }
    None
}

/// Quote-aware parsing of function definitions: name() { body; }
/// Only matches if the ENTIRE input is a single function definition with a
/// valid identifier name.  This prevents false positives on compound commands
/// that happen to contain braces. Trailing comments are allowed.
fn try_parse_function_def(input: &str) -> Option<(String, String)> {
    let trimmed = input.trim();

    let bytes = trimmed.as_bytes();
    let len = bytes.len();
    let mut brace_pos = None;
    let mut single = false;
    let mut double = false;
    let mut backtick = false;
    let mut paren_depth: usize = 0; // tracks $(...) nesting
    let mut i = 0;

    while i < len {
        let b = bytes[i];
        if single {
            if b == b'\'' {
                single = false;
            }
            i += 1;
        } else if double {
            if b == b'"' {
                double = false;
            } else if b == b'\\' && i + 1 < len {
                i += 1;
            } else if b == b'$' && i + 1 < len && bytes[i + 1] == b'(' {
                paren_depth += 1;
                i += 2;
                continue;
            } else if b == b'(' && paren_depth > 0 {
                paren_depth += 1;
            } else if b == b')' && paren_depth > 0 {
                paren_depth -= 1;
            }
            i += 1;
        } else if backtick {
            if b == b'\\' && i + 1 < len {
                i += 1;
            } else if b == b'`' {
                backtick = false;
            }
            i += 1;
        } else if b == b'\'' {
            single = true;
            i += 1;
        } else if b == b'"' {
            double = true;
            i += 1;
        } else if b == b'`' {
            backtick = true;
            i += 1;
        } else if b == b'$' && i + 1 < len && bytes[i + 1] == b'(' {
            paren_depth += 1;
            i += 2;
        } else if b == b'(' && paren_depth > 0 {
            paren_depth += 1;
            i += 1;
        } else if b == b')' && paren_depth > 0 {
            paren_depth -= 1;
            i += 1;
        } else if b == b'{' && paren_depth == 0 {
            brace_pos = Some(i);
            break;
        } else if b == b';' || b == b'|' || b == b'&' {
            // If we encounter a command separator before '{', this is not a
            // pure function definition.
            return None;
        } else {
            i += 1;
        }
    }

    let open = brace_pos?;

    // Find the matching closing brace (now aware of nested substitutions)
    let close = find_matching_brace(trimmed, open)?;

    // Check if everything after the closing brace is just a comment or whitespace
    let remainder = trimmed[close + 1..].trim();
    if !remainder.is_empty() && !remainder.starts_with('#') {
        return None;
    }

    let header = trimmed[..open].trim();
    let name = header.strip_suffix("()")?;
    let name = name.trim();

    // Validate that the function name is a legal POSIX identifier.
    if !is_valid_identifier(name) {
        return None;
    }

    let body = trimmed[open + 1..close].trim().to_string();
    Some((name.to_string(), body))
}

/// Split a command string on top-level pipe characters '|', respecting
/// quotes, $(...), `...`, and parentheses.  Does NOT split on '||' (that
/// is handled at the command-list level).
fn split_pipeline(input: &str) -> Vec<String> {
    let bytes = input.as_bytes();
    let mut segments: Vec<String> = Vec::new();
    let mut current = String::with_capacity(64);
    let mut in_single = false;
    let mut in_double = false;
    let mut in_backtick = false;
    let mut paren_depth: usize = 0;
    let mut i = 0;

    while i < bytes.len() {
        let b = bytes[i];

        if in_single {
            push_char_at(input, &mut current, &mut i);
            if b == b'\'' {
                in_single = false;
            }
            continue;
        }

        if in_double {
            if b == b'"' {
                current.push('"');
                in_double = false;
                i += 1;
            } else if b == b'\\' && i + 1 < bytes.len() {
                i += 1;
                push_char_at(input, &mut current, &mut i);
            } else if b == b'$' && i + 1 < bytes.len() && bytes[i + 1] == b'(' {
                current.push('(');
                paren_depth += 1;
                i += 2;
            } else if b == b'(' && paren_depth > 0 {
                current.push('(');
                paren_depth += 1;
                i += 1;
            } else if b == b')' && paren_depth > 0 {
                current.push(')');
                paren_depth -= 1;
                i += 1;
            } else {
                push_char_at(input, &mut current, &mut i);
            }
            continue;
        }

        if in_backtick {
            if b == b'\\' && i + 1 < bytes.len() {
                current.push('\\');
                i += 1;
                push_char_at(input, &mut current, &mut i);
            } else if b == b'`' {
                current.push('`');
                in_backtick = false;
                i += 1;
            } else {
                push_char_at(input, &mut current, &mut i);
            }
            continue;
        }

        if b == b'\'' {
            in_single = true;
            current.push('\'');
            i += 1;
        } else if b == b'"' {
            in_double = true;
            current.push('"');
            i += 1;
        } else if b == b'`' {
            in_backtick = true;
            current.push('`');
            i += 1;
        } else if b == b'$' && i + 1 < bytes.len() && bytes[i + 1] == b'(' {
            current.push('$');
            current.push('(');
            paren_depth += 1;
            i += 2;
        } else if b == b'(' {
            paren_depth += 1;
            current.push('(');
            i += 1;
        } else if b == b')' {
            if paren_depth > 0 {
                paren_depth -= 1;
            }
            current.push(')');
            i += 1;
        } else if b == b'\\' && i + 1 < bytes.len() {
            current.push('\\');
            i += 1;
            push_char_at(input, &mut current, &mut i);
        } else if paren_depth == 0 && b == b'|' {
            // Check for '||' – should not appear here (handled earlier), but
            // guard against it to avoid mis-splitting.
            if i + 1 < bytes.len() && bytes[i + 1] == b'|' {
                current.push('|');
                current.push('|');
                i += 2;
            } else {
                segments.push(current.trim().to_string());
                current.clear();
                i += 1;
            }
        } else {
            push_char_at(input, &mut current, &mut i);
        }
    }

    if !current.trim().is_empty() {
        segments.push(current.trim().to_string());
    }

    segments
}

fn run_single_command_list(
    input: &str,
    state: &mut ShellState,
    depth: usize,
    rustybox_mode: bool,
    aliases: &mut HashMap<String, String>,
    functions: &mut HashMap<String, String>,
    background: bool,
) -> u8 {
    // Enforce depth limit on all paths (including command substitution and
    // process substitution) to prevent stack overflow.
    if depth > MAX_FUNCTION_DEPTH {
        eprintln!("sh: maximum nesting depth exceeded");
        return 1;
    }

    // Replace <(...) and >(...) with /dev/fd/N.  Track file descriptors that
    // must be closed after the command finishes to avoid leaking them.
    let (processed, fds_to_close) =
        replace_process_substitution(input, state, depth, rustybox_mode, aliases, functions);

    let segments: Vec<String> = split_pipeline(&processed);
    let segments: Vec<&str> = segments.iter().map(|s| s.as_str()).collect();

    if segments.is_empty() {
        // Close any fds that were opened for unused process substitutions.
        for fd in fds_to_close {
            unsafe {
                libc::close(fd);
            }
        }
        return state.last_status;
    }

    if background {
        if !acquire_child_slot() {
            eprintln!("sh: too many child processes");
            for fd in fds_to_close {
                unsafe {
                    libc::close(fd);
                }
            }
            return 1;
        }

        let pid = unsafe { libc::fork() };
        if pid < 0 {
            eprintln!("sh: fork failed");
            release_child_slot();
            for fd in fds_to_close {
                unsafe {
                    libc::close(fd);
                }
            }
            return 1;
        }
        if pid == 0 {
            unsafe {
                // Child: create a new process group for proper job control.
                libc::setpgid(0, 0);
                // Reset per-process child counter (this process tracks its own).
                ACTIVE_CHILDREN.store(0, Ordering::SeqCst);
                let status = if segments.len() == 1 {
                    run_simple_command(segments[0], state, depth, rustybox_mode, aliases, functions)
                } else {
                    run_pipeline(&segments, state, depth, rustybox_mode, aliases, functions)
                };
                libc::_exit(status as i32)
            };
        }
        // Parent: set the child's process group (race-free: both sides call).
        unsafe {
            libc::setpgid(pid, pid);
        }
        // Parent: close our copy of the process-substitution fds.
        for fd in fds_to_close {
            unsafe {
                libc::close(fd);
            }
        }
        state.last_bg_pid = Some(pid);
        eprintln!("[{}] {}", pid, input);
        add_job(pid, pid, input.to_string(), JobState::Running);
        return state.last_status;
    }

    let status = if segments.len() == 1 {
        run_simple_command(segments[0], state, depth, rustybox_mode, aliases, functions)
    } else {
        run_pipeline(&segments, state, depth, rustybox_mode, aliases, functions)
    };

    // After the foreground command finishes, close any fds that were kept open.
    for fd in fds_to_close {
        unsafe {
            libc::close(fd);
        }
    }
    status
}

fn run_simple_command(
    input: &str,
    state: &mut ShellState,
    depth: usize,
    rustybox_mode: bool,
    aliases: &mut HashMap<String, String>,
    functions: &mut HashMap<String, String>,
) -> u8 {
    let tokens = tokenize(input);
    if tokens.is_empty() {
        return 0;
    }

    let tokens = expand_aliases_struct(tokens, aliases);
    let tokens = expand_tilde_struct(tokens);
    let env_cache: HashMap<String, String> = env::vars().collect();
    let tokens = expand_variables_struct(&tokens, state, &env_cache);

    let ifs = env::var("IFS").unwrap_or_else(|_| " \t\n".to_string());
    let final_tokens = expand_command_substitution_struct(
        tokens,
        state,
        depth,
        rustybox_mode,
        aliases,
        functions,
        &ifs,
    );

    // Parse redirections BEFORE globbing to detect ambiguous redirects correctly.
    let (cmd_tokens, stdin_file_token, stdout_file_token, append, _here_doc) =
        match parse_redirections_tokens(final_tokens) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("sh: {e}");
                return 1;
            }
        };

    let stdin_file = if let Some(f) = stdin_file_token {
        let expanded = expand_globs_struct(vec![f]);
        if expanded.len() != 1 {
            eprintln!("sh: ambiguous redirect or no match");
            return 1;
        }
        Some(expanded.into_iter().next().unwrap().value)
    } else {
        None
    };

    let stdout_file = if let Some(f) = stdout_file_token {
        let expanded = expand_globs_struct(vec![f]);
        if expanded.len() != 1 {
            eprintln!("sh: ambiguous redirect or no match");
            return 1;
        }
        Some(expanded.into_iter().next().unwrap().value)
    } else {
        None
    };

    let final_tokens = expand_globs_struct(cmd_tokens);

    // Convert to Vec<String> for execution helpers
    let exec_tokens: Vec<String> = final_tokens.into_iter().map(|t| t.value).collect();

    if exec_tokens.is_empty() {
        return 0;
    }

    if let Some(result) = handle_builtin(&exec_tokens, state, depth, aliases, functions) {
        return result;
    }

    if exec_tokens.len() >= 1 {
        if let Some(body) = functions.get(&exec_tokens[0]).cloned() {
            let status =
                run_command_list(&body, state, depth + 1, rustybox_mode, aliases, functions);
            return status;
        }
    }

    if rustybox_mode {
        if let Some(result) = handle_rustybox_builtin(&exec_tokens) {
            return result;
        }
    }

    run_external(&exec_tokens, stdin_file, stdout_file, append, rustybox_mode)
}

fn run_pipeline(
    segments: &[&str],
    state: &mut ShellState,
    depth: usize,
    rustybox_mode: bool,
    aliases: &mut HashMap<String, String>,
    functions: &mut HashMap<String, String>,
) -> u8 {
    let mut prev_read_fd: Option<RawFd> = None;
    let mut pids: Vec<i32> = Vec::new();
    let mut statuses: Vec<u8> = Vec::new();
    let pipefail = state.pipefail;

    let env_cache: HashMap<String, String> = env::vars().collect();
    let ifs = env::var("IFS").unwrap_or_else(|_| " \t\n".to_string());

    for (i, segment) in segments.iter().enumerate() {
        let tokens = tokenize(segment);
        if tokens.is_empty() {
            continue;
        }
        let tokens = expand_aliases_struct(tokens, aliases);
        let tokens = expand_tilde_struct(tokens);
        let tokens = expand_variables_struct(&tokens, state, &env_cache);

        let final_tokens = expand_command_substitution_struct(
            tokens,
            state,
            depth,
            rustybox_mode,
            aliases,
            functions,
            &ifs,
        );

        // Parse redirections BEFORE globbing.
        let (cmd_tokens, stdin_file_token, stdout_file_token, append, _) =
            match parse_redirections_tokens(final_tokens) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("sh: {e}");
                    return 1;
                }
            };

        let stdin_file = if let Some(f) = stdin_file_token {
            let expanded = expand_globs_struct(vec![f]);
            if expanded.len() != 1 {
                eprintln!("sh: ambiguous redirect or no match");
                return 1;
            }
            Some(expanded.into_iter().next().unwrap().value)
        } else {
            None
        };

        let stdout_file = if let Some(f) = stdout_file_token {
            let expanded = expand_globs_struct(vec![f]);
            if expanded.len() != 1 {
                eprintln!("sh: ambiguous redirect or no match");
                return 1;
            }
            Some(expanded.into_iter().next().unwrap().value)
        } else {
            None
        };

        let final_tokens = expand_globs_struct(cmd_tokens);
        let exec_tokens: Vec<String> = final_tokens.into_iter().map(|t| t.value).collect();

        if exec_tokens.is_empty() {
            continue;
        }

        // Reject all builtins that modify shell state – they would run in a
        // child process and their effects would be silently lost.
        if matches!(
            exec_tokens[0].as_str(),
            "cd" | "exit" | "exec" | "alias" | "export" | "set" | "unset" | "source" | "." | "eval"
        ) {
            eprintln!("sh: '{}' cannot be used in pipelines", exec_tokens[0]);
            return 1;
        }

        let (pipe_read, pipe_write) = if i != segments.len() - 1 {
            let fds = pipe_cloexec().unwrap();
            (Some(fds.0), Some(fds.1))
        } else {
            (None, None)
        };

        if !acquire_child_slot() {
            eprintln!("sh: too many child processes");
            // Kill already-forked children in this pipeline to avoid orphans.
            for &prev_pid in &pids {
                unsafe {
                    libc::kill(prev_pid, libc::SIGKILL);
                }
            }
            if let Some(r) = prev_read_fd {
                unsafe {
                    libc::close(r);
                }
            }
            if let Some(r) = pipe_read {
                unsafe {
                    libc::close(r);
                }
            }
            if let Some(w) = pipe_write {
                unsafe {
                    libc::close(w);
                }
            }
            // Wait for killed children to prevent zombies.
            for &prev_pid in &pids {
                let mut st: i32 = 0;
                unsafe {
                    libc::waitpid(prev_pid, &mut st, 0);
                }
                release_child_slot();
            }
            return 1;
        }

        let pid = unsafe { libc::fork() };
        if pid < 0 {
            eprintln!("sh: fork failed");
            release_child_slot();
            // Kill already-forked children.
            for &prev_pid in &pids {
                unsafe {
                    libc::kill(prev_pid, libc::SIGKILL);
                }
            }
            if let Some(r) = prev_read_fd {
                unsafe {
                    libc::close(r);
                }
            }
            if let Some(r) = pipe_read {
                unsafe {
                    libc::close(r);
                }
            }
            if let Some(w) = pipe_write {
                unsafe {
                    libc::close(w);
                }
            }
            for &prev_pid in &pids {
                let mut st: i32 = 0;
                unsafe {
                    libc::waitpid(prev_pid, &mut st, 0);
                }
                release_child_slot();
            }
            return 1;
        }

        if pid == 0 {
            unsafe {
                // Child: create a new process group for job control.
                libc::setpgid(0, 0);

                // Reset per-process child counter.
                ACTIVE_CHILDREN.store(0, Ordering::SeqCst);

                // Wire up the pipeline pipes
                if let Some(read_fd) = prev_read_fd {
                    libc::dup2(read_fd, libc::STDIN_FILENO);
                    libc::close(read_fd);
                }
                if let Some(write_fd) = pipe_write {
                    libc::dup2(write_fd, libc::STDOUT_FILENO);
                    libc::close(write_fd);
                }
                if let Some(read_fd) = pipe_read {
                    libc::close(read_fd);
                }

                // Handle explicit file redirections
                if let Some(f) = &stdin_file {
                    if let Ok(fd) = open_file_for_read(f) {
                        libc::dup2(fd, libc::STDIN_FILENO);
                        libc::close(fd);
                    } else {
                        libc::_exit(1);
                    }
                }
                if let Some(f) = &stdout_file {
                    if let Ok(fd) = open_file_for_write(f, append) {
                        libc::dup2(fd, libc::STDOUT_FILENO);
                        libc::close(fd);
                    } else {
                        libc::_exit(1);
                    }
                }

                // POSIX COMPLIANCE: Reset signal dispositions to default before exec.
                // This ensures the executed program handles SIGPIPE correctly
                // (terminates on broken pipe) instead of inheriting SIG_IGN,
                // which would cause CPU exhaustion (e.g., `cat /dev/urandom | head`).
                libc::signal(libc::SIGINT, libc::SIG_DFL);
                libc::signal(libc::SIGQUIT, libc::SIG_DFL);
                libc::signal(libc::SIGPIPE, libc::SIG_DFL);

                if rustybox_mode {
                    if let Some(def) = registry::find(&exec_tokens[0]) {
                        let argv: Vec<String> = exec_tokens.to_vec();
                        let mut ctx = Context::new(def, argv);
                        let code = (def.run)(&mut ctx);
                        libc::_exit(code as i32);
                    }
                }

                let (args, argv) = match build_argv(&exec_tokens) {
                    Some(v) => v,
                    None => {
                        eprintln!("sh: invalid argument (null byte)");
                        libc::_exit(1);
                    }
                };

                libc::execvp(argv[0], argv.as_ptr());

                // If execvp returns, it failed
                eprintln!("sh: {}: command not found", exec_tokens[0]);
                drop(args);
                libc::_exit(127);
            }
        }
        // Parent: set child's process group (race-free with child's setpgid).
        unsafe {
            libc::setpgid(pid, pid);
        }

        if let Some(write_fd) = pipe_write {
            unsafe {
                libc::close(write_fd);
            }
        }
        if let Some(read_fd) = prev_read_fd {
            unsafe {
                libc::close(read_fd);
            }
        }
        prev_read_fd = pipe_read;
        pids.push(pid);
    }

    if let Some(read_fd) = prev_read_fd {
        unsafe {
            libc::close(read_fd);
        }
    }

    for &pid in &pids {
        let status = wait_for_child(pid);
        release_child_slot();
        statuses.push(status);
    }

    if pipefail {
        for s in &statuses {
            if *s != 0 {
                return *s;
            }
        }
        0
    } else {
        *statuses.last().unwrap_or(&0)
    }
}

fn run_external(
    tokens: &[String],
    stdin_file: Option<String>,
    stdout_file: Option<String>,
    append: bool,
    rustybox_mode: bool,
) -> u8 {
    if !acquire_child_slot() {
        eprintln!("sh: too many child processes");
        return 1;
    }

    let pid = unsafe { libc::fork() };
    if pid < 0 {
        eprintln!("sh: fork failed");
        release_child_slot();
        return 1;
    }

    if pid == 0 {
        unsafe {
            // Child: reset per-process child counter.
            ACTIVE_CHILDREN.store(0, Ordering::SeqCst);

            if let Some(f) = &stdin_file {
                if let Ok(fd) = open_file_for_read(f) {
                    libc::dup2(fd, libc::STDIN_FILENO);
                    libc::close(fd);
                } else {
                    libc::_exit(1);
                }
            }
            if let Some(f) = &stdout_file {
                if let Ok(fd) = open_file_for_write(f, append) {
                    libc::dup2(fd, libc::STDOUT_FILENO);
                    libc::close(fd);
                } else {
                    libc::_exit(1);
                }
            }

            libc::signal(libc::SIGINT, libc::SIG_DFL);
            libc::signal(libc::SIGQUIT, libc::SIG_DFL);

            if rustybox_mode {
                if let Some(def) = registry::find(&tokens[0]) {
                    let argv: Vec<String> = tokens.to_vec();
                    let mut ctx = Context::new(def, argv);
                    let code = (def.run)(&mut ctx);
                    libc::_exit(code as i32);
                }
            }

            let (args, argv) = match build_argv(tokens) {
                Some(v) => v,
                None => {
                    eprintln!("sh: invalid argument (null byte)");
                    libc::_exit(1);
                }
            };
            libc::execvp(argv[0], argv.as_ptr());
            eprintln!("sh: {}: command not found", tokens[0]);
            drop(args);
            libc::_exit(127);
        }
    }

    let status = wait_for_child(pid);
    release_child_slot();
    status
}

// -----------------------------------------------------------------------------
// Wait for child, handling signals and job control.
// -----------------------------------------------------------------------------
fn wait_for_child(pid: i32) -> u8 {
    loop {
        let mut status: i32 = 0;
        let ret = unsafe { libc::waitpid(pid, &mut status, libc::WUNTRACED) };
        if ret < 0 {
            let err = io::Error::last_os_error();
            if err.kind() == io::ErrorKind::Interrupted {
                if SIGINT_RECEIVED.swap(false, Ordering::SeqCst) {
                    // Send to process group to avoid PID recycling race conditions.
                    unsafe {
                        libc::kill(-pid, libc::SIGINT);
                    }
                }
                if SIGTSTP_RECEIVED.swap(false, Ordering::SeqCst) {
                    unsafe {
                        libc::kill(-pid, libc::SIGTSTP);
                    }
                    update_job_state(pid, JobState::Stopped);
                    eprintln!("\n[{}] + stopped", pid);
                    let sig = unsafe { libc::WSTOPSIG(status) };
                    return 128u8.saturating_add(sig as u8);
                }
                continue;
            }
            return 1;
        }
        if libc::WIFEXITED(status) {
            let code = libc::WEXITSTATUS(status) as u8;
            remove_job(pid);
            return code;
        }
        if libc::WIFSIGNALED(status) {
            let sig = libc::WTERMSIG(status);
            if sig == libc::SIGTSTP {
                update_job_state(pid, JobState::Stopped);
            }
            remove_job(pid);
            // Use saturating arithmetic to prevent u8 overflow on exotic
            // platforms with signal numbers > 127.
            return 128u8.saturating_add(sig as u8);
        }
        if libc::WIFSTOPPED(status) {
            let sig = libc::WSTOPSIG(status);
            update_job_state(pid, JobState::Stopped);
            return 128u8.saturating_add(sig as u8);
        }
        return 1;
    }
}

/// Reap finished child processes (both background jobs and process substitutions).
/// This function is called periodically (e.g. before prompting) and ensures
/// that child-process slots are released for all reaped children, preventing
/// resource exhaustion (fork-bomb mitigation).
fn reap_background() {
    // First, reap process substitution children that have already completed.
    {
        let mut proc_pids = PROC_SUB_PIDS.lock().unwrap();
        let mut i = 0;
        while i < proc_pids.len() {
            let pid = proc_pids[i];
            let mut status: i32 = 0;
            let ret = unsafe { libc::waitpid(pid, &mut status, libc::WNOHANG) };
            if ret > 0 {
                release_child_slot();
                proc_pids.remove(i);
            } else if ret < 0 {
                // Error or already reaped
                proc_pids.remove(i);
            } else {
                i += 1;
            }
        }
    }

    // Then reap any other background jobs (including those from JOBS) and also
    // catch any process-substitution children that may have exited after the
    // first loop (to avoid leaking slots).
    loop {
        let mut status: i32 = 0;
        let pid = unsafe { libc::waitpid(-1, &mut status, libc::WNOHANG | libc::WUNTRACED) };
        if pid <= 0 {
            break;
        }

        let mut handled = false;

        // Check if it's a regular job from the job table.
        if JOBS.lock().unwrap().iter().any(|j| j.pid == pid) {
            if libc::WIFEXITED(status) || libc::WIFSIGNALED(status) {
                eprintln!("[{}] Done", pid);
                remove_job(pid);
                release_child_slot();
                handled = true;
            } else if libc::WIFSTOPPED(status) {
                update_job_state(pid, JobState::Stopped);
                handled = true;
            }
        }

        // If not handled as a regular job, check if it's a process substitution.
        if !handled {
            let mut proc_pids = PROC_SUB_PIDS.lock().unwrap();
            if let Some(pos) = proc_pids.iter().position(|&p| p == pid) {
                proc_pids.remove(pos);
                release_child_slot();
                handled = true;
            }
        }

        // If the PID was not found in either table, it might have already been
        // removed (e.g., double wait), but we still need to ensure the slot is
        // freed if it was ever allocated.  Since we cannot know, we conservatively
        // attempt to release a slot only if the process was waited for successfully.
        // However, we must avoid underflow.  We assume that any child that
        // waitpid returns for was previously counted, so we can safely release.
        // But to be safe, we check if the PID is known to us; if not, we still
        // release a slot to prevent leaks (the counter will not go negative
        // because we only release if we previously acquired).
        if !handled {
            // This case should be rare; we release a slot to avoid leaks.
            release_child_slot();
        }
    }
}

// -----------------------------------------------------------------------------
// Job control builtins helpers.
// -----------------------------------------------------------------------------
fn add_job(pid: i32, pgid: i32, cmd: String, state: JobState) {
    let mut jobs = JOBS.lock().unwrap();
    jobs.push(Job {
        pid,
        pgid,
        command: cmd,
        state,
    });
}

fn remove_job(pid: i32) {
    let mut jobs = JOBS.lock().unwrap();
    jobs.retain(|j| j.pid != pid);
}

fn update_job_state(pid: i32, state: JobState) {
    if let Some(job) = JOBS.lock().unwrap().iter_mut().find(|j| j.pid == pid) {
        job.state = state;
    }
}

/// Look up a job by PID.  Returns true only if the PID is present in the
/// job table AND the process is still alive (kill(pid, 0) succeeds),
/// preventing signals from being sent to recycled PIDs.
fn job_exists_and_alive(pid: i32) -> bool {
    let in_table = JOBS.lock().unwrap().iter().any(|j| j.pid == pid);
    if !in_table {
        return false;
    }
    // Verify the process is still alive to guard against PID recycling.
    unsafe { libc::kill(pid, 0) == 0 }
}

/// Return the PID of the most recently added job (the "current" job).
fn current_job_pid() -> Option<i32> {
    JOBS.lock().unwrap().last().map(|j| j.pid)
}

/// Return the PGID of a job by PID.
fn job_pgid(pid: i32) -> Option<i32> {
    JOBS.lock()
        .unwrap()
        .iter()
        .find(|j| j.pid == pid)
        .map(|j| j.pgid)
}

fn jobs_builtin() -> u8 {
    let jobs = JOBS.lock().unwrap();
    for job in jobs.iter() {
        let status = match job.state {
            JobState::Running => "Running",
            JobState::Stopped => "Stopped",
        };
        println!("[{}] {} - {}", job.pid, status, job.command);
    }
    0
}

fn fg_builtin(pid: Option<i32>) -> u8 {
    // Acquire the job table lock for the whole operation to narrow the PID
    // recycling window (though not completely eliminating it on all platforms).
    let jobs = JOBS.lock().unwrap();

    let target_pid = match pid {
        Some(p) => {
            if !jobs
                .iter()
                .any(|j| j.pid == p && unsafe { libc::kill(p, 0) == 0 })
            {
                eprintln!("fg: {}: no such job", p);
                return 1;
            }
            p
        }
        None => match jobs.last() {
            Some(j) if unsafe { libc::kill(j.pid, 0) == 0 } => j.pid,
            _ => {
                eprintln!("fg: no current job");
                return 1;
            }
        },
    };

    let pgid = jobs
        .iter()
        .find(|j| j.pid == target_pid)
        .map(|j| j.pgid)
        .unwrap_or(target_pid);

    // Drop the lock before performing blocking wait, but keep the pgid.
    // On Linux, a pidfd could be used for stronger guarantee; here we accept
    // a residual race.
    drop(jobs);

    // Re-check that the process is still alive before proceeding.
    if unsafe { libc::kill(target_pid, 0) } != 0 {
        eprintln!("fg: process {} terminated", target_pid);
        return 1;
    }

    unsafe {
        libc::signal(libc::SIGTTOU, libc::SIG_IGN);
        libc::tcsetpgrp(libc::STDIN_FILENO, pgid);
    }
    update_job_state(target_pid, JobState::Running);
    unsafe {
        libc::kill(-pgid, libc::SIGCONT);
    }
    let status = wait_for_child(target_pid);
    unsafe {
        libc::tcsetpgrp(libc::STDIN_FILENO, libc::getpgrp());
        libc::signal(libc::SIGTTOU, libc::SIG_DFL);
    }
    status
}

fn bg_builtin(pid: Option<i32>) -> u8 {
    let jobs = JOBS.lock().unwrap();

    let target_pid = match pid {
        Some(p) => {
            if !jobs
                .iter()
                .any(|j| j.pid == p && unsafe { libc::kill(p, 0) == 0 })
            {
                eprintln!("bg: {}: no such job", p);
                return 1;
            }
            p
        }
        None => match jobs.last() {
            Some(j) if unsafe { libc::kill(j.pid, 0) == 0 } => j.pid,
            _ => {
                eprintln!("bg: no current job");
                return 1;
            }
        },
    };

    let pgid = jobs
        .iter()
        .find(|j| j.pid == target_pid)
        .map(|j| j.pgid)
        .unwrap_or(target_pid);

    drop(jobs);

    if unsafe { libc::kill(target_pid, 0) } != 0 {
        eprintln!("bg: process {} terminated", target_pid);
        return 1;
    }

    update_job_state(target_pid, JobState::Running);
    unsafe {
        libc::kill(-pgid, libc::SIGCONT);
    }
    eprintln!("[{}] continued in background", target_pid);
    0
}

// -----------------------------------------------------------------------------
// Builtin commands.
// -----------------------------------------------------------------------------
fn handle_builtin(
    tokens: &[String],
    state: &mut ShellState,
    depth: usize,
    aliases: &mut HashMap<String, String>,
    functions: &mut HashMap<String, String>,
) -> Option<u8> {
    if tokens.is_empty() {
        return Some(0);
    }

    match tokens[0].as_str() {
        "cd" => {
            let dir = if tokens.len() > 1 {
                expand_tilde_one(&tokens[1])
            } else {
                env::var("HOME").unwrap_or_else(|_| ".".into())
            };
            Some(cd_builtin(&dir))
        }
        "exit" => {
            let code = if tokens.len() > 1 {
                tokens[1].parse::<u8>().unwrap_or(0)
            } else {
                state.last_status
            };
            // Instead of calling std::process::exit (which skips destructors
            // and history saving), set a flag that the main loop checks.
            state.exit_requested = true;
            state.exit_code = code;
            Some(code)
        }
        "exec" => {
            if tokens.len() < 2 {
                eprintln!("exec: missing command");
                return Some(1);
            }
            let (args, argv) = match build_argv(&tokens[1..]) {
                Some(v) => v,
                None => {
                    eprintln!("exec: invalid argument (null byte)");
                    return Some(1);
                }
            };
            release_child_slot();
            unsafe {
                libc::execvp(argv[0], argv.as_ptr());
            }
            eprintln!("exec: command not found");
            drop(args);
            unsafe {
                libc::_exit(127);
            }
        }
        "export" => {
            for arg in &tokens[1..] {
                if let Some((key, value)) = arg.split_once('=') {
                    // Validate that the key is a legal POSIX identifier before
                    // setting it in the environment.
                    if !is_valid_identifier(key) {
                        eprintln!("export: '{}': not a valid identifier", key);
                        return Some(1);
                    }
                    // SAFETY: we assume single-threaded access to environ at
                    // this point (no concurrent readers in the shell process).
                    unsafe { env::set_var(key, value) };
                } else {
                    // Bare name: mark existing variable for export (no-op in
                    // this implementation since all env vars are inherited).
                    if !is_valid_identifier(arg) {
                        eprintln!("export: '{}': not a valid identifier", arg);
                        return Some(1);
                    }
                    if env::var(arg).is_err() {
                        eprintln!("export: {}: not a valid identifier", arg);
                        return Some(1);
                    }
                }
            }
            Some(0)
        }
        "pwd" => match env::current_dir() {
            Ok(p) => {
                println!("{}", p.display());
                Some(0)
            }
            Err(e) => {
                eprintln!("pwd: {e}");
                Some(1)
            }
        },
        "alias" => {
            if tokens.len() == 1 {
                for (name, value) in aliases.iter() {
                    println!("alias {}='{}'", name, value);
                }
            } else {
                for arg in &tokens[1..] {
                    if let Some((name, value)) = arg.split_once('=') {
                        aliases.insert(name.to_string(), value.trim_matches('\'').to_string());
                    } else if let Some(value) = aliases.get(arg) {
                        println!("alias {}='{}'", arg, value);
                    } else {
                        eprintln!("alias: {}: not found", arg);
                        return Some(1);
                    }
                }
            }
            Some(0)
        }
        "jobs" => Some(jobs_builtin()),
        "fg" => {
            let pid = tokens.get(1).and_then(|s| s.parse::<i32>().ok());
            Some(fg_builtin(pid))
        }
        "bg" => {
            let pid = tokens.get(1).and_then(|s| s.parse::<i32>().ok());
            Some(bg_builtin(pid))
        }
        "source" | "." => {
            if tokens.len() < 2 {
                eprintln!("{}: missing filename", tokens[0]);
                return Some(1);
            }
            let filename = expand_tilde_one(&tokens[1]);

            // Open with O_NOFOLLOW to prevent symlink following attacks.
            let file = match OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NOFOLLOW)
                .open(&filename)
            {
                Ok(f) => f,
                Err(e) => {
                    eprintln!("{}: {}: {}", tokens[0], filename, e);
                    return Some(1);
                }
            };

            // Verify it's a regular file.
            let meta = match file.metadata() {
                Ok(m) => m,
                Err(e) => {
                    eprintln!("{}: {}: {}", tokens[0], filename, e);
                    return Some(1);
                }
            };
            if !meta.is_file() {
                eprintln!("{}: {}: not a regular file", tokens[0], filename);
                return Some(126);
            }

            // Read with a hard limit to prevent TOCTOU.
            let mut content = String::with_capacity(4096);
            {
                let mut reader = BufReader::new(file.take(MAX_SCRIPT_BYTES as u64));
                if let Err(e) = reader.read_to_string(&mut content) {
                    eprintln!("{}: {}: {}", tokens[0], filename, e);
                    return Some(1);
                }
            }

            // Pass functions by mutable reference so that definitions
            // made inside the sourced file persist in the caller.
            let status = run_command_list(&content, state, depth + 1, false, aliases, functions);
            Some(status)
        }
        "eval" => {
            // POSIX: eval concatenates its arguments with spaces and re-parses
            // the result as a shell command.  This is intentional and expected
            // behaviour – the caller is responsible for proper quoting.
            let args = tokens[1..].join(" ");
            if args.len() > MAX_SCRIPT_BYTES {
                eprintln!("eval: command too long");
                return Some(1);
            }
            let status = run_command_list(&args, state, depth + 1, false, aliases, functions);
            Some(status)
        }
        "set" => {
            if tokens.len() > 1 {
                let mut idx = 1;
                while idx < tokens.len() {
                    let arg = &tokens[idx];
                    if arg == "-o" || arg == "+o" {
                        let enable = arg.starts_with('-');
                        if let Some(opt) = tokens.get(idx + 1) {
                            match opt.as_str() {
                                "pipefail" => {
                                    let shellopts = env::var("SHELLOPTS").unwrap_or_default();
                                    if enable {
                                        let new_val = if shellopts.is_empty() {
                                            "pipefail".to_string()
                                        } else if !shellopts.contains("pipefail") {
                                            format!("{} pipefail", shellopts)
                                        } else {
                                            shellopts
                                        };
                                        unsafe {
                                            env::set_var("SHELLOPTS", new_val);
                                        }
                                        state.pipefail = true;
                                    } else {
                                        let new_val = shellopts
                                            .split_whitespace()
                                            .filter(|w| *w != "pipefail")
                                            .collect::<Vec<_>>()
                                            .join(" ");
                                        unsafe {
                                            env::set_var("SHELLOPTS", new_val);
                                        }
                                        state.pipefail = false;
                                    }
                                }
                                _ => eprintln!("set: unknown option {}", opt),
                            }
                            idx += 1;
                        }
                    }
                    idx += 1;
                }
            } else {
                // Print shell variables and environment.
                println!("SHELL_STATE:");
                println!("  last_status={}", state.last_status);
                println!("  script_name={}", state.script_name);
                println!("  pipefail={}", state.pipefail);
                println!("  positional_params={:?}", state.positional_params);
                if let Some(bg) = state.last_bg_pid {
                    println!("  last_bg_pid={}", bg);
                }
                println!("ENVIRONMENT:");
                let mut vars: Vec<(String, String)> = env::vars().collect();
                vars.sort_by(|a, b| a.0.cmp(&b.0));
                for (k, v) in vars {
                    println!("{}={}", k, v);
                }
            }
            Some(0)
        }
        "unset" => {
            for arg in &tokens[1..] {
                if !is_valid_identifier(arg) {
                    eprintln!("unset: '{}': not a valid identifier", arg);
                    return Some(1);
                }
                unsafe { env::remove_var(arg) };
            }
            Some(0)
        }
        _ => None,
    }
}

fn handle_rustybox_builtin(tokens: &[String]) -> Option<u8> {
    if tokens.is_empty() {
        return None;
    }
    if matches!(
        tokens[0].as_str(),
        "cd" | "exit" | "exec" | "sh" | "export" | "pwd" | "alias"
    ) {
        return None;
    }
    let def = registry::find(&tokens[0])?;
    let argv: Vec<String> = tokens.to_vec();
    let mut ctx = Context::new(def, argv);
    let code = (def.run)(&mut ctx);
    Some(code)
}

fn cd_builtin(dir: &str) -> u8 {
    let target = if dir == "-" {
        match env::var("OLDPWD") {
            Ok(old) => {
                println!("{old}");
                old
            }
            Err(_) => {
                eprintln!("cd: OLDPWD not set");
                return 1;
            }
        }
    } else {
        resolve_cdpath(dir)
    };

    let old_pwd = env::current_dir()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();

    if let Err(e) = env::set_current_dir(&target) {
        eprintln!("cd: {}: {}", target, e);
        return 1;
    }

    let new_pwd = env::current_dir()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or(target);
    unsafe { env::set_var("OLDPWD", old_pwd) };
    unsafe { env::set_var("PWD", &new_pwd) };
    0
}

fn resolve_cdpath(dir: &str) -> String {
    let p = Path::new(dir);
    if p.is_absolute() || p.starts_with(".") {
        return dir.to_string();
    }
    let cdpath = env::var("CDPATH").unwrap_or_default();
    if cdpath.is_empty() {
        return dir.to_string();
    }
    for entry in cdpath.split(':') {
        let candidate = if entry.is_empty() {
            Path::new(dir).to_path_buf()
        } else {
            Path::new(entry).join(dir)
        };
        if candidate.is_dir() {
            return candidate.to_string_lossy().into_owned();
        }
    }
    dir.to_string()
}

// -----------------------------------------------------------------------------
// Tokenizer and expansions.
// -----------------------------------------------------------------------------

/// Represents a shell token with metadata about its quoting status.
/// This allows subsequent expansion phases to respect POSIX quoting rules.
#[derive(Debug, Clone)]
struct Token {
    value: String,
    /// True if the token was entirely enclosed in single quotes ('...').
    /// Single-quoted tokens must NOT undergo any expansion.
    is_single_quoted: bool,
    /// True if the token was entirely enclosed in double quotes ("...").
    /// Double-quoted tokens must NOT undergo word splitting or globbing.
    is_double_quoted: bool,
    /// True if the token started with a backslash escape (\).
    /// Escaped tokens should not undergo globbing or command substitution.
    is_escaped: bool,
}

fn tokenize(input: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let bytes = input.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }

        let mut current = String::with_capacity(64);
        let mut has_single = false;
        let mut has_double = false;
        let mut has_unquoted = false;
        let mut is_escaped = false;

        let mut in_single = false;
        let mut in_double = false;

        while i < bytes.len() && !bytes[i].is_ascii_whitespace() {
            let b = bytes[i];

            if in_single {
                push_char_at(input, &mut current, &mut i);
                if b == b'\'' {
                    in_single = false;
                }
            } else if in_double {
                if b == b'"' {
                    in_double = false;
                    i += 1;
                } else if b == b'\\' && i + 1 < bytes.len() {
                    // POSIX: inside double quotes, backslash is special only before $ ` " \ newline.
                    // We preserve the backslash so expansion can handle it.
                    match bytes[i + 1] {
                        b'$' | b'`' | b'"' | b'\\' | b'\n' => {
                            current.push('\\');
                            i += 1;
                            push_char_at(input, &mut current, &mut i);
                        }
                        _ => {
                            // Consume backslash for other characters
                            i += 1;
                            push_char_at(input, &mut current, &mut i);
                        }
                    }
                } else {
                    push_char_at(input, &mut current, &mut i);
                }
            } else {
                // Unquoted context
                if b == b'\'' {
                    has_single = true;
                    in_single = true;
                    i += 1;
                } else if b == b'"' {
                    has_double = true;
                    in_double = true;
                    i += 1;
                } else if b == b'\\' && i + 1 < bytes.len() {
                    // Preserve backslash if it escapes $ or ` so expansion can handle it.
                    // For other characters, consume it and mark token as escaped.
                    if bytes[i + 1] == b'$' || bytes[i + 1] == b'`' {
                        current.push('\\');
                        i += 1;
                        push_char_at(input, &mut current, &mut i);
                    } else {
                        is_escaped = true;
                        i += 1;
                        push_char_at(input, &mut current, &mut i);
                    }
                } else if b == b'#' && current.is_empty() && !has_single && !has_double {
                    return tokens; // Comment
                } else {
                    has_unquoted = true;
                    push_char_at(input, &mut current, &mut i);
                }
            }
        }

        if !current.is_empty() || has_single || has_double {
            let final_is_double = has_double && !has_unquoted && !has_single;
            let final_is_single = has_single && !has_unquoted && !has_double;

            tokens.push(Token {
                value: current,
                is_single_quoted: final_is_single,
                is_double_quoted: final_is_double,
                is_escaped: is_escaped && !final_is_single && !final_is_double,
            });
        }
    }
    tokens
}

/// Helper to convert Tokens back to Strings for legacy interfaces where quote info isn't needed
/// or has already been processed.
fn tokens_to_strings(tokens: Vec<Token>) -> Vec<String> {
    tokens.into_iter().map(|t| t.value).collect()
}

/// Helper to convert Strings to Tokens (unquoted, unescaped) for legacy interfaces.
fn strings_to_tokens(strings: Vec<String>) -> Vec<Token> {
    strings
        .into_iter()
        .map(|s| Token {
            value: s,
            is_single_quoted: false,
            is_double_quoted: false,
            is_escaped: false,
        })
        .collect()
}

/// Legacy tokenizer that returns just strings, used for completion and simple parsing.
fn tokenize_to_strings(input: &str) -> Vec<String> {
    tokens_to_strings(tokenize(input))
}

/// Expand aliases without infinite recursion using a tracking set.
fn expand_aliases_struct(tokens: Vec<Token>, aliases: &HashMap<String, String>) -> Vec<Token> {
    let mut expanded_set = HashSet::new();
    expand_aliases_recursive(tokens, aliases, &mut expanded_set)
}

fn expand_aliases_recursive(
    mut tokens: Vec<Token>,
    aliases: &HashMap<String, String>,
    expanded: &mut HashSet<String>,
) -> Vec<Token> {
    if tokens.is_empty() {
        return tokens;
    }
    // Only expand the first token if it's not quoted
    if !tokens[0].is_single_quoted && !tokens[0].is_double_quoted && !tokens[0].is_escaped {
        let name = &tokens[0].value;
        if !expanded.contains(name) {
            if let Some(expansion) = aliases.get(name) {
                expanded.insert(name.clone());
                let mut new_tokens = tokenize(expansion);
                if !new_tokens.is_empty() {
                    // Recursively expand the new first token
                    new_tokens.extend_from_slice(&tokens[1..]);
                    return expand_aliases_recursive(new_tokens, aliases, expanded);
                }
            }
        }
    }
    tokens
}

/// Legacy alias expander for string vectors.
fn expand_aliases(tokens: Vec<String>, aliases: &HashMap<String, String>) -> Vec<String> {
    tokens_to_strings(expand_aliases_struct(strings_to_tokens(tokens), aliases))
}

fn expand_tilde_struct(tokens: Vec<Token>) -> Vec<Token> {
    tokens
        .into_iter()
        .map(|t| {
            if t.is_single_quoted || t.is_double_quoted || t.is_escaped {
                t
            } else {
                Token {
                    value: expand_tilde_one(&t.value),
                    ..t
                }
            }
        })
        .collect()
}

fn expand_tilde(tokens: Vec<String>) -> Vec<String> {
    tokens_to_strings(expand_tilde_struct(strings_to_tokens(tokens)))
}

fn expand_tilde_one(s: &str) -> String {
    if s.starts_with('~') {
        if s == "~" {
            return env::var("HOME").unwrap_or_else(|_| "~".into());
        }
        if s.starts_with("~/") {
            return env::var("HOME").unwrap_or_else(|_| "~".into()) + &s[1..];
        }
        if let Some((user, rest)) = s[1..].split_once('/') {
            // Validate the username to prevent path traversal via crafted
            // user names (e.g. "~../../etc/passwd").
            if user
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
                && !user.is_empty()
            {
                // Try getpwnam for a correct home directory lookup.
                if let Some(home) = lookup_user_home(user) {
                    // Security fix: ensure the final resolved path does not escape the home
                    // directory by canonicalising both and checking the prefix.
                    if let Ok(canon_home) = Path::new(&home).canonicalize() {
                        let joined = Path::new(&home).join(rest);
                        if let Ok(canon_path) = joined.canonicalize() {
                            if canon_path.starts_with(&canon_home) {
                                return canon_path.to_string_lossy().into_owned();
                            }
                        }
                    }
                    // If canonicalisation fails or the path escapes, fall through to the
                    // original string without expansion.
                    return s.to_string();
                }
                // Fallback: guess /home/<user>.
                let guess = format!("/home/{}", user);
                if Path::new(&guess).is_dir() {
                    let joined = Path::new(&guess).join(rest);
                    if let (Ok(canon_home), Ok(canon_path)) =
                        (Path::new(&guess).canonicalize(), joined.canonicalize())
                    {
                        if canon_path.starts_with(&canon_home) {
                            return canon_path.to_string_lossy().into_owned();
                        }
                    }
                }
            }
        }
    }
    s.to_string()
}

/// Look up a user's home directory via getpwnam_r (thread-safe).
/// Returns None if the user does not exist or the lookup fails.
fn lookup_user_home(user: &str) -> Option<String> {
    let c_user = safe_cstring(user)?;
    let mut pw: libc::passwd = unsafe { std::mem::zeroed() };
    let mut buf = vec![0u8; 4096]; // 4KB buffer
    let mut pw_ptr: *mut libc::passwd = std::ptr::null_mut();

    let ret = unsafe {
        libc::getpwnam_r(
            c_user.as_ptr(),
            &mut pw,
            buf.as_mut_ptr() as *mut libc::c_char,
            buf.len(),
            &mut pw_ptr,
        )
    };

    if ret != 0 || pw_ptr.is_null() {
        return None;
    }
    let dir_ptr = pw.pw_dir;
    if dir_ptr.is_null() {
        return None;
    }
    let dir = unsafe { CStr::from_ptr(dir_ptr) }
        .to_string_lossy()
        .into_owned();
    if dir.is_empty() { None } else { Some(dir) }
}

/// Expand `$?`, `$#`, `$N`, `$$`, `$!`, `$@`, `$*` positional parameters
/// using `ShellState`.  Environment variables are also expanded from the
/// provided cache.
fn expand_variables_struct(
    tokens: &[Token],
    state: &ShellState,
    env_cache: &HashMap<String, String>,
) -> Vec<Token> {
    tokens
        .iter()
        .map(|token| {
            if token.is_single_quoted {
                return token.clone();
            }

            let mut result = String::with_capacity(token.value.len());
            let bytes = token.value.as_bytes();
            let mut i = 0;

            while i < bytes.len() {
                // Handle backslash escaping of special characters
                if bytes[i] == b'\\' && i + 1 < bytes.len() {
                    let next = bytes[i + 1];
                    if next == b'$' || next == b'`' || next == b'"' || next == b'\\' {
                        result.push(next as char);
                        i += 2;
                        continue;
                    }
                }

                if !token.is_escaped && bytes[i] == b'$' && i + 1 < bytes.len() {
                    match bytes[i + 1] {
                        b'?' => {
                            use std::fmt::Write;
                            write!(result, "{}", state.last_status).unwrap();
                            i += 2;
                            continue;
                        }
                        b'#' => {
                            let count = state.positional_params.len();
                            result.push_str(&count.to_string());
                            i += 2;
                            continue;
                        }
                        b'$' => {
                            // Shell PID.
                            result.push_str(&std::process::id().to_string());
                            i += 2;
                            continue;
                        }
                        b'!' => {
                            // Last background PID.
                            if let Some(pid) = state.last_bg_pid {
                                result.push_str(&pid.to_string());
                            }
                            i += 2;
                            continue;
                        }
                        b'@' | b'*' => {
                            // All positional parameters joined by space.
                            result.push_str(&state.positional_params.join(" "));
                            i += 2;
                            continue;
                        }
                        b'0' => {
                            result.push_str(&state.script_name);
                            i += 2;
                            continue;
                        }
                        d if d.is_ascii_digit() => {
                            let idx = (d - b'0') as usize;
                            if idx > 0 && idx <= state.positional_params.len() {
                                result.push_str(&state.positional_params[idx - 1]);
                            }
                            i += 2;
                            continue;
                        }
                        b'{' => {
                            if let Some(end) = token.value[i + 2..].find('}') {
                                let var_name = &token.value[i + 2..i + 2 + end];
                                result.push_str(
                                    &env_cache.get(var_name).cloned().unwrap_or_default(),
                                );
                                i += 3 + end;
                                continue;
                            }
                        }
                        _ if bytes[i + 1].is_ascii_alphabetic() || bytes[i + 1] == b'_' => {
                            let start = i + 1;
                            let mut end = start;
                            while end < bytes.len()
                                && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_')
                            {
                                end += 1;
                            }
                            let var_name = std::str::from_utf8(&bytes[start..end]).unwrap();
                            result.push_str(&env_cache.get(var_name).cloned().unwrap_or_default());
                            i = end;
                            continue;
                        }
                        _ => {}
                    }
                }

                push_char_at(&token.value, &mut result, &mut i);
            }

            Token {
                value: result,
                is_single_quoted: false,
                // Preserve double quote state to prevent word splitting/globbing
                is_double_quoted: token.is_double_quoted,
                is_escaped: false,
            }
        })
        .collect()
}

/// Legacy variable expander for string vectors.
fn expand_variables(
    tokens: &[String],
    state: &ShellState,
    env_cache: &HashMap<String, String>,
) -> Vec<String> {
    tokens_to_strings(expand_variables_struct(
        &strings_to_tokens(tokens.to_vec()),
        state,
        env_cache,
    ))
}

/// Split string by IFS characters for word splitting (POSIX compliant).
/// Whitespace characters in IFS are collapsed, non-whitespace are not.
fn split_by_ifs(s: &str, ifs: &str) -> Vec<String> {
    if ifs.is_empty() {
        return vec![s.to_string()];
    }

    let mut ifs_ws = Vec::new();
    let mut ifs_non_ws = Vec::new();
    for c in ifs.chars() {
        if c.is_ascii_whitespace() {
            ifs_ws.push(c);
        } else {
            ifs_non_ws.push(c);
        }
    }

    let mut result = Vec::new();
    let mut current = String::new();
    let mut chars = s.chars().peekable();

    // Skip leading IFS whitespace
    while let Some(&c) = chars.peek() {
        if ifs_ws.contains(&c) {
            chars.next();
        } else {
            break;
        }
    }

    while let Some(c) = chars.next() {
        if ifs_non_ws.contains(&c) {
            result.push(current.clone());
            current.clear();
            // Skip trailing IFS whitespace after non-whitespace separator
            while let Some(&next_c) = chars.peek() {
                if ifs_ws.contains(&next_c) {
                    chars.next();
                } else {
                    break;
                }
            }
        } else if ifs_ws.contains(&c) {
            if !current.is_empty() {
                result.push(current.clone());
                current.clear();
            }
            // Skip consecutive whitespace
            while let Some(&next_c) = chars.peek() {
                if ifs_ws.contains(&next_c) {
                    chars.next();
                } else {
                    break;
                }
            }
        } else {
            current.push(c);
        }
    }

    if !current.is_empty() {
        result.push(current);
    }

    result
}

/// Expand command substitutions ($(…) and `…`) at the token level.
/// Respects POSIX quoting: if the token is double-quoted, the result is NOT
/// word-split and NOT globbed.
fn expand_command_substitution_struct(
    tokens: Vec<Token>,
    state: &mut ShellState,
    depth: usize,
    rustybox_mode: bool,
    aliases: &mut HashMap<String, String>,
    functions: &mut HashMap<String, String>,
    ifs: &str,
) -> Vec<Token> {
    let mut result = Vec::new();
    for token in tokens {
        if token.is_single_quoted || token.is_escaped {
            result.push(token);
            continue;
        }

        let expanded_string = expand_command_substitution_in_token_legacy(
            &token.value,
            state,
            depth,
            rustybox_mode,
            aliases,
            functions,
        );

        if token.is_double_quoted {
            // No word splitting. The whole expanded string becomes one token.
            result.push(Token {
                value: expanded_string,
                is_single_quoted: false,
                is_double_quoted: token.is_double_quoted,
                is_escaped: token.is_escaped,
            });
        } else {
            // Word splitting based on IFS
            let parts = split_by_ifs(&expanded_string, ifs);
            for part in parts {
                result.push(Token {
                    value: part,
                    is_single_quoted: false,
                    is_double_quoted: false,
                    is_escaped: false,
                });
            }
        }
    }
    result
}

/// Legacy helper for string-based expansion.
fn expand_command_substitution_in_token_legacy(
    token: &str,
    state: &mut ShellState,
    depth: usize,
    rustybox_mode: bool,
    aliases: &mut HashMap<String, String>,
    functions: &mut HashMap<String, String>,
) -> String {
    let mut result = String::with_capacity(token.len());
    let bytes = token.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        // Handle backslash escaping
        if bytes[i] == b'\\' && i + 1 < bytes.len() {
            let next = bytes[i + 1];
            if next == b'$' || next == b'`' || next == b'"' || next == b'\\' {
                result.push(next as char);
                i += 2;
                continue;
            }
        }

        if bytes[i] == b'$' && i + 1 < bytes.len() && bytes[i + 1] == b'(' {
            // Quote-aware parenthesis matching.
            if let Some(end) = find_closing_paren(token, i + 2) {
                let cmd = &token[i + 2..end];
                let output = capture_command_output(
                    cmd,
                    state,
                    depth + 1,
                    rustybox_mode,
                    aliases,
                    functions,
                );
                result.push_str(output.trim_end_matches('\n'));
                i = end + 1; // skip past ')'
                continue;
            }
        } else if bytes[i] == b'`' {
            let mut j = i + 1;
            // Respect backslash escapes inside backticks.
            while j < bytes.len() {
                if bytes[j] == b'\\' && j + 1 < bytes.len() {
                    j += 2;
                    continue;
                }
                if bytes[j] == b'`' {
                    break;
                }
                j += 1;
            }
            if j < bytes.len() {
                let cmd = &token[i + 1..j];
                let output = capture_command_output(
                    cmd,
                    state,
                    depth + 1,
                    rustybox_mode,
                    aliases,
                    functions,
                );
                result.push_str(output.trim_end_matches('\n'));
                i = j + 1;
                continue;
            }
        }
        push_char_at(token, &mut result, &mut i);
    }
    result
}

/// Find the matching closing parenthesis for a command substitution,
/// respecting single quotes, double quotes, and nested parentheses.
/// `start` is the index immediately after the opening '('.
/// Returns the index of the matching ')' or None.
fn find_closing_paren(input: &str, start: usize) -> Option<usize> {
    let bytes = input.as_bytes();
    let mut depth: usize = 1;
    let mut i = start;
    let mut in_single = false;
    let mut in_double = false;
    let mut in_backtick = false;

    while i < bytes.len() {
        let b = bytes[i];
        if in_single {
            if b == b'\'' {
                in_single = false;
            }
        } else if in_double {
            if b == b'"' {
                in_double = false;
            } else if b == b'\\' && i + 1 < bytes.len() {
                i += 1; // skip escaped char
            } else if b == b'`' {
                in_backtick = true;
            }
        } else if in_backtick {
            if b == b'\\' && i + 1 < bytes.len() {
                i += 1;
            } else if b == b'`' {
                in_backtick = false;
            }
        } else {
            match b {
                b'\'' => in_single = true,
                b'"' => in_double = true,
                b'`' => in_backtick = true,
                b'(' => depth += 1,
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(i);
                    }
                }
                _ => {}
            }
        }
        i += 1;
    }
    None
}

/// Capture output of a command substitution with size and timeout limits.
/// If the output exceeds MAX_SUBSTITUTION_BYTES, the child is killed and
/// the function returns what has been read so far (truncated).
fn capture_command_output(
    cmd: &str,
    state: &mut ShellState,
    depth: usize,
    rustybox_mode: bool,
    aliases: &mut HashMap<String, String>,
    functions: &mut HashMap<String, String>,
) -> String {
    // Enforce depth limit to prevent stack overflow from deeply nested
    // command substitutions: $(echo $(echo $(echo ...))).
    if depth > MAX_FUNCTION_DEPTH {
        eprintln!("sh: maximum substitution nesting depth exceeded");
        return String::new();
    }

    let fds = match pipe_cloexec() {
        Ok(fds) => fds,
        Err(_) => return String::new(),
    };

    if !acquire_child_slot() {
        unsafe {
            libc::close(fds.0);
            libc::close(fds.1);
        }
        eprintln!("sh: too many child processes");
        return String::new();
    }

    let pid = unsafe { libc::fork() };
    if pid < 0 {
        unsafe {
            libc::close(fds.0);
            libc::close(fds.1);
        }
        release_child_slot();
        return String::new();
    }

    if pid == 0 {
        unsafe {
            // FIX 1: Create a new process group. This allows the parent to kill
            // the entire process tree (including grandchildren spawned by '&')
            // if the command substitution times out or exceeds size limits.
            libc::setpgid(0, 0);

            libc::close(fds.0);
            libc::dup2(fds.1, libc::STDOUT_FILENO);
            libc::close(fds.1); // Close original write end after dup2

            // FIX 2: Reset signal dispositions to default.
            // Crucially, SIGPIPE must be SIG_DFL to prevent CPU exhaustion
            // in pipelines (e.g., if a grandchild ignores it).
            libc::signal(libc::SIGINT, libc::SIG_DFL);
            libc::signal(libc::SIGQUIT, libc::SIG_DFL);
            libc::signal(libc::SIGPIPE, libc::SIG_DFL);

            // Reset per-process child counter.
            ACTIVE_CHILDREN.store(0, Ordering::SeqCst);

            let status = run_single_command_list(
                cmd,
                state,
                depth,
                rustybox_mode,
                aliases,
                functions,
                false,
            );
            libc::_exit(status as i32);
        }
    }

    // Parent process
    unsafe {
        libc::close(fds.1); // Parent only reads
        // Race-free process group setup (both parent and child call setpgid)
        libc::setpgid(pid, pid);
    }

    // Use poll() for timeout instead of setsockopt(SO_RCVTIMEO) which only works on sockets.
    let mut pfd = libc::pollfd {
        fd: fds.0,
        events: libc::POLLIN,
        revents: 0,
    };
    let timeout_ms = (SUBSTITUTION_READ_TIMEOUT_USEC / 1000) as i32;

    // Read output with a size limit to prevent memory exhaustion.
    let mut output = String::new();
    let mut buf = [0u8; 4096];
    let mut total_read: usize = 0;

    loop {
        // Check for user interrupt to allow breaking out of long reads.
        if SIGINT_RECEIVED.load(Ordering::SeqCst) {
            unsafe {
                // FIX 3: Kill the entire process group, not just the direct child
                libc::kill(-pid, libc::SIGINT);
            }
            break;
        }

        let ret = unsafe { libc::poll(&mut pfd, 1, timeout_ms) };
        if ret < 0 {
            let err = io::Error::last_os_error();
            if err.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            break;
        }
        if ret == 0 {
            // Timeout expired
            eprintln!("sh: command substitution timed out");
            unsafe {
                // FIX 3: Kill the entire process group to ensure grandchildren
                // release their inherited copies of the pipe write end.
                libc::kill(-pid, libc::SIGKILL);
            }
            break;
        }

        let n = unsafe { libc::read(fds.0, buf.as_mut_ptr() as *mut libc::c_void, buf.len()) };
        if n < 0 {
            let err = io::Error::last_os_error();
            if err.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            break;
        }
        if n == 0 {
            break; // EOF reached
        }

        let n = n as usize;
        total_read += n;

        if total_read > MAX_SUBSTITUTION_BYTES {
            eprintln!(
                "sh: command substitution output truncated (limit {} bytes)",
                MAX_SUBSTITUTION_BYTES
            );
            unsafe {
                // FIX 3: Kill the entire process group
                libc::kill(-pid, libc::SIGKILL);
            }

            // Drain any already-pending data to prevent SIGPIPE in the child.
            // Since we killed the process group, the pipe will quickly reach EOF (POLLHUP).
            let drain_timeout = 100; // 100ms
            let mut pfd_drain = libc::pollfd {
                fd: fds.0,
                events: libc::POLLIN,
                revents: 0,
            };
            loop {
                let ret_drain = unsafe { libc::poll(&mut pfd_drain, 1, drain_timeout) };
                if ret_drain <= 0 {
                    break; // no more data, error, or timeout
                }
                // FIX 4: Check for hangup to exit drain loop immediately
                if (pfd_drain.revents & libc::POLLHUP) != 0 {
                    break;
                }
                let d =
                    unsafe { libc::read(fds.0, buf.as_mut_ptr() as *mut libc::c_void, buf.len()) };
                if d <= 0 {
                    break;
                }
            }
            break;
        }
        output.push_str(&String::from_utf8_lossy(&buf[..n]));
    }

    unsafe {
        libc::close(fds.0);
    }

    let mut status: i32 = 0;
    unsafe {
        // Wait for the direct child to prevent zombies.
        // Grandchildren will be orphaned and adopted by init, which is fine
        // since they have been killed and no longer hold the pipe open.
        libc::waitpid(pid, &mut status, 0);
    }
    release_child_slot();

    output
}
// -----------------------------------------------------------------------------
// Redirection parsing.
// -----------------------------------------------------------------------------

/// Parse redirections from Token stream. Done BEFORE globbing to detect
/// ambiguous redirects properly.
fn parse_redirections_tokens(
    tokens: Vec<Token>,
) -> Result<
    (
        Vec<Token>,
        Option<Token>,
        Option<Token>,
        bool,
        Option<Token>,
    ),
    String,
> {
    let mut stdin_file = None;
    let mut stdout_file = None;
    let mut append = false;
    let here_doc = None;
    let mut result = Vec::with_capacity(tokens.len());
    let mut i = 0;
    while i < tokens.len() {
        if tokens[i].value == "<" {
            if i + 1 >= tokens.len() {
                return Err("missing filename for '<'".into());
            }
            stdin_file = Some(tokens[i + 1].clone());
            i += 2;
        } else if tokens[i].value == ">" {
            if i + 1 >= tokens.len() {
                return Err("missing filename for '>'".into());
            }
            stdout_file = Some(tokens[i + 1].clone());
            append = false;
            i += 2;
        } else if tokens[i].value == ">>" {
            if i + 1 >= tokens.len() {
                return Err("missing filename for '>>'".into());
            }
            stdout_file = Some(tokens[i + 1].clone());
            append = true;
            i += 2;
        } else if tokens[i].value == "<<" {
            if i + 1 >= tokens.len() {
                return Err("missing here-doc delimiter".into());
            }
            return Err("here-documents are not yet implemented".into());
        } else {
            result.push(tokens[i].clone());
            i += 1;
        }
    }
    Ok((result, stdin_file, stdout_file, append, here_doc))
}

// -----------------------------------------------------------------------------
// Process substitution: <(...) and >(...).
// Returns the modified input string and a list of file descriptors that
// must be closed after the command finishes.
// FIXED: Now respects quoting contexts and clears O_CLOEXEC so child inherits it.
// -----------------------------------------------------------------------------
fn replace_process_substitution(
    input: &str,
    state: &mut ShellState,
    depth: usize,
    rustybox_mode: bool,
    aliases: &mut HashMap<String, String>,
    functions: &mut HashMap<String, String>,
) -> (String, Vec<RawFd>) {
    let mut result = String::with_capacity(input.len());
    let mut fds = Vec::new();
    let bytes = input.as_bytes();
    let mut i = 0;

    let mut in_single = false;
    let mut in_double = false;
    let mut in_backtick = false;

    while i < bytes.len() {
        let b = bytes[i];

        // Handle quoting state transitions
        if in_single {
            push_char_at(input, &mut result, &mut i);
            if b == b'\'' {
                in_single = false;
            }
            continue;
        }

        if in_double {
            if b == b'"' {
                result.push('"');
                in_double = false;
                i += 1;
            } else if b == b'\\' && i + 1 < bytes.len() {
                i += 1;
                push_char_at(input, &mut result, &mut i);
            } else {
                push_char_at(input, &mut result, &mut i);
            }
            continue;
        }

        if in_backtick {
            if b == b'\\' && i + 1 < bytes.len() {
                result.push('\\');
                i += 1;
                push_char_at(input, &mut result, &mut i);
            } else if b == b'`' {
                result.push('`');
                in_backtick = false;
                i += 1;
            } else {
                push_char_at(input, &mut result, &mut i);
            }
            continue;
        }

        // Outside quotes
        if b == b'\'' {
            in_single = true;
            result.push('\'');
            i += 1;
        } else if b == b'"' {
            in_double = true;
            result.push('"');
            i += 1;
        } else if b == b'`' {
            in_backtick = true;
            result.push('`');
            i += 1;
        } else if i + 1 < bytes.len() && (b == b'<' || b == b'>') && bytes[i + 1] == b'(' {
            let direction = b;
            // Use quote-aware matching for the closing paren.
            if let Some(end) = find_closing_paren(input, i + 2) {
                let cmd_str = &input[i + 2..end];

                let pipe_fds = match pipe_cloexec() {
                    Ok(fds) => fds,
                    Err(_) => {
                        // Cannot create pipe; emit the literal text.
                        push_char_at(input, &mut result, &mut i);
                        continue;
                    }
                };

                if !acquire_child_slot() {
                    unsafe {
                        libc::close(pipe_fds.0);
                        libc::close(pipe_fds.1);
                    }
                    eprintln!("sh: too many child processes");
                    push_char_at(input, &mut result, &mut i);
                    continue;
                }

                let pid = unsafe { libc::fork() };
                if pid < 0 {
                    unsafe {
                        libc::close(pipe_fds.0);
                        libc::close(pipe_fds.1);
                    }
                    release_child_slot();
                    eprintln!("sh: fork failed");
                    push_char_at(input, &mut result, &mut i);
                    continue;
                }

                if pid == 0 {
                    unsafe {
                        // Child: create a new process group.
                        libc::setpgid(0, 0);
                        // Reset per-process child counter.
                        ACTIVE_CHILDREN.store(0, Ordering::SeqCst);
                        if direction == b'<' {
                            libc::close(pipe_fds.0);
                            libc::dup2(pipe_fds.1, libc::STDOUT_FILENO);
                            libc::close(pipe_fds.1);
                        } else {
                            libc::close(pipe_fds.1);
                            libc::dup2(pipe_fds.0, libc::STDIN_FILENO);
                            libc::close(pipe_fds.0);
                        }
                        // Inherit the parent shell's aliases, functions, and
                        // state so that process substitutions behave like any
                        // other command in the current shell environment.
                        let status = run_command_list(
                            cmd_str,
                            state,
                            depth + 1,
                            rustybox_mode,
                            aliases,
                            functions,
                        );
                        libc::_exit(status as i32);
                    }
                }

                // Parent: set child's process group.
                unsafe {
                    libc::setpgid(pid, pid);
                }

                // Parent: keep the fd we need, close the other.
                let fd_to_keep = if direction == b'<' {
                    unsafe {
                        libc::close(pipe_fds.1);
                    }
                    pipe_fds.0
                } else {
                    unsafe {
                        libc::close(pipe_fds.0);
                    }
                    pipe_fds.1
                };

                // Clear FD_CLOEXEC so the child process inherits the fd when execvp is called.
                unsafe {
                    libc::fcntl(fd_to_keep, libc::F_SETFD, 0);
                }

                result.push_str(&format!("/dev/fd/{}", fd_to_keep));
                fds.push(fd_to_keep);

                // Track PID separately for process substitution to avoid
                // polluting the job table and prevent zombie accumulation.
                PROC_SUB_PIDS.lock().unwrap().push(pid);

                i = end + 1;
                continue;
            }
        } else if b == b'\\' && i + 1 < bytes.len() {
            result.push('\\');
            i += 1;
            push_char_at(input, &mut result, &mut i);
        } else {
            push_char_at(input, &mut result, &mut i);
        }
    }
    (result, fds)
}

// -----------------------------------------------------------------------------
// Globbing.
// -----------------------------------------------------------------------------
fn expand_globs_struct(tokens: Vec<Token>) -> Vec<Token> {
    let mut result = Vec::new();
    for token in tokens {
        // Do not expand globs if the token was single-quoted, double-quoted, or escaped
        if token.is_single_quoted || token.is_escaped || token.is_double_quoted {
            result.push(token);
            continue;
        }

        if token.value.contains('*') || token.value.contains('?') || token.value.contains('[') {
            match glob(&token.value) {
                Ok(matches) if !matches.is_empty() => {
                    for m in matches {
                        result.push(Token {
                            value: m,
                            is_single_quoted: false,
                            is_double_quoted: false,
                            is_escaped: false,
                        });
                    }
                }
                _ => result.push(token),
            }
        } else {
            result.push(token);
        }
    }
    result
}

fn expand_globs(tokens: Vec<String>) -> Vec<String> {
    tokens_to_strings(expand_globs_struct(strings_to_tokens(tokens)))
}

fn glob(pattern: &str) -> io::Result<Vec<String>> {
    let path = Path::new(pattern);
    let (dir, file_pattern) = if pattern.contains('/') {
        (
            path.parent().unwrap_or(Path::new(".")).to_path_buf(),
            path.file_name()
                .unwrap_or_default()
                .to_str()
                .unwrap_or("")
                .to_string(),
        )
    } else {
        (Path::new(".").to_path_buf(), pattern.to_string())
    };

    let mut matches = Vec::new();
    if dir.is_dir() {
        let mut dir_iterations = 0;
        for entry in fs::read_dir(&dir)? {
            dir_iterations += 1;
            if dir_iterations > 100_000 {
                // Strict scanning limit
                eprintln!("sh: glob: directory too large, truncating results");
                break;
            }
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if matches_pattern(&name, &file_pattern) {
                matches.push(dir.join(&name).to_string_lossy().into_owned());
                if matches.len() >= MAX_GLOB_RESULTS {
                    break;
                }
            }
        }
    }
    matches.sort();
    Ok(matches)
}

fn matches_pattern(name: &str, pattern: &str) -> bool {
    let name_bytes = name.as_bytes();
    let pat_bytes = pattern.as_bytes();
    let mut ni = 0;
    let mut pi = 0;
    let mut star = None;
    let mut match_start = 0;
    let mut iterations: usize = 0;

    while ni < name_bytes.len() {
        // ReDoS mitigation: abort on pathological patterns.
        iterations += 1;
        if iterations > MAX_GLOB_ITERATIONS {
            return false;
        }

        if pi < pat_bytes.len() && pat_bytes[pi] == b'*' {
            star = Some(pi);
            match_start = ni;
            pi += 1;
        } else if pi < pat_bytes.len() && (pat_bytes[pi] == b'?' || pat_bytes[pi] == name_bytes[ni])
        {
            ni += 1;
            pi += 1;
        } else if pi < pat_bytes.len() && pat_bytes[pi] == b'[' {
            pi += 1;
            let mut matched = false;
            let mut negate = false;
            if pi < pat_bytes.len() && pat_bytes[pi] == b'!' {
                negate = true;
                pi += 1;
            }
            // Handle ']' as the first character in the bracket (literal).
            if pi < pat_bytes.len() && pat_bytes[pi] == b']' {
                if ni < name_bytes.len() && name_bytes[ni] == b']' {
                    matched = true;
                }
                pi += 1;
            }
            while pi < pat_bytes.len() && pat_bytes[pi] != b']' {
                if pi + 2 < pat_bytes.len() && pat_bytes[pi + 1] == b'-' {
                    let start = pat_bytes[pi];
                    let end = pat_bytes[pi + 2];
                    if ni < name_bytes.len() && name_bytes[ni] >= start && name_bytes[ni] <= end {
                        matched = true;
                    }
                    pi += 3;
                } else {
                    if ni < name_bytes.len() && name_bytes[ni] == pat_bytes[pi] {
                        matched = true;
                    }
                    pi += 1;
                }
            }
            // If we reached end of pattern without finding ']', treat the
            // bracket expression as a literal non-match (fail closed).
            if pi >= pat_bytes.len() {
                return false;
            }
            // Skip closing ']'.
            pi += 1;
            if matched == negate {
                if let Some(s) = star {
                    pi = s + 1;
                    match_start += 1;
                    ni = match_start;
                } else {
                    return false;
                }
            } else {
                ni += 1;
            }
        } else if let Some(s) = star {
            pi = s + 1;
            match_start += 1;
            ni = match_start;
        } else {
            return false;
        }
    }
    while pi < pat_bytes.len() && pat_bytes[pi] == b'*' {
        pi += 1;
    }
    pi == pat_bytes.len()
}

// -----------------------------------------------------------------------------
// File descriptor utilities.
// -----------------------------------------------------------------------------

/// Create a pipe with O_CLOEXEC set on both ends to prevent file descriptor
/// leaks into child processes spawned by nested substitutions or exec.
fn pipe_cloexec() -> io::Result<(RawFd, RawFd)> {
    let mut fds = [-1i32; 2];
    let rc = unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) };
    if rc < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok((fds[0], fds[1]))
    }
}

fn open_file_for_read(path: &str) -> io::Result<RawFd> {
    // Use O_NOFOLLOW to prevent symlink-following attacks on redirection
    // targets.  If the path is a symlink the open will fail with ELOOP.
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?;
    Ok(file.into_raw_fd())
}

fn open_file_for_write(path: &str, append: bool) -> io::Result<RawFd> {
    // Use O_NOFOLLOW to prevent symlink-following attacks.  An attacker
    // cannot redirect output to an arbitrary file by placing a symlink at
    // the target path.
    let file = OpenOptions::new()
        .write(true)
        .create(true)
        .append(append)
        .truncate(!append)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?;
    Ok(file.into_raw_fd())
}

// -----------------------------------------------------------------------------
// Registration macro.
// -----------------------------------------------------------------------------
register_command!(
    SH_CMD,
    "sh",
    "",
    CommandFlags::BIN.bits() | CommandFlags::NOFORK.bits(),
    sh_main
);
