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
//     SIGQUIT; SIGTSTP stops child and enables job control.
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

use std::collections::HashMap;
use std::env;
use std::ffi::{CStr, CString};
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::os::unix::io::{FromRawFd, IntoRawFd, RawFd};
use std::path::{Path, PathBuf};
use std::process::exit;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{LazyLock, Mutex};

// -----------------------------------------------------------------------------
// ANSI escape sequences for prompt coloring.
// -----------------------------------------------------------------------------
const ANSI_CYAN: &str = "\x1b[36m";
const ANSI_GREEN: &str = "\x1b[32m";
const ANSI_RED: &str = "\x1b[31m";
const ANSI_RESET: &str = "\x1b[0m";

// -----------------------------------------------------------------------------
// Default history size limit.
// -----------------------------------------------------------------------------
const DEFAULT_HIST_LIMIT: usize = 500;

// -----------------------------------------------------------------------------
// Global flags set by signal handlers.
// -----------------------------------------------------------------------------
static SIGINT_RECEIVED: AtomicBool = AtomicBool::new(false);
static SIGTSTP_RECEIVED: AtomicBool = AtomicBool::new(false);

// -----------------------------------------------------------------------------
// Job control structures and global job table.
// -----------------------------------------------------------------------------
struct Job {
    pid: i32,
    command: String,
    state: JobState,
}

#[derive(Clone, PartialEq)]
enum JobState {
    Running,
    Stopped,
}

static JOBS: LazyLock<Mutex<HashMap<i32, Job>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

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
        // SIGINT – forward to foreground child
        let mut sa: libc::sigaction = std::mem::zeroed();
        sa.sa_sigaction = sigint_handler as libc::sighandler_t;
        sa.sa_flags = 0; // no SA_RESTART
        libc::sigemptyset(&mut sa.sa_mask);
        libc::sigaction(libc::SIGINT, &sa, std::ptr::null_mut());

        // SIGQUIT – ignore
        let mut sa_ignore: libc::sigaction = std::mem::zeroed();
        sa_ignore.sa_sigaction = libc::SIG_IGN;
        sa_ignore.sa_flags = 0;
        libc::sigemptyset(&mut sa_ignore.sa_mask);
        libc::sigaction(libc::SIGQUIT, &sa_ignore, std::ptr::null_mut());

        // SIGTSTP – stop foreground child
        let mut sa_tstp: libc::sigaction = std::mem::zeroed();
        sa_tstp.sa_sigaction = sigtstp_handler as *const () as libc::sighandler_t;
        sa_tstp.sa_flags = 0;
        libc::sigemptyset(&mut sa_tstp.sa_mask);
        libc::sigaction(libc::SIGTSTP, &sa_tstp, std::ptr::null_mut());

        // SIGCHLD – default
        libc::signal(libc::SIGCHLD, libc::SIG_DFL);
    }
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

        // Variable name completion after '$'
        if word.starts_with('$') {
            let varname = &word[1..];
            let mut matches: Vec<Pair> = env::vars()
                .filter(|(k, _)| k.starts_with(varname))
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
                    // If the user typed a tilde, replace the expanded prefix with '~'
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
        // Highlight unmatched quotes in red.
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
    let tokens = tokenize(line);
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

    // Load existing history from file, if any.
    // If the file doesn't exist, we start fresh.
    let _ = rl.load_history(&hist_file);

    // Limit in-memory history by reading entries, truncating, and clearing/re-adding.
    // FileHistory only provides `iter()`, `len()`, `add()`, `save()`, `load()`.
    // We rebuild the history with the last DEFAULT_HIST_LIMIT entries.
    {
        let hist = rl.history_mut();
        let entries: Vec<String> = hist.iter().map(|s| s.to_string()).collect();
        let total = entries.len();
        if total > DEFAULT_HIST_LIMIT {
            let skip = total - DEFAULT_HIST_LIMIT;
            let kept = &entries[skip..];
            // Clear in-memory history
            // There's no clear() method; we must iterate and remove from the end.
            // Instead we'll just iterate backwards and hope... Actually, FileHistory
            // doesn't support remove. We'll save trimmed to disk later.
            // For now, just track that we need trimming.
        }
    }

    let mut last_status: u8 = 0;
    let mut aliases: HashMap<String, String> = HashMap::new();
    let mut functions: HashMap<String, String> = HashMap::new();

    // Source ~/.shrc on startup if it exists.
    if let Ok(home) = env::var("HOME") {
        let rc_file = format!("{}/.shrc", home);
        if Path::new(&rc_file).exists() {
            if let Ok(content) = fs::read_to_string(&rc_file) {
                let _ = run_command_list(
                    &content,
                    last_status,
                    rustybox_mode,
                    &mut aliases,
                    &mut functions,
                );
            }
        }
    }

    loop {
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
                    // HISTCONTROL=ignoredups prevents consecutive duplicates.
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
                last_status = run_command_list(
                    &input,
                    last_status,
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

    // Before saving, trim the history to DEFAULT_HIST_LIMIT by saving only the
    // last DEFAULT_HIST_LIMIT entries to the file manually.
    {
        let hist = rl.history();
        let entries: Vec<String> = hist.iter().map(|s| s.to_string()).collect();
        let total = entries.len();
        let skip = if total > DEFAULT_HIST_LIMIT {
            total - DEFAULT_HIST_LIMIT
        } else {
            0
        };

        // Write the trimmed history to the file.
        if let Ok(mut file) = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&hist_file)
        {
            for entry in &entries[skip..] {
                let _ = writeln!(file, "{}", entry);
            }
        }
    }

    last_status
}

// -----------------------------------------------------------------------------
// Prompt generation (hostname obtained via gethostname).
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
        let mut buf = [0u8; 256];
        unsafe {
            if libc::gethostname(buf.as_mut_ptr() as *mut libc::c_char, buf.len()) == 0 {
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
// Command execution.
// -----------------------------------------------------------------------------
fn run_command_list(
    input: &str,
    last_status: u8,
    rustybox_mode: bool,
    aliases: &mut HashMap<String, String>,
    functions: &mut HashMap<String, String>,
) -> u8 {
    // Check for function definition: name() { body; }
    if let Some(captures) = function_definition_regex(input) {
        functions.insert(captures.0.to_string(), captures.1.to_string());
        return last_status;
    }

    let mut status = last_status;
    let bytes = input.as_bytes();
    let mut commands: Vec<(String, Option<char>)> = Vec::new();

    let mut current = String::with_capacity(64);
    let mut in_single = false;
    let mut in_double = false;
    let mut i = 0;

    while i < bytes.len() {
        let b = bytes[i];
        if in_single {
            current.push(b as char);
            if b == b'\'' {
                in_single = false;
            }
        } else if in_double {
            current.push(b as char);
            if b == b'"' {
                in_double = false;
            } else if b == b'\\' && i + 1 < bytes.len() {
                i += 1;
                current.push(bytes[i] as char);
            }
        } else if b == b'\'' {
            in_single = true;
            current.push(b as char);
        } else if b == b'"' {
            in_double = true;
            current.push(b as char);
        } else if b == b'&' && i + 1 < bytes.len() && bytes[i + 1] == b'&' {
            commands.push((current.trim().to_string(), Some('&')));
            current.clear();
            i += 2;
            continue;
        } else if b == b'|' && i + 1 < bytes.len() && bytes[i + 1] == b'|' {
            commands.push((current.trim().to_string(), Some('|')));
            current.clear();
            i += 2;
            continue;
        } else if b == b';' {
            commands.push((current.trim().to_string(), Some(';')));
            current.clear();
            i += 1;
            continue;
        } else if b == b'&' {
            commands.push((current.trim().to_string(), Some('b')));
            current.clear();
            i += 1;
            continue;
        } else {
            current.push(b as char);
        }
        i += 1;
    }
    if !current.trim().is_empty() {
        commands.push((current.trim().to_string(), None));
    }

    for (pos, (cmd, _sep)) in commands.iter().enumerate() {
        if cmd.is_empty() {
            continue;
        }

        if pos > 0 {
            let prev_sep = commands[pos - 1].1;
            match prev_sep {
                Some('&') if status != 0 => continue,
                Some('|') if status == 0 => continue,
                _ => {}
            }
        }

        let is_background = commands[pos].1 == Some('b');
        let expanded = expand_command_substitution(cmd, status, rustybox_mode, aliases, functions);
        status = run_single_command_list(
            &expanded,
            status,
            rustybox_mode,
            aliases,
            functions,
            is_background,
        );
    }

    reap_background();
    status
}

fn function_definition_regex(input: &str) -> Option<(&str, &str)> {
    let trimmed = input.trim();
    if let Some(rest) = trimmed.strip_suffix("}") {
        if let Some(open) = rest.find('{') {
            let header = &rest[..open].trim();
            if let Some(name) = header.strip_suffix("()") {
                let body = &rest[open + 1..].trim();
                return Some((name.trim(), body));
            }
        }
    }
    None
}

fn run_single_command_list(
    input: &str,
    last_status: u8,
    rustybox_mode: bool,
    aliases: &mut HashMap<String, String>,
    functions: &mut HashMap<String, String>,
    background: bool,
) -> u8 {
    // Replace <(...) and >(...) with /dev/fd/N.
    let input = replace_process_substitution(input);

    let segments: Vec<&str> = input.split('|').filter(|s| !s.trim().is_empty()).collect();
    if segments.is_empty() {
        return last_status;
    }

    if background {
        let pid = unsafe { libc::fork() };
        if pid < 0 {
            eprintln!("sh: fork failed");
            return 1;
        }
        if pid == 0 {
            let status = if segments.len() == 1 {
                run_simple_command(segments[0], last_status, rustybox_mode, aliases, functions)
            } else {
                run_pipeline(&segments, last_status, rustybox_mode, aliases, functions)
            };
            unsafe { libc::_exit(status as i32) };
        }
        eprintln!("[{}] {}", pid, input);
        add_job(pid, input.to_string(), JobState::Running);
        return last_status;
    }

    if segments.len() == 1 {
        run_simple_command(segments[0], last_status, rustybox_mode, aliases, functions)
    } else {
        run_pipeline(&segments, last_status, rustybox_mode, aliases, functions)
    }
}

fn run_simple_command(
    input: &str,
    last_status: u8,
    rustybox_mode: bool,
    aliases: &mut HashMap<String, String>,
    functions: &mut HashMap<String, String>,
) -> u8 {
    let mut tokens = tokenize(input);
    if tokens.is_empty() {
        return 0;
    }

    tokens = expand_aliases(tokens, aliases);
    tokens = expand_tilde(tokens);
    tokens = expand_variables(&tokens, last_status);
    tokens = expand_globs(tokens);
    if tokens.is_empty() {
        return 0;
    }

    let (tokens, stdin_file, stdout_file, append, _here_doc) = match parse_redirections(tokens) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("sh: {e}");
            return 1;
        }
    };

    if let Some(result) = handle_builtin(&tokens, aliases, functions) {
        return result;
    }

    if tokens.len() >= 1 {
        if let Some(body) = functions.get(&tokens[0]).cloned() {
            let status = run_command_list(&body, last_status, rustybox_mode, aliases, functions);
            return status;
        }
    }

    if rustybox_mode {
        if let Some(result) = handle_rustybox_builtin(&tokens) {
            return result;
        }
    }

    run_external(&tokens, stdin_file, stdout_file, append, rustybox_mode)
}

fn run_pipeline(
    segments: &[&str],
    last_status: u8,
    rustybox_mode: bool,
    aliases: &mut HashMap<String, String>,
    functions: &mut HashMap<String, String>,
) -> u8 {
    let mut prev_read_fd: Option<RawFd> = None;
    let mut pids: Vec<i32> = Vec::new();
    let mut statuses: Vec<u8> = Vec::new();
    let pipefail = env::var("SHELLOPTS")
        .unwrap_or_default()
        .contains("pipefail");

    for (i, segment) in segments.iter().enumerate() {
        let mut tokens = tokenize(segment);
        if tokens.is_empty() {
            continue;
        }
        tokens = expand_aliases(tokens, aliases);
        tokens = expand_tilde(tokens);
        tokens = expand_variables(&tokens, last_status);
        tokens = expand_globs(tokens);
        if tokens.is_empty() {
            continue;
        }

        let (tokens, stdin_file, stdout_file, append, _) = match parse_redirections(tokens) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("sh: {e}");
                return 1;
            }
        };

        if matches!(tokens[0].as_str(), "cd" | "exit" | "exec" | "alias") {
            eprintln!("sh: '{}' cannot be used in pipelines", tokens[0]);
            return 1;
        }

        let (pipe_read, pipe_write) = if i != segments.len() - 1 {
            let fds = pipe().unwrap();
            (Some(fds.0), Some(fds.1))
        } else {
            (None, None)
        };

        let pid = unsafe { libc::fork() };
        if pid < 0 {
            eprintln!("sh: fork failed");
            // Close any open fds to avoid leaks.
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
            return 1;
        }

        if pid == 0 {
            unsafe {
                libc::signal(libc::SIGPIPE, libc::SIG_IGN);

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

                if let Some(f) = &stdin_file {
                    if let Ok(fd) = open_file(f, false) {
                        libc::dup2(fd, libc::STDIN_FILENO);
                        libc::close(fd);
                    } else {
                        libc::_exit(1);
                    }
                }
                if let Some(f) = &stdout_file {
                    if let Ok(fd) = open_file(f, append) {
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

                let args: Vec<CString> = tokens
                    .iter()
                    .map(|s| CString::new(s.as_bytes()).unwrap())
                    .collect();
                let argv: Vec<*const libc::c_char> = args
                    .iter()
                    .map(|s| s.as_ptr())
                    .chain(std::iter::once(std::ptr::null()))
                    .collect();
                libc::execvp(argv[0], argv.as_ptr());
                eprintln!("sh: {}: command not found", tokens[0]);
                libc::_exit(127);
            }
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
    let pid = unsafe { libc::fork() };
    if pid < 0 {
        eprintln!("sh: fork failed");
        return 1;
    }

    if pid == 0 {
        unsafe {
            if let Some(f) = &stdin_file {
                if let Ok(fd) = open_file(f, false) {
                    libc::dup2(fd, libc::STDIN_FILENO);
                    libc::close(fd);
                } else {
                    libc::_exit(1);
                }
            }
            if let Some(f) = &stdout_file {
                if let Ok(fd) = open_file(f, append) {
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

            let args: Vec<CString> = tokens
                .iter()
                .map(|s| CString::new(s.as_bytes()).unwrap())
                .collect();
            let argv: Vec<*const libc::c_char> = args
                .iter()
                .map(|s| s.as_ptr())
                .chain(std::iter::once(std::ptr::null()))
                .collect();
            libc::execvp(argv[0], argv.as_ptr());
            eprintln!("sh: {}: command not found", tokens[0]);
            libc::_exit(127);
        }
    }

    wait_for_child(pid)
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
                    unsafe {
                        libc::kill(pid, libc::SIGINT);
                    }
                }
                if SIGTSTP_RECEIVED.swap(false, Ordering::SeqCst) {
                    unsafe {
                        libc::kill(pid, libc::SIGTSTP);
                    }
                    update_job_state(pid, JobState::Stopped);
                    eprintln!("\n[{}] + stopped", pid);
                    return 146; // signal + 128
                }
                continue;
            }
            return 1;
        }
        if libc::WIFEXITED(status) {
            let code = libc::WEXITSTATUS(status) as u8;
            if code == 0 {
                remove_job(pid);
            }
            return code;
        }
        if libc::WIFSIGNALED(status) {
            let sig = libc::WTERMSIG(status);
            if sig == libc::SIGTSTP {
                update_job_state(pid, JobState::Stopped);
            }
            remove_job(pid);
            return 128 + sig as u8;
        }
        if libc::WIFSTOPPED(status) {
            update_job_state(pid, JobState::Stopped);
            return 146;
        }
        return 1;
    }
}

fn reap_background() {
    loop {
        let mut status: i32 = 0;
        let pid = unsafe { libc::waitpid(-1, &mut status, libc::WNOHANG | libc::WUNTRACED) };
        if pid <= 0 {
            break;
        }
        if libc::WIFEXITED(status) || libc::WIFSIGNALED(status) {
            eprintln!("[{}] Done", pid);
            remove_job(pid);
        } else if libc::WIFSTOPPED(status) {
            update_job_state(pid, JobState::Stopped);
        }
    }
}

// -----------------------------------------------------------------------------
// Job control builtins helpers.
// -----------------------------------------------------------------------------
fn add_job(pid: i32, cmd: String, state: JobState) {
    let mut jobs = JOBS.lock().unwrap();
    jobs.insert(
        pid,
        Job {
            pid,
            command: cmd,
            state,
        },
    );
}

fn remove_job(pid: i32) {
    let mut jobs = JOBS.lock().unwrap();
    jobs.remove(&pid);
}

fn update_job_state(pid: i32, state: JobState) {
    if let Some(job) = JOBS.lock().unwrap().get_mut(&pid) {
        job.state = state;
    }
}

fn jobs_builtin() -> u8 {
    let jobs = JOBS.lock().unwrap();
    for (_, job) in jobs.iter() {
        let status = match job.state {
            JobState::Running => "Running",
            JobState::Stopped => "Stopped",
        };
        println!("[{}] {} - {}", job.pid, status, job.command);
    }
    0
}

fn fg_builtin(pid: Option<i32>) -> u8 {
    let target_pid = match pid {
        Some(p) => p,
        None => {
            let jobs = JOBS.lock().unwrap();
            if let Some((&p, _)) = jobs.iter().next() {
                p
            } else {
                eprintln!("fg: no current job");
                return 1;
            }
        }
    };
    unsafe {
        libc::signal(libc::SIGTTOU, libc::SIG_DFL);
        libc::tcsetpgrp(libc::STDIN_FILENO, target_pid);
    }
    update_job_state(target_pid, JobState::Running);
    unsafe {
        libc::kill(target_pid, libc::SIGCONT);
    }
    let status = wait_for_child(target_pid);
    unsafe {
        libc::tcsetpgrp(libc::STDIN_FILENO, libc::getpid());
        libc::signal(libc::SIGTTOU, libc::SIG_DFL);
    }
    status
}

fn bg_builtin(pid: Option<i32>) -> u8 {
    let target_pid = match pid {
        Some(p) => p,
        None => {
            let jobs = JOBS.lock().unwrap();
            if let Some((&p, _)) = jobs.iter().next() {
                p
            } else {
                eprintln!("bg: no current job");
                return 1;
            }
        }
    };
    update_job_state(target_pid, JobState::Running);
    unsafe {
        libc::kill(target_pid, libc::SIGCONT);
    }
    eprintln!("[{}] continued in background", target_pid);
    0
}

// -----------------------------------------------------------------------------
// Builtin commands.
// -----------------------------------------------------------------------------
fn handle_builtin(
    tokens: &[String],
    aliases: &mut HashMap<String, String>,
    functions: &HashMap<String, String>,
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
                0
            };
            exit(code as i32);
        }
        "exec" => {
            if tokens.len() < 2 {
                eprintln!("exec: missing command");
                return Some(1);
            }
            let args: Vec<CString> = tokens[1..]
                .iter()
                .map(|s| CString::new(s.as_bytes()).unwrap())
                .collect();
            let argv: Vec<*const libc::c_char> = args
                .iter()
                .map(|s| s.as_ptr())
                .chain(std::iter::once(std::ptr::null()))
                .collect();
            unsafe {
                libc::execvp(argv[0], argv.as_ptr());
            }
            eprintln!("exec: command not found");
            exit(127);
        }
        "export" => {
            for arg in &tokens[1..] {
                if let Some((key, value)) = arg.split_once('=') {
                    unsafe {
                        env::set_var(key, value);
                    }
                } else if env::var(arg).is_err() {
                    eprintln!("export: {}: not a valid identifier", arg);
                    return Some(1);
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
            match fs::read_to_string(&filename) {
                Ok(content) => {
                    let status =
                        run_command_list(&content, 0, false, aliases, &mut functions.clone());
                    Some(status)
                }
                Err(e) => {
                    eprintln!("{}: {}", tokens[0], e);
                    Some(1)
                }
            }
        }
        "eval" => {
            let args = tokens[1..].join(" ");
            let status = run_command_list(&args, 0, false, aliases, &mut functions.clone());
            Some(status)
        }
        "set" => {
            if tokens.len() > 1 {
                let mut idx = 1;
                while idx < tokens.len() {
                    let arg = &tokens[idx];
                    if arg == "-o" || arg == "+o" {
                        let val = if arg.starts_with('-') { "on" } else { "off" };
                        if let Some(opt) = tokens.get(idx + 1) {
                            match opt.as_str() {
                                "pipefail" => {
                                    let shellopts = env::var("SHELLOPTS").unwrap_or_default();
                                    if val == "on" {
                                        unsafe {
                                            env::set_var(
                                                "SHELLOPTS",
                                                format!("{} pipefail", shellopts).trim(),
                                            );
                                        }
                                    } else {
                                        unsafe {
                                            env::set_var(
                                                "SHELLOPTS",
                                                shellopts.replace("pipefail", "").trim(),
                                            );
                                        }
                                    }
                                }
                                _ => eprintln!("set: unknown option {}", opt),
                            }
                            idx += 1; // skip the option name
                        }
                    }
                    idx += 1;
                }
            } else {
                for (k, v) in env::vars() {
                    println!("{}={}", k, v);
                }
            }
            Some(0)
        }
        "unset" => {
            for arg in &tokens[1..] {
                unsafe {
                    env::remove_var(arg);
                }
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
    unsafe {
        env::set_var("OLDPWD", old_pwd);
        env::set_var("PWD", &new_pwd);
    }
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
fn tokenize(input: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let bytes = input.as_bytes();
    let mut i = 0;
    let mut current = String::with_capacity(64);

    while i < bytes.len() {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        current.clear();

        if bytes[i] == b'\'' {
            i += 1;
            while i < bytes.len() && bytes[i] != b'\'' {
                current.push(bytes[i] as char);
                i += 1;
            }
            if i < bytes.len() {
                i += 1;
            }
        } else if bytes[i] == b'"' {
            i += 1;
            while i < bytes.len() && bytes[i] != b'"' {
                if bytes[i] == b'\\' && i + 1 < bytes.len() {
                    i += 1;
                    match bytes[i] {
                        b'$' | b'`' | b'"' | b'\\' | b'\n' => current.push(bytes[i] as char),
                        _ => {
                            current.push('\\');
                            current.push(bytes[i] as char);
                        }
                    }
                } else {
                    current.push(bytes[i] as char);
                }
                i += 1;
            }
            if i < bytes.len() {
                i += 1;
            }
        } else {
            while i < bytes.len() && !bytes[i].is_ascii_whitespace() {
                if bytes[i] == b'\\' && i + 1 < bytes.len() {
                    i += 1;
                    current.push(bytes[i] as char);
                } else if bytes[i] == b'#' {
                    if !current.is_empty() {
                        tokens.push(current.clone());
                    }
                    return tokens;
                } else {
                    current.push(bytes[i] as char);
                }
                i += 1;
            }
        }
        if !current.is_empty() {
            tokens.push(current.clone());
        }
    }
    tokens
}

fn expand_aliases(tokens: Vec<String>, aliases: &HashMap<String, String>) -> Vec<String> {
    if tokens.is_empty() {
        return tokens;
    }
    if let Some(expansion) = aliases.get(&tokens[0]) {
        let mut new_tokens = tokenize(expansion);
        new_tokens.extend_from_slice(&tokens[1..]);
        return new_tokens;
    }
    tokens
}

fn expand_tilde(tokens: Vec<String>) -> Vec<String> {
    tokens.into_iter().map(|t| expand_tilde_one(&t)).collect()
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
            let guess = format!("/home/{}", user);
            if Path::new(&guess).is_dir() {
                return guess + "/" + rest;
            }
        }
    }
    s.to_string()
}

fn expand_variables(tokens: &[String], last_status: u8) -> Vec<String> {
    tokens
        .iter()
        .map(|token| {
            let mut result = String::with_capacity(token.len());
            let bytes = token.as_bytes();
            let mut i = 0;
            while i < bytes.len() {
                if bytes[i] == b'$' && i + 1 < bytes.len() {
                    match bytes[i + 1] {
                        b'?' => {
                            use std::fmt::Write;
                            write!(result, "{}", last_status).unwrap();
                            i += 2;
                            continue;
                        }
                        b'{' => {
                            if let Some(end) = token[i + 2..].find('}') {
                                let var_name = &token[i + 2..i + 2 + end];
                                result.push_str(&env::var(var_name).unwrap_or_default());
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
                            result.push_str(&env::var(var_name).unwrap_or_default());
                            i = end;
                            continue;
                        }
                        _ => {}
                    }
                }
                result.push(bytes[i] as char);
                i += 1;
            }
            result
        })
        .collect()
}

fn expand_globs(tokens: Vec<String>) -> Vec<String> {
    let mut result = Vec::new();
    for token in tokens {
        if token.contains('*') || token.contains('?') || token.contains('[') {
            match glob(&token) {
                Ok(matches) if !matches.is_empty() => result.extend(matches),
                _ => result.push(token),
            }
        } else {
            result.push(token);
        }
    }
    result
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
        for entry in fs::read_dir(&dir)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if matches_pattern(&name, &file_pattern) {
                matches.push(dir.join(&name).to_string_lossy().into_owned());
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

    while ni < name_bytes.len() {
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
            while pi < pat_bytes.len() && pat_bytes[pi] != b']' {
                if pi + 2 < pat_bytes.len() && pat_bytes[pi + 1] == b'-' {
                    let start = pat_bytes[pi];
                    let end = pat_bytes[pi + 2];
                    if name_bytes[ni] >= start && name_bytes[ni] <= end {
                        matched = true;
                    }
                    pi += 3;
                } else {
                    if name_bytes[ni] == pat_bytes[pi] {
                        matched = true;
                    }
                    pi += 1;
                }
            }
            if pi < pat_bytes.len() {
                pi += 1;
            }
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
// Command substitution.
// -----------------------------------------------------------------------------
fn expand_command_substitution(
    input: &str,
    last_status: u8,
    rustybox_mode: bool,
    aliases: &mut HashMap<String, String>,
    functions: &mut HashMap<String, String>,
) -> String {
    let mut result = String::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'$' && i + 1 < bytes.len() && bytes[i + 1] == b'(' {
            let mut depth = 1;
            let mut j = i + 2;
            while j < bytes.len() && depth > 0 {
                if bytes[j] == b'(' {
                    depth += 1;
                } else if bytes[j] == b')' {
                    depth -= 1;
                }
                j += 1;
            }
            if depth == 0 {
                let cmd = &input[i + 2..j - 1];
                let output =
                    capture_command_output(cmd, last_status, rustybox_mode, aliases, functions);
                result.push_str(output.trim());
                i = j;
                continue;
            }
        } else if bytes[i] == b'`' {
            let mut j = i + 1;
            while j < bytes.len() && bytes[j] != b'`' {
                j += 1;
            }
            if j < bytes.len() {
                let cmd = &input[i + 1..j];
                let output =
                    capture_command_output(cmd, last_status, rustybox_mode, aliases, functions);
                result.push_str(output.trim());
                i = j + 1;
                continue;
            }
        }
        result.push(bytes[i] as char);
        i += 1;
    }
    result
}

fn capture_command_output(
    cmd: &str,
    last_status: u8,
    rustybox_mode: bool,
    aliases: &mut HashMap<String, String>,
    functions: &mut HashMap<String, String>,
) -> String {
    let fds = match pipe() {
        Ok(fds) => fds,
        Err(_) => return String::new(),
    };
    let pid = unsafe { libc::fork() };
    if pid < 0 {
        unsafe {
            libc::close(fds.0);
            libc::close(fds.1);
        }
        return String::new();
    }
    if pid == 0 {
        unsafe {
            libc::close(fds.0);
            libc::dup2(fds.1, libc::STDOUT_FILENO);
            libc::close(fds.1);
            libc::signal(libc::SIGINT, libc::SIG_DFL);
            libc::signal(libc::SIGQUIT, libc::SIG_DFL);
            let status =
                run_single_command_list(cmd, last_status, rustybox_mode, aliases, functions, false);
            libc::_exit(status as i32);
        }
    }
    unsafe {
        libc::close(fds.1);
    }
    let mut output = String::new();
    let mut buf = [0u8; 4096];
    loop {
        let n = unsafe { libc::read(fds.0, buf.as_mut_ptr() as *mut libc::c_void, buf.len()) };
        if n <= 0 {
            break;
        }
        output.push_str(&String::from_utf8_lossy(&buf[..n as usize]));
    }
    unsafe {
        libc::close(fds.0);
    }
    let mut status: i32 = 0;
    unsafe {
        libc::waitpid(pid, &mut status, 0);
    }
    output
}

// -----------------------------------------------------------------------------
// Redirection parsing.
// -----------------------------------------------------------------------------
fn parse_redirections(
    tokens: Vec<String>,
) -> Result<
    (
        Vec<String>,
        Option<String>,
        Option<String>,
        bool,
        Option<String>,
    ),
    String,
> {
    let mut stdin_file = None;
    let mut stdout_file = None;
    let mut append = false;
    let mut here_doc = None;
    let mut result = Vec::with_capacity(tokens.len());
    let mut i = 0;
    while i < tokens.len() {
        if tokens[i] == "<" {
            if i + 1 >= tokens.len() {
                return Err("missing filename for '<'".into());
            }
            stdin_file = Some(tokens[i + 1].clone());
            i += 2;
        } else if tokens[i] == ">" {
            if i + 1 >= tokens.len() {
                return Err("missing filename for '>'".into());
            }
            stdout_file = Some(tokens[i + 1].clone());
            append = false;
            i += 2;
        } else if tokens[i] == ">>" {
            if i + 1 >= tokens.len() {
                return Err("missing filename for '>>'".into());
            }
            stdout_file = Some(tokens[i + 1].clone());
            append = true;
            i += 2;
        } else if tokens[i] == "<<" {
            if i + 1 >= tokens.len() {
                return Err("missing here-doc delimiter".into());
            }
            // Here-documents are not yet implemented.
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
// -----------------------------------------------------------------------------
fn replace_process_substitution(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if i + 1 < bytes.len() && (bytes[i] == b'<' || bytes[i] == b'>') && bytes[i + 1] == b'(' {
            let direction = bytes[i];
            let mut depth = 1;
            let mut j = i + 2;
            while j < bytes.len() && depth > 0 {
                if bytes[j] == b'(' {
                    depth += 1;
                } else if bytes[j] == b')' {
                    depth -= 1;
                }
                j += 1;
            }
            if depth == 0 {
                let cmd_str = &input[i + 2..j - 1];
                let fds = pipe().expect("pipe failed");
                let pid = unsafe { libc::fork() };
                if pid == 0 {
                    unsafe {
                        if direction == b'<' {
                            libc::close(fds.0);
                            libc::dup2(fds.1, libc::STDOUT_FILENO);
                            libc::close(fds.1);
                        } else {
                            libc::close(fds.1);
                            libc::dup2(fds.0, libc::STDIN_FILENO);
                            libc::close(fds.0);
                        }
                        let expanded = expand_command_substitution(
                            cmd_str,
                            0,
                            false,
                            &mut HashMap::new(),
                            &mut HashMap::new(),
                        );
                        let status = run_command_list(
                            &expanded,
                            0,
                            false,
                            &mut HashMap::new(),
                            &mut HashMap::new(),
                        );
                        libc::_exit(status as i32);
                    }
                }
                if direction == b'<' {
                    unsafe {
                        libc::close(fds.1);
                    }
                    result.push_str(&format!("/dev/fd/{}", fds.0));
                } else {
                    unsafe {
                        libc::close(fds.0);
                    }
                    result.push_str(&format!("/dev/fd/{}", fds.1));
                }
                i = j;
                continue;
            }
        }
        result.push(bytes[i] as char);
        i += 1;
    }
    result
}

// -----------------------------------------------------------------------------
// File descriptor utilities.
// -----------------------------------------------------------------------------
fn pipe() -> io::Result<(RawFd, RawFd)> {
    let mut fds = [-1i32; 2];
    let rc = unsafe { libc::pipe(fds.as_mut_ptr()) };
    if rc < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok((fds[0], fds[1]))
    }
}

fn open_file(path: &str, append: bool) -> io::Result<RawFd> {
    let file = OpenOptions::new()
        .write(true)
        .create(true)
        .append(append)
        .truncate(!append)
        .open(path)?;
    Ok(file.into_raw_fd())
}

// -----------------------------------------------------------------------------
// Registration macro (unchanged).
// -----------------------------------------------------------------------------
register_command!(
    SH_CMD,
    "sh",
    "",
    CommandFlags::BIN.bits() | CommandFlags::NOFORK.bits(),
    sh_main
);
