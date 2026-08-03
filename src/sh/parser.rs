// =============================================================================
// sh parser — tokenization, command list splitting, pipeline splitting,
// function definition parsing, redirection parsing.
// =============================================================================
use crate::sh::globals::push_char_at;

// -----------------------------------------------------------------------------
// Token representation with quoting metadata.  This allows subsequent
// expansion phases to respect POSIX quoting rules.
// -----------------------------------------------------------------------------
#[derive(Debug, Clone)]
pub struct Token {
    pub value: String,
    /// True if the token was entirely enclosed in single quotes ('...').
    /// Single-quoted tokens must NOT undergo any expansion.
    pub is_single_quoted: bool,
    /// True if the token was entirely enclosed in double quotes ("...").
    /// Double-quoted tokens must NOT undergo word splitting or globbing.
    pub is_double_quoted: bool,
    /// True if the token started with a backslash escape (\).
    /// Escaped tokens should not undergo globbing or command substitution.
    pub is_escaped: bool,
}

// -----------------------------------------------------------------------------
// Tokenizer.  Splits input into tokens while tracking quoting state so that
// later expansion phases can honour POSIX semantics.
// -----------------------------------------------------------------------------
pub fn tokenize(input: &str) -> Vec<Token> {
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
                    // POSIX: inside double quotes, backslash is special only
                    // before $ ` " \ newline.  We preserve the backslash so
                    // expansion can handle it.
                    match bytes[i + 1] {
                        b'$' | b'`' | b'"' | b'\\' | b'\n' => {
                            current.push('\\');
                            i += 1;
                            push_char_at(input, &mut current, &mut i);
                        }
                        _ => {
                            // Consume backslash for other characters.
                            i += 1;
                            push_char_at(input, &mut current, &mut i);
                        }
                    }
                } else {
                    push_char_at(input, &mut current, &mut i);
                }
            } else {
                // Unquoted context.
                if b == b'\'' {
                    has_single = true;
                    in_single = true;
                    i += 1;
                } else if b == b'"' {
                    has_double = true;
                    in_double = true;
                    i += 1;
                } else if b == b'\\' && i + 1 < bytes.len() {
                    // Preserve backslash if it escapes $ or ` so expansion
                    // can handle it.  For other characters, consume it and
                    // mark token as escaped.
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

/// Helper to convert Tokens back to Strings for legacy interfaces where
/// quote info is not needed or has already been processed.
pub fn tokens_to_strings(tokens: Vec<Token>) -> Vec<String> {
    tokens.into_iter().map(|t| t.value).collect()
}

/// Helper to convert Strings to Tokens (unquoted, unescaped) for legacy
/// interfaces.
pub fn strings_to_tokens(strings: Vec<String>) -> Vec<Token> {
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

/// Legacy tokenizer that returns just strings, used for completion and
/// simple parsing.
pub fn tokenize_to_strings(input: &str) -> Vec<String> {
    tokens_to_strings(tokenize(input))
}

// -----------------------------------------------------------------------------
// Split input on top-level shell operators (;, &&, ||, &) while respecting
// single quotes, double quotes, $(...), `...`, and (...) subshells.
// Returns a vector of (command_string, separator) pairs.  The separator
// indicates what followed the command:
//   Some('&')  – followed by &&
//   Some('|')  – followed by ||
//   Some(';')  – followed by ;
//   Some('b')  – followed by & (background)
//   None       – last command (no trailing operator)
// -----------------------------------------------------------------------------
pub fn split_command_list(input: &str) -> Vec<(String, Option<char>)> {
    let bytes = input.as_bytes();
    let mut commands: Vec<(String, Option<char>)> = Vec::new();
    let mut current = String::with_capacity(64);
    let mut in_single = false;
    let mut in_double = false;
    let mut in_backtick = false;
    let mut paren_depth: usize = 0; // tracks $(...) and (...)
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

// -----------------------------------------------------------------------------
// Find the matching closing brace for a function definition, respecting
// quotes, $(...), `...`, and nested braces.
// -----------------------------------------------------------------------------
pub fn find_matching_brace(input: &str, start: usize) -> Option<usize> {
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

// -----------------------------------------------------------------------------
// Quote-aware parsing of function definitions: name() { body; }
// Only matches if the ENTIRE input is a single function definition with a
// valid identifier name.  This prevents false positives on compound commands
// that happen to contain braces.  Trailing comments are allowed.
// -----------------------------------------------------------------------------
pub fn try_parse_function_def(
    input: &str,
    is_valid_identifier: fn(&str) -> bool,
) -> Option<(String, String)> {
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
    // Find the matching closing brace (now aware of nested substitutions).
    let close = find_matching_brace(trimmed, open)?;
    // Check if everything after the closing brace is just a comment or whitespace.
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

// -----------------------------------------------------------------------------
// Split a command string on top-level pipe characters '|', respecting
// quotes, $(...), `...`, and parentheses.  Does NOT split on '||' (that
// is handled at the command-list level).
// -----------------------------------------------------------------------------
pub fn split_pipeline(input: &str) -> Vec<String> {
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

// -----------------------------------------------------------------------------
// Find the matching closing parenthesis for a command substitution,
// respecting single quotes, double quotes, and nested parentheses.
// `start` is the index immediately after the opening '('.
// Returns the index of the matching ')' or None.
// -----------------------------------------------------------------------------
pub fn find_closing_paren(input: &str, start: usize) -> Option<usize> {
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

// -----------------------------------------------------------------------------
// Parse redirections from Token stream.  Done BEFORE globbing to detect
// ambiguous redirects properly.
// -----------------------------------------------------------------------------
pub fn parse_redirections_tokens(
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
