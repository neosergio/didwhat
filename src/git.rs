use std::path::Path;
use std::process::Command;

use serde::Serialize;

const FIELD: char = '\x1f';
const RECORD: char = '\x1e';
const FORMAT: &str = "--format=%x1e%H%x1f%h%x1f%ad%x1f%at%x1f%an%x1f%ae%x1f%D%x1f%P%x1f%s";
/// Author date in the local time zone of the machine running the report, so a
/// commit made at 23:30 lands on the day it was actually made.
const DATE: &str = "--date=format-local:%Y-%m-%d %H:%M %z";

#[derive(Debug, Clone, Default, Serialize)]
pub struct Commit {
    pub hash: String,
    pub short: String,
    pub date: String,
    pub time: String,
    pub utc_offset: String,
    /// Author date as a Unix timestamp.
    pub timestamp: i64,
    pub author_name: String,
    pub author_email: String,
    pub refs: Vec<String>,
    pub subject: String,
    pub is_merge: bool,
    pub files: Vec<String>,
    pub files_total: usize,
}

/// What to ask `git log` for.
pub struct LogQuery<'a> {
    pub since: &'a str,
    pub until: Option<&'a str>,
    /// Author patterns, matched by git against name and email. Empty means
    /// every author.
    pub authors: &'a [String],
    pub merges_only: bool,
    pub max_files: usize,
}

/// Runs git in `repo` and returns its stdout, or its stderr as the error.
pub fn run(repo: &Path, args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        // Never stop to ask for credentials: a fetch that needs them fails
        // and is reported, instead of hanging the whole scan.
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .map_err(|e| format!("could not run git: {e}"))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}

pub fn is_available() -> bool {
    Command::new("git")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// The `user.name` and `user.email` git uses in `repo`, whichever are set.
pub fn identity(repo: &Path) -> Vec<String> {
    ["user.name", "user.email"]
        .iter()
        .filter_map(|key| run(repo, &["config", key]).ok())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect()
}

/// Escapes a literal so git's `--author` (a basic regular expression) matches
/// it exactly: a `.` in an email should not match any character.
pub fn escape_pattern(literal: &str) -> String {
    let mut out = String::with_capacity(literal.len());
    for c in literal.chars() {
        if matches!(c, '.' | '[' | ']' | '*' | '^' | '$' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

pub fn log(repo: &Path, query: &LogQuery) -> Result<Vec<Commit>, String> {
    let mut args: Vec<String> = vec![
        "-c".into(),
        "core.quotepath=false".into(),
        "log".into(),
        // Every branch, remote and tag, but not the stash.
        "--branches".into(),
        "--remotes".into(),
        "--tags".into(),
        "--date-order".into(),
        format!("--since={}", query.since),
        DATE.into(),
        FORMAT.into(),
    ];
    if let Some(until) = query.until {
        args.push(format!("--until={until}"));
    }
    if !query.authors.is_empty() {
        args.push("--regexp-ignore-case".into());
        args.extend(query.authors.iter().map(|a| format!("--author={a}")));
    }
    if query.merges_only {
        args.push("--merges".into());
    } else {
        args.push("--name-only".into());
    }
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let out = run(repo, &args)?;
    Ok(parse_log(&out, query.max_files))
}

pub fn parse_log(out: &str, max_files: usize) -> Vec<Commit> {
    out.split(RECORD)
        .filter(|record| !record.trim().is_empty())
        .filter_map(|record| parse_record(record, max_files))
        .collect()
}

fn parse_record(record: &str, max_files: usize) -> Option<Commit> {
    let mut lines = record.lines();
    let header = lines.next()?;
    let fields: Vec<&str> = header.split(FIELD).collect();
    if fields.len() < 9 {
        return None;
    }
    let mut when = fields[2].split_whitespace();
    let files: Vec<String> = lines
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(String::from)
        .collect();
    Some(Commit {
        hash: fields[0].to_string(),
        short: fields[1].to_string(),
        date: when.next().unwrap_or_default().to_string(),
        time: when.next().unwrap_or_default().to_string(),
        utc_offset: when.next().unwrap_or_default().to_string(),
        timestamp: fields[3].parse().ok()?,
        author_name: fields[4].to_string(),
        author_email: fields[5].to_string(),
        refs: fields[6]
            .split(", ")
            .filter(|r| !r.is_empty())
            .map(String::from)
            .collect(),
        is_merge: fields[7].split_whitespace().count() > 1,
        subject: fields[8..].join(&FIELD.to_string()),
        files_total: files.len(),
        files: files.into_iter().take(max_files).collect(),
    })
}

/// Whether a merge only brought another branch in (`Merge branch 'main' into
/// x`), as opposed to landing work, like merging a pull request.
pub fn is_sync_merge(commit: &Commit) -> bool {
    commit.is_merge
        && (commit.subject.starts_with("Merge branch ")
            || commit.subject.starts_with("Merge remote-tracking branch "))
}

/// A date with no time, like `2026-10-13`, made explicit: git would otherwise
/// read it as that day at the current time of day, and drop the morning.
pub fn whole_day(date: &str, end_of_day: bool) -> String {
    let is_date_only = date.len() == 10
        && date.char_indices().all(|(i, c)| {
            if i == 4 || i == 7 {
                c == '-'
            } else {
                c.is_ascii_digit()
            }
        });
    match (is_date_only, end_of_day) {
        (true, false) => format!("{date} 00:00:00"),
        (true, true) => format!("{date} 23:59:59"),
        (false, _) => date.to_string(),
    }
}

/// The period as Unix timestamps, resolved by git itself so every date format
/// git accepts works the same here: `(since, until)`.
pub fn window(repo: &Path, since: &str, until: Option<&str>) -> Result<(i64, Option<i64>), String> {
    let since_arg = format!("--since={since}");
    let until_arg = until.map(|u| format!("--until={u}"));
    let mut args = vec!["rev-parse", since_arg.as_str()];
    if let Some(u) = &until_arg {
        args.push(u.as_str());
    }
    let out = run(repo, &args)?;
    let value = |prefix: &str| {
        out.lines()
            .find_map(|line| line.strip_prefix(prefix))
            .and_then(|v| v.trim().parse::<i64>().ok())
    };
    let start =
        value("--max-age=").ok_or_else(|| format!("git did not understand --since {since:?}"))?;
    let end = match until {
        Some(u) => Some(
            value("--min-age=").ok_or_else(|| format!("git did not understand --until {u:?}"))?,
        ),
        None => None,
    };
    Ok((start, end))
}

/// Drops commits authored outside the period and the copies a rebase or
/// cherry-pick leaves behind.
///
/// `git log --since` filters by commit date, so a branch rebased today brings
/// back work authored weeks ago; that work belongs to its own day, outside
/// this report. The copies share author date and subject; the one that still
/// has a branch or tag on it is kept.
pub fn in_period(commits: Vec<Commit>, since: i64, until: Option<i64>) -> Vec<Commit> {
    let mut kept: Vec<Commit> = Vec::new();
    for commit in commits {
        if commit.timestamp < since || until.is_some_and(|u| commit.timestamp > u) {
            continue;
        }
        let copy = kept.iter_mut().find(|k| {
            k.timestamp == commit.timestamp
                && k.subject == commit.subject
                && k.author_email == commit.author_email
        });
        match copy {
            Some(existing) if existing.refs.is_empty() && !commit.refs.is_empty() => {
                *existing = commit
            }
            Some(_) => {}
            None => kept.push(commit),
        }
    }
    kept
}

/// `git status` for `repo`: the branch line (`main...origin/main [ahead 1]`)
/// and one line per uncommitted change.
pub fn status(repo: &Path) -> Result<(Option<String>, Vec<String>), String> {
    let out = run(
        repo,
        &[
            "-c",
            "core.quotepath=false",
            "status",
            "--porcelain=v1",
            "-b",
        ],
    )?;
    let mut branch = None;
    let mut changes = Vec::new();
    for line in out.lines() {
        if let Some(rest) = line.strip_prefix("## ") {
            branch = Some(rest.to_string());
        } else if !line.trim().is_empty() {
            changes.push(line.to_string());
        }
    }
    Ok((branch, changes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_commits_with_files() {
        let out = "\x1eaaa111\x1faaa\x1f2026-10-14 19:14 +0200\x1f1791997000\x1fAda\x1fada@example.com\x1fHEAD -> main, origin/main\x1fp1\x1ffeat: one\n\nsrc/a.rs\nsrc/b.rs\nsrc/c.rs\n\
                   \x1ebbb222\x1fbbb\x1f2026-10-13 09:05 +0200\x1f1791900000\x1fAda\x1fada@example.com\x1f\x1fp1 p2\x1fMerge branch 'x'\n";
        let commits = parse_log(out, 2);
        assert_eq!(commits.len(), 2);

        let first = &commits[0];
        assert_eq!(first.short, "aaa");
        assert_eq!(first.date, "2026-10-14");
        assert_eq!(first.time, "19:14");
        assert_eq!(first.utc_offset, "+0200");
        assert_eq!(first.refs, ["HEAD -> main", "origin/main"]);
        assert_eq!(first.subject, "feat: one");
        assert!(!first.is_merge);
        assert_eq!(first.files, ["src/a.rs", "src/b.rs"]);
        assert_eq!(first.files_total, 3);

        let merge = &commits[1];
        assert!(merge.is_merge);
        assert!(merge.refs.is_empty());
        assert!(merge.files.is_empty());
    }

    #[test]
    fn ignores_malformed_records() {
        assert!(parse_log("\x1enot a commit\n", 8).is_empty());
        assert!(parse_log("", 8).is_empty());
    }

    #[test]
    fn escapes_regex_metacharacters() {
        assert_eq!(escape_pattern("ada.l+x@ex.com"), r"ada\.l+x@ex\.com");
        assert_eq!(escape_pattern("[bot]"), r"\[bot\]");
    }

    fn at(timestamp: i64, subject: &str, refs: &[&str]) -> Commit {
        Commit {
            timestamp,
            subject: subject.into(),
            refs: refs.iter().map(|r| r.to_string()).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn in_period_drops_rebased_old_work_and_duplicate_copies() {
        let commits = vec![
            at(200, "feat: x", &[]),
            at(200, "feat: x", &["topic"]),
            at(200, "feat: x", &[]),
            at(150, "authored before the period", &["main"]),
            at(400, "authored after the period", &[]),
            at(300, "feat: y", &[]),
        ];
        let kept = in_period(commits, 180, Some(350));
        let summary: Vec<(&str, usize)> = kept
            .iter()
            .map(|c| (c.subject.as_str(), c.refs.len()))
            .collect();
        assert_eq!(summary, [("feat: x", 1), ("feat: y", 0)]);
    }

    #[test]
    fn whole_day_pins_date_only_values_to_midnight() {
        assert_eq!(whole_day("2026-10-13", false), "2026-10-13 00:00:00");
        assert_eq!(whole_day("2026-10-13", true), "2026-10-13 23:59:59");
        assert_eq!(whole_day("2 weeks ago", false), "2 weeks ago");
        assert_eq!(whole_day("2026-10-13 09:00", false), "2026-10-13 09:00");
    }

    #[test]
    fn sync_merges_are_told_apart_from_pull_requests() {
        let merge = |subject: &str| Commit {
            is_merge: true,
            subject: subject.into(),
            ..Default::default()
        };
        assert!(is_sync_merge(&merge("Merge branch 'master' into topic")));
        assert!(is_sync_merge(&merge(
            "Merge remote-tracking branch 'origin/master' into topic"
        )));
        assert!(!is_sync_merge(&merge(
            "Merge pull request #5534 from org/topic"
        )));
    }
}
