// =============================================================================
// sort — Sort, merge, or sequence check text files.
// =============================================================================
// Copyright (c) 2026 Semyon Tsarev
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// project, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Implementation inspired by Toybox (https://landley.net/toybox/)
// Toybox is copyrighted by Rob Landley, see NOTICE file for license details.
//
// Supported options:
//   -b        Ignore leading blanks when determining sort keys.
//   -c        Check that input is sorted; do not output.
//   -C        Like -c but silent on disorder.
//   -d        Dictionary order: only blanks and alphanumerics are significant.
//   -f        Fold case: treat lowercase as uppercase equivalent.
//   -i        Ignore non-printable characters.
//   -k KEYDEF Define a sort key field.
//   -m        Merge already-sorted files.
//   -n        Numeric sort.
//   -o FILE   Write output to FILE instead of stdout.
//   -r        Reverse comparison result.
//   -t CHAR   Use CHAR as field separator.
//   -u        Unique: suppress duplicate lines based on sort keys.
// =============================================================================

use crate::context::Context;
use crate::flags::CommandFlags;
use std::cell::RefCell;
use std::cmp::Ordering;
use std::collections::BinaryHeap;
use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Write};

#[derive(Clone)]
struct KeySpec {
    field_start: usize,
    char_start: Option<usize>,
    field_end: Option<usize>,
    char_end: Option<usize>,
    ignore_blanks: bool,
    numeric: bool,
    reverse: bool,
    fold_case: bool,
    dictionary: bool,
    ignore_nonprint: bool,
}

impl KeySpec {
    fn new() -> Self {
        Self {
            field_start: 1,
            char_start: None,
            field_end: None,
            char_end: None,
            ignore_blanks: false,
            numeric: false,
            reverse: false,
            fold_case: false,
            dictionary: false,
            ignore_nonprint: false,
        }
    }
}

struct SortConfig {
    ignore_blanks: bool,
    numeric: bool,
    reverse: bool,
    fold_case: bool,
    dictionary: bool,
    ignore_nonprint: bool,
    unique: bool,
    check: bool,
    check_silent: bool,
    merge: bool,
    separator: Option<char>,
    output_file: Option<String>,
    keys: Vec<KeySpec>,
}

fn parse_keydef(s: &str, global: &SortConfig) -> Result<KeySpec, String> {
    let mut spec = KeySpec::new();
    let (start_part, end_part) = if let Some(pos) = s.find(',') {
        (&s[..pos], Some(&s[pos + 1..]))
    } else {
        (s, None)
    };

    let (start_nums, start_mods) = split_nums_and_mods(start_part);
    let (fs, cs) = parse_field_char(start_nums)?;
    spec.field_start = fs;
    spec.char_start = cs;

    if let Some(end_str) = end_part {
        let (end_nums, end_mods) = split_nums_and_mods(end_str);
        let (fe, ce) = parse_field_char(end_nums)?;
        spec.field_end = Some(fe);
        spec.char_end = ce;
        apply_modifiers(&mut spec, end_mods);
    }
    apply_modifiers(&mut spec, start_mods);

    let has_per_key = spec.ignore_blanks
        || spec.numeric
        || spec.reverse
        || spec.fold_case
        || spec.dictionary
        || spec.ignore_nonprint;

    if !has_per_key {
        spec.ignore_blanks = global.ignore_blanks;
        spec.numeric = global.numeric;
        spec.reverse = global.reverse;
        spec.fold_case = global.fold_case;
        spec.dictionary = global.dictionary;
        spec.ignore_nonprint = global.ignore_nonprint;
    }
    Ok(spec)
}

fn split_nums_and_mods(s: &str) -> (&str, &str) {
    let first_alpha = s.find(|c: char| c.is_ascii_alphabetic());
    match first_alpha {
        Some(pos) => (&s[..pos], &s[pos..]),
        None => (s, ""),
    }
}

fn parse_field_char(s: &str) -> Result<(usize, Option<usize>), String> {
    if let Some(dot) = s.find('.') {
        let field: usize = s[..dot]
            .parse()
            .map_err(|_| format!("invalid field '{}'", &s[..dot]))?;
        let ch: usize = s[dot + 1..]
            .parse()
            .map_err(|_| format!("invalid char '{}'", &s[dot + 1..]))?;
        Ok((field, Some(ch)))
    } else {
        let field: usize = s.parse().map_err(|_| format!("invalid field '{}'", s))?;
        Ok((field, None))
    }
}

fn apply_modifiers(spec: &mut KeySpec, mods: &str) {
    for ch in mods.chars() {
        match ch {
            'b' => spec.ignore_blanks = true,
            'd' => spec.dictionary = true,
            'f' => spec.fold_case = true,
            'i' => spec.ignore_nonprint = true,
            'n' => spec.numeric = true,
            'r' => spec.reverse = true,
            _ => {}
        }
    }
}

// =============================================================================
// FAST PATH OPTIMIZATIONS
// =============================================================================

/// Check if we can use the ultra-fast path (no complex -k, -f, -d, -i)
fn can_use_fast_path(config: &SortConfig) -> bool {
    if config.fold_case || config.dictionary || config.ignore_nonprint {
        return false;
    }
    if config.keys.len() != 1 {
        return false;
    }
    let k = &config.keys[0];
    // Fast path only if sorting the whole line, or a simple field without char offsets
    if k.char_start.is_some() || k.char_end.is_some() {
        return false;
    }
    if k.field_start != 1 || k.field_end.is_some() {
        return false; // Complex field extraction needed
    }
    true
}

/// Ultra-fast numeric sort using index sorting (Zero string moves)
fn sort_numeric_fast(
    lines: &[String],
    reverse: bool,
    unique: bool,
    writer: &mut Box<dyn Write>,
) -> u8 {
    // (value, original_index)
    let mut indexed: Vec<(f64, usize)> = lines
        .iter()
        .enumerate()
        .map(|(i, line)| {
            let val = line.trim().parse::<f64>().unwrap_or(0.0);
            (val, i)
        })
        .collect();

    indexed.sort_unstable_by(|a, b| {
        let cmp = a.0.total_cmp(&b.0);
        if reverse { cmp.reverse() } else { cmp }
    });

    if unique {
        indexed.dedup_by(|a, b| a.0 == b.0);
    }

    for &(_, idx) in &indexed {
        if writeln!(writer, "{}", lines[idx]).is_err() {
            return 2;
        }
    }
    0
}

/// Ultra-fast lexicographic sort (Zero-allocation slice sorting)
fn sort_string_fast(
    lines: &mut Vec<String>,
    reverse: bool,
    unique: bool,
    writer: &mut Box<dyn Write>,
) -> u8 {
    if reverse {
        lines.sort_unstable_by(|a, b| b.cmp(a));
    } else {
        lines.sort_unstable();
    }

    if unique {
        lines.dedup_by(|a, b| a == b);
    }

    for line in lines {
        if writeln!(writer, "{}", line).is_err() {
            return 2;
        }
    }
    0
}

// =============================================================================
// GENERIC PATH (For complex -k, modifiers, etc.)
// =============================================================================

enum SortKey<'a> {
    Numeric(f64),
    Borrowed(&'a str),
    Owned(String),
}

#[derive(Clone)]
enum SortKeyOwned {
    Numeric(f64),
    Text(String),
}

impl PartialEq for SortKeyOwned {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (SortKeyOwned::Numeric(a), SortKeyOwned::Numeric(b)) => {
                a.total_cmp(b) == Ordering::Equal
            }
            (SortKeyOwned::Text(a), SortKeyOwned::Text(b)) => a == b,
            _ => false,
        }
    }
}
impl Eq for SortKeyOwned {}

impl PartialOrd for SortKeyOwned {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for SortKeyOwned {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            (SortKeyOwned::Numeric(a), SortKeyOwned::Numeric(b)) => a.total_cmp(b),
            (SortKeyOwned::Text(a), SortKeyOwned::Text(b)) => a.cmp(b),
            (SortKeyOwned::Numeric(_), SortKeyOwned::Text(_)) => Ordering::Less,
            (SortKeyOwned::Text(_), SortKeyOwned::Numeric(_)) => Ordering::Greater,
        }
    }
}

struct Record<'a> {
    line: &'a str,
    keys: Vec<SortKey<'a>>,
}

#[derive(Clone)]
struct MergeRecord {
    line: String,
    keys: Vec<SortKeyOwned>,
}

fn get_field_range(line: &str, field_num: usize, separator: Option<char>) -> (usize, usize) {
    if field_num == 0 {
        return (0, line.len());
    }
    let mut current_field = 1;
    let mut in_field = false;
    let mut start_idx = 0;
    let is_sep = |c: char| -> bool {
        if let Some(sep) = separator {
            c == sep
        } else {
            c == ' ' || c == '\t'
        }
    };

    for (i, c) in line.char_indices() {
        if is_sep(c) {
            if in_field {
                if current_field == field_num {
                    return (start_idx, i);
                }
                in_field = false;
            }
        } else {
            if !in_field {
                if current_field == field_num {
                    start_idx = i;
                }
                in_field = true;
                current_field += 1;
            }
        }
    }
    if in_field && current_field - 1 == field_num {
        return (start_idx, line.len());
    }
    (0, 0)
}

fn build_record<'a>(line: &'a str, config: &SortConfig) -> Record<'a> {
    let mut keys = Vec::with_capacity(config.keys.len());
    for spec in &config.keys {
        let (start, end) = get_field_range(line, spec.field_start, config.separator);
        let (mut f_start, mut f_end) = if start == 0 && end == 0 {
            (0, 0)
        } else if let Some(fe) = spec.field_end {
            let (_, fe_end) = get_field_range(line, fe, config.separator);
            (start, fe_end.max(start))
        } else {
            (start, end)
        };

        let mut key_slice = &line[f_start..f_end];
        if spec.char_start.is_some() || spec.char_end.is_some() {
            let cs = spec.char_start.map(|c| c.saturating_sub(1)).unwrap_or(0);
            let ce = spec.char_end.unwrap_or(usize::MAX);
            let mut start_byte = 0;
            let mut end_byte = key_slice.len();
            let mut char_idx = 0;
            for (i, _) in key_slice.char_indices() {
                if char_idx == cs {
                    start_byte = i;
                }
                if char_idx == ce {
                    end_byte = i;
                    break;
                }
                char_idx += 1;
            }
            if cs >= char_idx && char_idx > 0 {
                start_byte = key_slice.len();
            }
            key_slice = &key_slice[start_byte..end_byte];
        }
        if spec.ignore_blanks {
            key_slice = key_slice.trim_start_matches(|c| c == ' ' || c == '\t');
        }

        let sort_key = if spec.numeric {
            SortKey::Numeric(
                key_slice
                    .trim_start_matches(|c| c == ' ' || c == '\t')
                    .parse::<f64>()
                    .unwrap_or(0.0),
            )
        } else if spec.fold_case || spec.dictionary || spec.ignore_nonprint {
            let mut transformed = String::with_capacity(key_slice.len());
            for c in key_slice.chars() {
                if spec.dictionary && !(c.is_alphanumeric() || c.is_whitespace()) {
                    continue;
                }
                if spec.ignore_nonprint && c.is_control() {
                    continue;
                }
                if spec.fold_case {
                    transformed.extend(c.to_lowercase());
                } else {
                    transformed.push(c);
                }
            }
            SortKey::Owned(transformed)
        } else {
            SortKey::Borrowed(key_slice)
        };
        keys.push(sort_key);
    }
    Record { line, keys }
}

fn build_merge_record(line: String, config: &SortConfig) -> MergeRecord {
    let mut keys = Vec::with_capacity(config.keys.len());
    for spec in &config.keys {
        let (start, end) = get_field_range(&line, spec.field_start, config.separator);
        let (mut f_start, mut f_end) = if start == 0 && end == 0 {
            (0, 0)
        } else if let Some(fe) = spec.field_end {
            let (_, fe_end) = get_field_range(&line, fe, config.separator);
            (start, fe_end.max(start))
        } else {
            (start, end)
        };

        let mut key_slice = &line[f_start..f_end];
        if spec.char_start.is_some() || spec.char_end.is_some() {
            let cs = spec.char_start.map(|c| c.saturating_sub(1)).unwrap_or(0);
            let ce = spec.char_end.unwrap_or(usize::MAX);
            let mut start_byte = 0;
            let mut end_byte = key_slice.len();
            let mut char_idx = 0;
            for (i, _) in key_slice.char_indices() {
                if char_idx == cs {
                    start_byte = i;
                }
                if char_idx == ce {
                    end_byte = i;
                    break;
                }
                char_idx += 1;
            }
            if cs >= char_idx && char_idx > 0 {
                start_byte = key_slice.len();
            }
            key_slice = &key_slice[start_byte..end_byte];
        }
        if spec.ignore_blanks {
            key_slice = key_slice.trim_start_matches(|c| c == ' ' || c == '\t');
        }

        let sort_key = if spec.numeric {
            SortKeyOwned::Numeric(
                key_slice
                    .trim_start_matches(|c| c == ' ' || c == '\t')
                    .parse::<f64>()
                    .unwrap_or(0.0),
            )
        } else if spec.fold_case || spec.dictionary || spec.ignore_nonprint {
            let mut transformed = String::with_capacity(key_slice.len());
            for c in key_slice.chars() {
                if spec.dictionary && !(c.is_alphanumeric() || c.is_whitespace()) {
                    continue;
                }
                if spec.ignore_nonprint && c.is_control() {
                    continue;
                }
                if spec.fold_case {
                    transformed.extend(c.to_lowercase());
                } else {
                    transformed.push(c);
                }
            }
            SortKeyOwned::Text(transformed)
        } else {
            SortKeyOwned::Text(key_slice.to_string())
        };
        keys.push(sort_key);
    }
    MergeRecord { line, keys }
}

fn compare_records(a: &Record, b: &Record, config: &SortConfig) -> Ordering {
    for (i, spec) in config.keys.iter().enumerate() {
        let key_a = &a.keys[i];
        let key_b = &b.keys[i];
        let cmp = match (key_a, key_b) {
            (SortKey::Numeric(na), SortKey::Numeric(nb)) => na.total_cmp(nb),
            (SortKey::Borrowed(sa), SortKey::Borrowed(sb)) => sa.cmp(sb),
            (SortKey::Owned(sa), SortKey::Owned(sb)) => sa.cmp(sb),
            (SortKey::Borrowed(sa), SortKey::Owned(sb)) => (*sa).cmp(sb.as_str()),
            (SortKey::Owned(sa), SortKey::Borrowed(sb)) => sa.as_str().cmp(*sb),
            _ => Ordering::Equal,
        };
        let cmp = if spec.reverse { cmp.reverse() } else { cmp };
        if cmp != Ordering::Equal {
            return cmp;
        }
    }
    a.line.cmp(b.line)
}

fn compare_merge_records(a: &MergeRecord, b: &MergeRecord, config: &SortConfig) -> Ordering {
    for (i, spec) in config.keys.iter().enumerate() {
        let key_a = &a.keys[i];
        let key_b = &b.keys[i];
        let cmp = match (key_a, key_b) {
            (SortKeyOwned::Numeric(na), SortKeyOwned::Numeric(nb)) => na.total_cmp(nb),
            (SortKeyOwned::Text(sa), SortKeyOwned::Text(sb)) => sa.cmp(sb),
            _ => Ordering::Equal,
        };
        let cmp = if spec.reverse { cmp.reverse() } else { cmp };
        if cmp != Ordering::Equal {
            return cmp;
        }
    }
    a.line.cmp(&b.line)
}

fn records_equal(a: &Record, b: &Record, config: &SortConfig) -> bool {
    compare_records(a, b, config) == Ordering::Equal
}

fn merge_records_equal(a: &MergeRecord, b: &MergeRecord, config: &SortConfig) -> bool {
    compare_merge_records(a, b, config) == Ordering::Equal
}

fn check_sorted_streaming<R: BufRead>(reader: R, config: &SortConfig, filename: &str) -> u8 {
    let mut lines = reader.lines();
    let mut prev_line: Option<String> = None;
    let mut line_num = 0;

    for line_result in lines {
        line_num += 1;
        let curr_line = match line_result {
            Ok(l) => l,
            Err(_) => return 2,
        };

        if let Some(ref p_line) = prev_line {
            let prev_rec = build_record(p_line, config);
            let curr_rec = build_record(&curr_line, config);
            let cmp = compare_records(&prev_rec, &curr_rec, config);

            if config.unique && cmp == Ordering::Equal {
                if !config.check_silent {
                    eprintln!("sort: {}: {}: disorder (duplicate key)", filename, line_num);
                }
                return 1;
            }
            if cmp == Ordering::Greater {
                if !config.check_silent {
                    eprintln!("sort: {}: {}: disorder", filename, line_num);
                }
                return 1;
            }
        }
        prev_line = Some(curr_line);
    }
    0
}

thread_local! {
    static MERGE_CONFIG: RefCell<Option<*const SortConfig>> = RefCell::new(None);
}

struct HeapItem {
    file_idx: usize,
    record: MergeRecord,
}

impl PartialEq for HeapItem {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}
impl Eq for HeapItem {}
impl PartialOrd for HeapItem {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for HeapItem {
    fn cmp(&self, other: &Self) -> Ordering {
        MERGE_CONFIG.with(|c| {
            let ptr = c.borrow().expect("Merge config not set");
            let cfg = unsafe { &*ptr };
            compare_merge_records(&self.record, &other.record, cfg).reverse()
        })
    }
}

enum MergeSource {
    File(std::io::Lines<BufReader<File>>),
    Memory(std::vec::IntoIter<String>),
}

fn merge_sorted_files(files: &[String], config: &SortConfig) -> u8 {
    let mut conflict_data: Option<(usize, Vec<String>)> = None;
    if let Some(ref out) = config.output_file {
        if let Some(pos) = files.iter().position(|f| f == out) {
            if let Ok(f) = File::open(out) {
                let lines: Vec<String> = BufReader::new(f).lines().filter_map(Result::ok).collect();
                conflict_data = Some((pos, lines));
            }
        }
    }

    let conflict_pos = conflict_data.as_ref().map(|(pos, _)| *pos);
    let mut sources: Vec<MergeSource> = Vec::with_capacity(files.len());
    let mut heap: BinaryHeap<HeapItem> = BinaryHeap::new();

    for (i, file) in files.iter().enumerate() {
        if conflict_pos == Some(i) {
            sources.push(MergeSource::Memory(Vec::new().into_iter()));
            continue;
        }
        let f = match File::open(file) {
            Ok(f) => f,
            Err(e) => {
                eprintln!("sort: '{}': {}", file, e);
                return 2;
            }
        };
        sources.push(MergeSource::File(BufReader::new(f).lines()));
    }

    let mut conflict_iter = conflict_data.map(|(_, lines)| lines.into_iter());

    for i in 0..files.len() {
        let line_opt = if conflict_pos == Some(i) {
            conflict_iter.as_mut().and_then(|iter| iter.next())
        } else {
            if let MergeSource::File(ref mut reader) = sources[i] {
                reader.next().and_then(Result::ok)
            } else {
                None
            }
        };

        if let Some(line) = line_opt {
            heap.push(HeapItem {
                file_idx: i,
                record: build_merge_record(line, config),
            });
        }
    }

    let mut writer: Box<dyn Write> = if let Some(ref outfile) = config.output_file {
        match File::create(outfile) {
            Ok(f) => Box::new(BufWriter::new(f)),
            Err(e) => {
                eprintln!("sort: '{}': {}", outfile, e);
                return 2;
            }
        }
    } else {
        Box::new(BufWriter::new(std::io::stdout().lock()))
    };

    let mut last_written: Option<MergeRecord> = None;
    MERGE_CONFIG.with(|c| *c.borrow_mut() = Some(config as *const _));

    while let Some(mut item) = heap.pop() {
        let is_duplicate = if config.unique {
            if let Some(ref last) = last_written {
                merge_records_equal(last, &item.record, config)
            } else {
                false
            }
        } else {
            false
        };

        if !is_duplicate {
            if writeln!(writer, "{}", item.record.line).is_err() {
                MERGE_CONFIG.with(|c| *c.borrow_mut() = None);
                return 2;
            }
            last_written = Some(item.record.clone());
        }

        let next_line = if conflict_pos == Some(item.file_idx) {
            conflict_iter.as_mut().and_then(|iter| iter.next())
        } else {
            if let MergeSource::File(ref mut reader) = sources[item.file_idx] {
                reader.next().and_then(Result::ok)
            } else {
                None
            }
        };

        if let Some(line) = next_line {
            item.record = build_merge_record(line, config);
            heap.push(item);
        }
    }

    MERGE_CONFIG.with(|c| *c.borrow_mut() = None);
    writer.flush().ok();
    0
}

fn sort_main(ctx: &mut Context) -> u8 {
    let opts = match crate::args::parse(ctx, "bcCdfik:mno:rt:u") {
        Ok(o) => o,
        Err(e) => {
            eprintln!("sort: {e}");
            return 2;
        }
    };

    let mut config = SortConfig {
        ignore_blanks: opts.count('b') > 0,
        numeric: opts.count('n') > 0,
        reverse: opts.count('r') > 0,
        fold_case: opts.count('f') > 0,
        dictionary: opts.count('d') > 0,
        ignore_nonprint: opts.count('i') > 0,
        unique: opts.count('u') > 0,
        check: opts.count('c') > 0,
        check_silent: opts.count('C') > 0,
        merge: opts.count('m') > 0,
        separator: opts.get_str('t').and_then(|s| s.chars().next()),
        output_file: opts.get_str('o').map(|s| s.to_string()),
        keys: Vec::new(),
    };

    for kdef in opts.get_strs('k') {
        match parse_keydef(kdef, &config) {
            Ok(spec) => config.keys.push(spec),
            Err(e) => {
                eprintln!("sort: {e}");
                return 2;
            }
        }
    }

    if config.keys.is_empty() {
        let mut default_key = KeySpec::new();
        default_key.ignore_blanks = config.ignore_blanks;
        default_key.numeric = config.numeric;
        default_key.reverse = config.reverse;
        default_key.fold_case = config.fold_case;
        default_key.dictionary = config.dictionary;
        default_key.ignore_nonprint = config.ignore_nonprint;
        config.keys.push(default_key);
    }

    if config.check || config.check_silent {
        if ctx.optargs.is_empty() || ctx.optargs.first().map(|s| s.as_str()) == Some("-") {
            return check_sorted_streaming(std::io::stdin().lock(), &config, "stdin");
        } else if ctx.optargs.len() == 1 {
            let file = &ctx.optargs[0];
            match File::open(file) {
                Ok(f) => return check_sorted_streaming(BufReader::new(f), &config, file),
                Err(e) => {
                    eprintln!("sort: '{}': {}", file, e);
                    return 2;
                }
            }
        } else {
            eprintln!("sort: multiple files not allowed with -c/-C");
            return 2;
        }
    }

    if config.merge {
        return merge_sorted_files(&ctx.optargs, &config);
    }

    // FAST I/O: Proper buffer reuse without allocation bombs
    let mut lines: Vec<String> = Vec::new();
    let mut buf = String::with_capacity(4096);

    let mut process_reader = |mut reader: Box<dyn BufRead>| -> u8 {
        loop {
            buf.clear();
            match reader.read_line(&mut buf) {
                Ok(0) => break,
                Ok(_) => {
                    // Strip trailing newline safely
                    if buf.ends_with('\n') {
                        buf.pop();
                        if buf.ends_with('\r') {
                            buf.pop();
                        }
                    }
                    lines.push(buf.clone()); // Clone exact size, buf retains 4096 capacity
                }
                Err(_) => return 2,
            }
        }
        0
    };

    let exit_code =
        if ctx.optargs.is_empty() || ctx.optargs.first().map(|s| s.as_str()) == Some("-") {
            process_reader(Box::new(std::io::stdin().lock()))
        } else {
            let mut code = 0;
            for file in &ctx.optargs {
                match File::open(file) {
                    Ok(f) => {
                        if process_reader(Box::new(BufReader::new(f))) != 0 {
                            eprintln!("sort: '{}': read error", file);
                            code = 2;
                        }
                    }
                    Err(e) => {
                        eprintln!("sort: '{}': {}", file, e);
                        code = 2;
                    }
                }
            }
            code
        };

    if exit_code != 0 || lines.is_empty() {
        return exit_code;
    }

    let mut writer: Box<dyn Write> = if let Some(ref outfile) = config.output_file {
        match File::create(outfile) {
            Ok(f) => Box::new(BufWriter::new(f)),
            Err(e) => {
                eprintln!("sort: '{}': {}", outfile, e);
                return 2;
            }
        }
    } else {
        Box::new(BufWriter::new(std::io::stdout().lock()))
    };

    // ==========================================================
    // FAST PATH DISPATCH
    // ==========================================================
    if can_use_fast_path(&config) {
        if config.numeric {
            return sort_numeric_fast(&lines, config.reverse, config.unique, &mut writer);
        } else {
            return sort_string_fast(&mut lines, config.reverse, config.unique, &mut writer);
        }
    }

    // ==========================================================
    // GENERIC PATH (Schwartzian transform)
    // ==========================================================
    let mut records: Vec<Record> = lines
        .iter()
        .map(|line| build_record(line, &config))
        .collect();
    records.sort_unstable_by(|a, b| compare_records(a, b, &config));

    if config.unique {
        records.dedup_by(|a, b| records_equal(a, b, &config));
    }

    for rec in &records {
        if writeln!(writer, "{}", rec.line).is_err() {
            return 2;
        }
    }

    writer.flush().ok();
    0
}

register_command!(
    SORT_CMD,
    "sort",
    "bcCdfik:mno:rt:u",
    CommandFlags::BIN.bits() | CommandFlags::USR.bits(),
    sort_main,
    description = "Sort, merge, or sequence check text files",
    help = "\
OPTIONS:
-b        Ignore leading blanks when determining sort keys.
-c        Check that input is sorted; do not output.
-C        Like -c but silent on disorder.
-d        Dictionary order: only blanks and alphanumerics are significant.
-f        Fold case: treat lowercase as uppercase equivalent.
-i        Ignore non-printable characters.
-k KEYDEF Define a sort key field.
-m        Merge already-sorted files.
-n        Numeric sort.
-o FILE   Write output to FILE instead of stdout.
-r        Reverse comparison result.
-t CHAR   Use CHAR as field separator.
-u        Unique: suppress duplicate lines based on sort keys."
);
