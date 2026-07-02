//! ccq — Claude Code Query: read-only queries over Claude Code's on-disk
//! session transcripts (`~/.claude/projects/*/`) and agent artifacts.

mod cmds;
mod model;
mod out;
mod scan;
mod timearg;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand};
use regex::Regex;

use cmds::inspect::GrepIn;
use out::{Format, OutOpts};
use scan::{Scope, Sidechain};

const VERSION: &str = env!("CCQ_VERSION");

#[derive(Parser)]
#[command(
    name = "ccq",
    version = VERSION,
    about = "Query Claude Code session transcripts (read-only)",
    after_help = "EXAMPLES:\n  \
    ccq bash --complex --since 30d -f tsv     # complex one-off scripts, last 30 days\n  \
    ccq bash -p myproj --count --by command   # most-run commands in a project\n  \
    ccq errors --count --by signature         # recurring failure signatures\n  \
    ccq writes --scratch --since 7d           # scratch-file writes this week\n  \
    ccq prompts -s 8717598f                   # what the user asked in a session\n  \
    ccq grep 'filters\\.json' --in tool-use    # sessions that touched a file\n  \
    ccq stats --since 2026-01-01              # corpus rollup"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Args, Clone)]
struct ScopeArgs {
    /// Substring match on project dir name; '.' = current directory's project. Repeatable.
    #[arg(short = 'p', long = "project")]
    project: Vec<String>,
    /// Session UUID prefix (>= 4 chars). Repeatable.
    #[arg(short = 's', long = "session")]
    session: Vec<String>,
    /// Only lines at/after this time (ISO date/datetime or 7d/24h/90m).
    #[arg(long)]
    since: Option<String>,
    /// Only lines at/before this time (ISO date/datetime or 7d/24h/90m).
    #[arg(long)]
    until: Option<String>,
    /// Subagent (sidechain) traffic handling.
    #[arg(long, value_enum, default_value = "include")]
    sidechain: Sidechain,
    /// Transcript root (default: $`CCQ_ROOT` or ~/.claude/projects).
    #[arg(long)]
    root: Option<PathBuf>,
}

#[derive(Args, Clone)]
struct OutArgs {
    /// Output format.
    #[arg(short = 'f', long = "format", value_enum, default_value = "table")]
    format: Format,
    /// Comma-separated column selection.
    #[arg(long, value_delimiter = ',')]
    fields: Option<Vec<String>>,
    /// Disable the default 300-char truncation of long fields.
    #[arg(long)]
    full: bool,
    /// Cap output rows (default: 200 for table, unlimited for tsv/json/jsonl).
    #[arg(long)]
    limit: Option<usize>,
    /// Aggregate into counts instead of listing rows.
    #[arg(long)]
    count: bool,
    /// Field to group by for --count (default: first column).
    #[arg(long)]
    by: Option<String>,
}

#[derive(Subcommand)]
enum Cmd {
    /// List projects with session count, size, and first/last activity
    Projects {
        #[command(flatten)]
        scope: ScopeArgs,
        #[command(flatten)]
        out: OutArgs,
    },
    /// List sessions with timing and message/tool counts
    Sessions {
        #[command(flatten)]
        scope: ScopeArgs,
        #[command(flatten)]
        out: OutArgs,
    },
    /// Every Bash `tool_use` (ts, session, description, command)
    Bash {
        #[command(flatten)]
        scope: ScopeArgs,
        #[command(flatten)]
        out: OutArgs,
        /// Regex filter on the command text.
        #[arg(long)]
        grep: Option<String>,
        /// Only complex commands (multi-line, heredoc, inline programs, loops).
        #[arg(long)]
        complex: bool,
    },
    /// Write/Edit/NotebookEdit `tool_use` (ts, session, tool, `file_path`, head)
    Writes {
        #[command(flatten)]
        scope: ScopeArgs,
        #[command(flatten)]
        out: OutArgs,
        /// Only paths under /tmp, /private/tmp, or *scratchpad*.
        #[arg(long)]
        scratch: bool,
        /// Regex filter on the file path.
        #[arg(long)]
        grep: Option<String>,
    },
    /// Tool-use frequency rollup (tool, count, projects, sessions)
    Tools {
        #[command(flatten)]
        scope: ScopeArgs,
        #[command(flatten)]
        out: OutArgs,
        /// Only tool uses whose result was an error.
        #[arg(long)]
        errors_only: bool,
    },
    /// Genuine human prompts (excludes tool results, command wrappers, reminders)
    Prompts {
        #[command(flatten)]
        scope: ScopeArgs,
        #[command(flatten)]
        out: OutArgs,
    },
    /// Failed tool results with normalized recurrence signatures
    Errors {
        #[command(flatten)]
        scope: ScopeArgs,
        #[command(flatten)]
        out: OutArgs,
    },
    /// Skill/slash-command invocations
    Slash {
        #[command(flatten)]
        scope: ScopeArgs,
        #[command(flatten)]
        out: OutArgs,
    },
    /// Subagent/workflow forensics over agent-*.jsonl, tasks/*.output, journal.jsonl
    Agents {
        /// A specific run/scratch directory (or artifact file) to inspect.
        dir: Option<PathBuf>,
        #[command(flatten)]
        scope: ScopeArgs,
        #[command(flatten)]
        out: OutArgs,
    },
    /// Full-text regex search with typed scoping
    Grep {
        /// Regex to search for.
        pattern: String,
        #[command(flatten)]
        scope: ScopeArgs,
        #[command(flatten)]
        out: OutArgs,
        /// Restrict to a content type.
        #[arg(long = "in", value_enum, default_value = "any")]
        r#in: GrepIn,
    },
    /// Pretty-print one event, a window, or a whole turn of a session
    Show {
        /// Session UUID prefix (>= 4 chars).
        prefix: String,
        #[command(flatten)]
        scope: ScopeArgs,
        /// Focus on this line number.
        #[arg(long)]
        line: Option<usize>,
        /// Lines of context around --line (default 5).
        #[arg(long, default_value = "5")]
        around: usize,
        /// Print the Nth user turn (prompt + responses + tool round-trips).
        #[arg(long)]
        turn: Option<usize>,
        /// Disable payload truncation.
        #[arg(long)]
        full: bool,
    },
    /// One-screen corpus rollup
    Stats {
        #[command(flatten)]
        scope: ScopeArgs,
        #[command(flatten)]
        out: OutArgs,
    },
}

fn resolve_root(arg: Option<PathBuf>) -> PathBuf {
    if let Some(r) = arg {
        return r;
    }
    if let Ok(env_root) = std::env::var("CCQ_ROOT") {
        if !env_root.is_empty() {
            return PathBuf::from(env_root);
        }
    }
    let home = std::env::var("HOME").unwrap_or_default();
    PathBuf::from(home).join(".claude").join("projects")
}

fn build_scope(args: &ScopeArgs) -> Result<Scope, String> {
    for s in &args.session {
        if s.len() < 4 {
            return Err(format!("session prefix '{s}' too short (need >= 4 chars)"));
        }
    }
    let now = chrono::Utc::now();
    let since =
        match &args.since {
            Some(v) => Some(timearg::parse_time_arg(v, now).ok_or_else(|| {
                format!("cannot parse --since '{v}' (use ISO date or 7d/24h/90m)")
            })?),
            None => None,
        };
    let until =
        match &args.until {
            Some(v) => Some(timearg::parse_time_arg(v, now).ok_or_else(|| {
                format!("cannot parse --until '{v}' (use ISO date or 7d/24h/90m)")
            })?),
            None => None,
        };
    let root = resolve_root(args.root.clone());
    if !root.is_dir() {
        return Err(format!("transcript root not found: {}", root.display()));
    }
    Ok(Scope {
        root,
        projects: args.project.clone(),
        sessions: args.session.clone(),
        since,
        until,
        sidechain: args.sidechain,
    })
}

fn build_out(args: &OutArgs, default_count_by: Option<&'static str>) -> OutOpts {
    let mut count = args.count;
    let mut by = args.by.clone();
    // Rollup-shaped commands (tools, slash) default to aggregation.
    if let Some(default_by) = default_count_by {
        if !count && args.fields.is_none() {
            count = true;
            by = by.or_else(|| Some(default_by.to_string()));
        }
    }
    if args.by.is_some() {
        count = true;
    }
    OutOpts {
        format: args.format,
        fields: args.fields.clone(),
        full: args.full,
        limit: args.limit,
        count,
        by,
    }
}

fn compile_regex(pattern: &str) -> Result<Regex, String> {
    Regex::new(pattern).map_err(|e| format!("invalid regex '{pattern}': {e}"))
}

fn run() -> Result<usize, String> {
    let cli = Cli::parse();
    let emitted = match cli.cmd {
        Cmd::Projects { scope, out } => {
            let scope = build_scope(&scope)?;
            out::render(cmds::listing::projects(&scope), &build_out(&out, None))
        }
        Cmd::Sessions { scope, out } => {
            let scope = build_scope(&scope)?;
            out::render(cmds::listing::sessions(&scope), &build_out(&out, None))
        }
        Cmd::Bash {
            scope,
            out,
            grep,
            complex,
        } => {
            let scope = build_scope(&scope)?;
            let re = grep.as_deref().map(compile_regex).transpose()?;
            out::render(
                cmds::listing::bash(&scope, re.as_ref(), complex),
                &build_out(&out, None),
            )
        }
        Cmd::Writes {
            scope,
            out,
            scratch,
            grep,
        } => {
            let scope = build_scope(&scope)?;
            let re = grep.as_deref().map(compile_regex).transpose()?;
            out::render(
                cmds::listing::writes(&scope, scratch, re.as_ref()),
                &build_out(&out, None),
            )
        }
        Cmd::Tools {
            scope,
            out,
            errors_only,
        } => {
            let scope = build_scope(&scope)?;
            // tools is already a rollup; plain render, no count default.
            out::render(
                cmds::listing::tools(&scope, errors_only),
                &build_out(&out, None),
            )
        }
        Cmd::Prompts { scope, out } => {
            let scope = build_scope(&scope)?;
            out::render(cmds::listing::prompts(&scope), &build_out(&out, None))
        }
        Cmd::Errors { scope, out } => {
            let scope = build_scope(&scope)?;
            out::render(cmds::listing::errors(&scope), &build_out(&out, None))
        }
        Cmd::Slash { scope, out } => {
            let scope = build_scope(&scope)?;
            out::render(
                cmds::listing::slash(&scope),
                &build_out(&out, Some("command")),
            )
        }
        Cmd::Agents { dir, scope, out } => {
            let scope = build_scope(&scope)?;
            out::render(
                cmds::agents::agents(&scope, dir.as_deref()),
                &build_out(&out, None),
            )
        }
        Cmd::Grep {
            pattern,
            scope,
            out,
            r#in,
        } => {
            let scope = build_scope(&scope)?;
            let re = compile_regex(&pattern)?;
            out::render(
                cmds::inspect::grep(&scope, &re, r#in),
                &build_out(&out, None),
            )
        }
        Cmd::Show {
            prefix,
            scope,
            line,
            around,
            turn,
            full,
        } => {
            if prefix.len() < 4 {
                return Err(format!(
                    "session prefix '{prefix}' too short (need >= 4 chars)"
                ));
            }
            let scope = build_scope(&scope)?;
            let sf = cmds::inspect::find_session(&scope, &prefix)?;
            cmds::inspect::show(&scope, &sf, line, around, turn, full)
                .map_err(|e| format!("{}: {e}", sf.path.display()))?
        }
        Cmd::Stats { scope, out } => {
            let scope = build_scope(&scope)?;
            out::render(cmds::stats::stats(&scope), &build_out(&out, None))
        }
    };
    Ok(emitted)
}

fn main() -> ExitCode {
    let result = run();
    scan::report_malformed();
    match result {
        Ok(0) => ExitCode::from(1),
        Ok(_) => ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("ccq: {msg}");
            ExitCode::from(2)
        }
    }
}

#[cfg(test)]
mod version_shape_tests {
    include!("version_shape.rs");

    #[test]
    fn tag_description_passes_through() {
        assert_eq!(shape_version(Some("v0.2.0"), "0.1.0"), "v0.2.0");
        assert_eq!(
            shape_version(Some("v0.2.0-4-gf40ddd4-dirty"), "0.1.0"),
            "v0.2.0-4-gf40ddd4-dirty"
        );
    }

    #[test]
    fn bare_hash_gets_pkg_prefix() {
        assert_eq!(shape_version(Some("f40ddd4"), "0.1.0"), "0.1.0+f40ddd4");
        assert_eq!(
            shape_version(Some("f40ddd4-dirty"), "0.1.0"),
            "0.1.0+f40ddd4-dirty"
        );
    }

    #[test]
    fn missing_git_falls_back() {
        assert_eq!(shape_version(None, "0.1.0"), "0.1.0");
    }
}
