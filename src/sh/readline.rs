// =============================================================================
// sh readline — rustyline integration: completion, hints, highlighting,
// validation, and dynamic prompt generation.
// =============================================================================
use crate::sh::expansion::expand_tilde_one;
use crate::sh::globals::*;
use crate::sh::parser::tokenize_to_strings;
use rustyline::completion::{Completer, FilenameCompleter, Pair};
use rustyline::error::ReadlineError;
use rustyline::highlight::Highlighter;
use rustyline::hint::{Hinter, HistoryHinter};
use rustyline::validate::{ValidationResult, Validator};
use rustyline::Helper;
use std::borrow::Cow;
use std::env;
use std::ffi::CStr;
use std::fs;
use std::path::PathBuf;

// -----------------------------------------------------------------------------
// Path cache for faster command lookup during completion.
// -----------------------------------------------------------------------------
#[derive(Clone, Default)]
pub struct PathCache {
    pub entries: Vec<String>,
    pub last_path: String,
}

impl PathCache {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            last_path: String::new(),
        }
    }
    pub fn refresh(&mut self) {
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
    pub fn all_commands(&mut self) -> Vec<String> {
        self.refresh();
        self.entries.clone()
    }
}

// -----------------------------------------------------------------------------
// Static completions for common command options.
// -----------------------------------------------------------------------------
pub fn static_options(cmd: &str) -> Vec<&'static str> {
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
pub const SENSITIVE_VAR_PREFIXES: &[&str] = &[
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

pub fn is_sensitive_var(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    SENSITIVE_VAR_PREFIXES
        .iter()
        .any(|prefix| upper.contains(prefix))
}

// -----------------------------------------------------------------------------
// Custom rustyline Helper: command/option/variable completion, history hints,
// syntax highlighting.
// -----------------------------------------------------------------------------
pub struct ShHelper {
    pub file_completer: FilenameCompleter,
    pub history_hinter: HistoryHinter,
    pub builtins: Vec<String>,
    pub path_cache: PathCache,
    pub opts: ShellOpts,
}

#[derive(Clone)]
pub struct ShellOpts {
    pub pipefail: bool,
    pub hist_control: Option<String>,
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
        // Option completion after a command.
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
        // First word: command + builtins.
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
        // Otherwise: file completion with tilde expansion.
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
    fn highlight<'l>(&self, line: &'l str, _pos: usize) -> Cow<'l, str> {
        let single_count = line.chars().filter(|&c| c == '\'').count();
        let double_count = line.chars().filter(|&c| c == '"').count();
        if single_count % 2 != 0 || double_count % 2 != 0 {
            return Cow::Owned(format!("{}{}{}", ANSI_RED, line, ANSI_RESET));
        }
        Cow::Borrowed(line)
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
pub fn extract_word(line: &str, pos: usize) -> (usize, &str) {
    let bytes = line.as_bytes();
    let mut start = pos;
    while start > 0 && !bytes[start - 1].is_ascii_whitespace() {
        start -= 1;
    }
    (start, &line[start..pos])
}

pub fn is_first_word(line: &str, word_start: usize) -> bool {
    line[..word_start].trim().is_empty()
}

pub fn get_first_command(line: &str) -> Option<String> {
    let tokens = tokenize_to_strings(line);
    tokens.into_iter().next()
}

// -----------------------------------------------------------------------------
// Prompt generation.
// -----------------------------------------------------------------------------
pub fn make_prompt() -> String {
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