use std::fs;
use std::path::Path;

fn main() {
    let mut changed_files = Vec::new();
    visit_dir(Path::new("docs"), &mut changed_files);
    if fs::metadata("README.md").is_ok() && process_file(Path::new("README.md")) {
        changed_files.push("README.md".to_string());
    }

    if changed_files.is_empty() {
        std::process::exit(0);
    }

    for path in &changed_files {
        eprintln!("  Padded tables in {path}");
    }
    // Exit 0 — padding is a fix, not a failure
}

fn visit_dir(dir: &Path, changed: &mut Vec<String>) {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    let mut entries: Vec<_> = entries.filter_map(|e| e.ok()).collect();
    entries.sort_by_key(|e| e.file_name());

    for entry in entries {
        let path = entry.path();
        if path.is_dir() {
            visit_dir(&path, changed);
        } else if path.extension().and_then(|e| e.to_str()) == Some("md") {
            if process_file(&path) {
                changed.push(path.display().to_string());
            }
        }
    }
}

fn process_file(path: &Path) -> bool {
    let content = match fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return false,
    };
    let lines: Vec<&str> = content.split('\n').collect();
    let mut result: Vec<String> = Vec::with_capacity(lines.len());
    let mut changed = false;
    let mut i = 0;

    while i < lines.len() {
        if parse_row(lines[i]).is_some() {
            let mut table_lines = Vec::new();
            while i < lines.len() && parse_row(lines[i]).is_some() {
                table_lines.push(lines[i]);
                i += 1;
            }

            if table_lines.len() >= 2 {
                let second = parse_row(table_lines[1]).unwrap();
                if is_separator(&second) {
                    let padded = pad_table(&table_lines);
                    for (j, orig) in table_lines.iter().enumerate() {
                        if padded[j] != *orig {
                            changed = true;
                        }
                    }
                    result.extend(padded);
                } else {
                    result.extend(table_lines.iter().map(|s| s.to_string()));
                }
            } else {
                result.extend(table_lines.iter().map(|s| s.to_string()));
            }
        } else {
            result.push(lines[i].to_string());
            i += 1;
        }
    }

    if changed {
        let new_content = result.join("\n");
        fs::write(path, new_content).expect("Failed to write file");
        true
    } else {
        false
    }
}

fn parse_row(line: &str) -> Option<Vec<String>> {
    let trimmed = line.trim();
    if !trimmed.starts_with('|') || !trimmed.ends_with('|') || trimmed.len() < 2 {
        return None;
    }
    let inner = &trimmed[1..trimmed.len() - 1];
    let cells: Vec<String> = inner.split('|').map(|c| c.trim().to_string()).collect();
    Some(cells)
}

fn is_separator(cells: &[String]) -> bool {
    if cells.is_empty() {
        return false;
    }
    cells.iter().all(|c| {
        let mut chars = c.chars();
        let first = match chars.next() {
            Some(ch) => ch,
            None => return false,
        };
        let rest_start = if first == ':' {
            match chars.next() {
                Some(ch) => ch,
                None => return false,
            }
        } else {
            first
        };
        if rest_start != '-' {
            return false;
        }
        let mut saw_colon = false;
        for ch in chars {
            if saw_colon { return false; }
            if ch == '-' { continue; }
            if ch == ':' { saw_colon = true; } else { return false; }
        }
        true
    })
}

fn visual_width(s: &str) -> usize {
    s.chars().count()
}

fn format_separator_cell(original: &str, width: usize) -> String {
    let left = original.starts_with(':');
    let right = original.ends_with(':');
    let colon_width = usize::from(left) + usize::from(right);
    let dash_count = if width > colon_width { width - colon_width } else { 1 };
    let mut s = String::with_capacity(width);
    if left { s.push(':'); }
    for _ in 0..dash_count { s.push('-'); }
    if right { s.push(':'); }
    s
}

fn pad_table(lines: &[&str]) -> Vec<String> {
    let mut rows: Vec<Vec<String>> = Vec::new();
    for line in lines {
        match parse_row(line) {
            Some(cells) => rows.push(cells),
            None => return lines.iter().map(|s| s.to_string()).collect(),
        }
    }
    if rows.len() < 2 {
        return lines.iter().map(|s| s.to_string()).collect();
    }

    let max_cols = rows.iter().map(|r| r.len()).max().unwrap_or(0);
    for row in &mut rows {
        while row.len() < max_cols { row.push(String::new()); }
    }

    let mut col_widths = vec![0usize; max_cols];
    for (i, row) in rows.iter().enumerate() {
        if i == 1 && is_separator(row) { continue; }
        for (j, cell) in row.iter().enumerate() {
            col_widths[j] = col_widths[j].max(visual_width(cell));
        }
    }

    for w in &mut col_widths {
        if *w < 3 { *w = 3; }
    }

    let mut result = Vec::with_capacity(rows.len());
    for (i, row) in rows.iter().enumerate() {
        let mut line = String::from("|");
        if i == 1 && is_separator(row) {
            for (j, cell) in row.iter().enumerate() {
                line.push(' ');
                line.push_str(&format_separator_cell(cell, col_widths[j]));
                line.push(' ');
                line.push('|');
            }
        } else {
            for (j, cell) in row.iter().enumerate() {
                line.push(' ');
                line.push_str(cell);
                for _ in 0..col_widths[j] - visual_width(cell) { line.push(' '); }
                line.push(' ');
                line.push('|');
            }
        }
        result.push(line);
    }
    result
}
