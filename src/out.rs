//! Output rendering: table / tsv / json / jsonl, field selection,
//! truncation, and `--count --by` aggregation.

use std::collections::HashMap;
use std::io::Write;

#[derive(Clone, Copy, PartialEq, Eq, Debug, clap::ValueEnum)]
pub enum Format {
    Table,
    Tsv,
    Json,
    Jsonl,
}

#[derive(Clone, Debug)]
pub struct OutOpts {
    pub format: Format,
    pub fields: Option<Vec<String>>,
    pub full: bool,
    pub limit: Option<usize>,
    pub count: bool,
    pub by: Option<String>,
}

pub struct Table {
    pub headers: Vec<&'static str>,
    pub rows: Vec<Vec<String>>,
}

const TRUNCATE_AT: usize = 300;
const DEFAULT_TABLE_LIMIT: usize = 200;

fn truncate(s: &str, at: usize) -> String {
    if s.chars().count() <= at {
        s.to_string()
    } else {
        let cut: String = s.chars().take(at).collect();
        format!("{cut}…")
    }
}

fn clean_inline(s: &str) -> String {
    s.replace('\t', " ").replace('\n', "⏎").replace('\r', "")
}

/// Render `table` to stdout per `opts`. Returns the number of rows emitted
/// (drives the process exit code: 0 rows → exit 1).
pub fn render(mut table: Table, opts: &OutOpts) -> usize {
    if opts.count {
        table = count_by(table, opts.by.as_deref());
    }

    if let Some(fields) = &opts.fields {
        table = select_fields(table, fields);
    }

    let limit = opts.limit.unwrap_or(match opts.format {
        Format::Table => DEFAULT_TABLE_LIMIT,
        _ => usize::MAX,
    });
    let total = table.rows.len();
    if table.rows.len() > limit {
        table.rows.truncate(limit);
    }

    let stdout = std::io::stdout();
    let mut w = stdout.lock();
    let emitted = table.rows.len();
    match opts.format {
        Format::Table => render_table(&table, opts.full, &mut w),
        Format::Tsv => render_tsv(&table, &mut w),
        Format::Json => render_json(&table, false, &mut w),
        Format::Jsonl => render_json(&table, true, &mut w),
    }
    if emitted < total && opts.format == Format::Table {
        eprintln!(
            "ccq: showing {emitted} of {total} rows (raise with --limit, or use -f tsv/json)"
        );
    }
    emitted
}

fn count_by(table: Table, by: Option<&str>) -> Table {
    let key_idx = match by {
        Some(field) => match table.headers.iter().position(|h| *h == field) {
            Some(i) => i,
            None => {
                eprintln!(
                    "ccq: --by field '{field}' not in field set [{}]",
                    table.headers.join(", ")
                );
                std::process::exit(2);
            }
        },
        None => 0,
    };
    let header: &'static str = table.headers[key_idx];
    let mut counts: HashMap<String, usize> = HashMap::new();
    for row in &table.rows {
        *counts.entry(row[key_idx].clone()).or_insert(0) += 1;
    }
    let mut rows: Vec<(String, usize)> = counts.into_iter().collect();
    rows.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    Table {
        headers: vec![header, "count"],
        rows: rows
            .into_iter()
            .map(|(k, n)| vec![k, n.to_string()])
            .collect(),
    }
}

fn select_fields(table: Table, fields: &[String]) -> Table {
    let mut indices = Vec::new();
    let mut headers = Vec::new();
    for f in fields {
        match table.headers.iter().position(|h| h == f) {
            Some(i) => {
                indices.push(i);
                headers.push(table.headers[i]);
            }
            None => {
                eprintln!(
                    "ccq: unknown field '{f}' (available: {})",
                    table.headers.join(", ")
                );
                std::process::exit(2);
            }
        }
    }
    let rows = table
        .rows
        .into_iter()
        .map(|row| indices.iter().map(|&i| row[i].clone()).collect())
        .collect();
    Table { headers, rows }
}

fn render_table(table: &Table, full: bool, w: &mut impl Write) {
    let cells: Vec<Vec<String>> = table
        .rows
        .iter()
        .map(|row| {
            row.iter()
                .map(|c| {
                    let c = clean_inline(c);
                    if full {
                        c
                    } else {
                        truncate(&c, TRUNCATE_AT)
                    }
                })
                .collect()
        })
        .collect();
    let mut widths: Vec<usize> = table.headers.iter().map(|h| h.chars().count()).collect();
    for row in &cells {
        for (i, c) in row.iter().enumerate() {
            widths[i] = widths[i].max(c.chars().count());
        }
    }
    let header_line: Vec<String> = table
        .headers
        .iter()
        .enumerate()
        .map(|(i, h)| format!("{h:<w$}", w = widths[i]))
        .collect();
    let _ = writeln!(w, "{}", header_line.join("  ").trim_end());
    let _ = writeln!(
        w,
        "{}",
        widths
            .iter()
            .map(|&n| "-".repeat(n))
            .collect::<Vec<_>>()
            .join("  ")
    );
    for row in &cells {
        let line: Vec<String> = row
            .iter()
            .enumerate()
            .map(|(i, c)| {
                let pad = widths[i].saturating_sub(c.chars().count());
                format!("{c}{}", " ".repeat(pad))
            })
            .collect();
        let _ = writeln!(w, "{}", line.join("  ").trim_end());
    }
}

fn render_tsv(table: &Table, w: &mut impl Write) {
    let _ = writeln!(w, "{}", table.headers.join("\t"));
    for row in &table.rows {
        let cols: Vec<String> = row.iter().map(|c| clean_inline(c)).collect();
        let _ = writeln!(w, "{}", cols.join("\t"));
    }
}

fn render_json(table: &Table, lines: bool, w: &mut impl Write) {
    let objects: Vec<serde_json::Value> = table
        .rows
        .iter()
        .map(|row| {
            let map: serde_json::Map<String, serde_json::Value> = table
                .headers
                .iter()
                .zip(row)
                .map(|(h, c)| ((*h).to_string(), serde_json::Value::String(c.clone())))
                .collect();
            serde_json::Value::Object(map)
        })
        .collect();
    if lines {
        for obj in &objects {
            let _ = writeln!(w, "{obj}");
        }
    } else {
        let _ = writeln!(w, "{}", serde_json::Value::Array(objects));
    }
}

/// Human-readable byte size.
pub fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = bytes as f64;
    let mut unit = 0;
    while v >= 1024.0 && unit < UNITS.len() - 1 {
        v /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{v:.1} {}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_respects_char_boundaries() {
        let s = "é".repeat(400);
        let t = truncate(&s, 300);
        assert!(t.ends_with('…'));
        assert_eq!(t.chars().count(), 301);
    }

    #[test]
    fn human_sizes() {
        assert_eq!(human_size(512), "512 B");
        assert_eq!(human_size(2048), "2.0 KB");
    }
}
