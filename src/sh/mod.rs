pub mod globals;
pub mod signals;
pub mod parser;
pub mod expansion;
pub mod exec;
pub mod builtins;
pub mod readline;
pub mod script;

use crate::context::Context;
use crate::flags::CommandFlags;
use crate::{register_command, registry};
use crate::sh::globals::*;
use crate::sh::signals::{setup_signal_handlers, setup_nproc_limit};
use crate::sh::readline::{make_prompt, PathCache, ShHelper, ShellOpts};
use crate::sh::exec::{reap_background, run_command_list};
use crate::sh::script::{execute_script, parse_shell_arguments};

use rustyline::completion::FilenameCompleter;
use rustyline::error::ReadlineError;
use rustyline::hint::HistoryHinter;
use rustyline::{CompletionType, Config, EditMode, Editor};
use std::collections::{HashMap, VecDeque};
use std::env;
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;
use std::sync::atomic::Ordering;

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


register_command!(
    SH_CMD,
    "sh",
    "",
    CommandFlags::BIN.bits() | CommandFlags::NOFORK.bits(),
    sh_main
);