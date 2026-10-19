use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

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
    /// The later commit that brought this one's changes into another branch,
    /// typically its pull request squashed or cherry-picked into main. On a
    /// branch of several commits it is set on the last one.
    pub landed: Option<Landed>,
    /// Every file, before `--max-files` cuts the list.
    #[serde(skip)]
    pub all_files: Vec<String>,
    #[serde(skip)]
    pub parents: Vec<String>,
}

/// A later commit with the same changes as a commit, or as a run of commits
/// ending in it.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Landed {
    pub hash: String,
    pub short: String,
    pub date: String,
    pub time: String,
    pub refs: Vec<String>,
    /// How many commits it squashed: 1 for a copy of a single commit.
    pub commits: usize,
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
        parents: fields[7].split_whitespace().map(String::from).collect(),
        subject: fields[8..].join(&FIELD.to_string()),
        files_total: files.len(),
        files: files.iter().take(max_files).cloned().collect(),
        all_files: files,
        landed: None,
    })
}

/// The `git patch-id --stable` of the changes from `base` to `tip`: the same
/// for any two ranges that make the same changes, whatever their messages,
/// dates or parents.
pub fn patch_id(repo: &Path, base: &str, tip: &str) -> Option<String> {
    let diff = run(repo, &["diff", "--no-color", "--no-ext-diff", base, tip]).ok()?;
    let mut child = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["patch-id", "--stable"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    child.stdin.take()?.write_all(diff.as_bytes()).ok()?;
    let output = child.wait_with_output().ok()?;
    let id = String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .next()?
        .to_string();
    Some(id)
}

/// The commits reachable from `hash` that were committed after `since`.
pub fn ancestors(repo: &Path, hash: &str, since: i64) -> HashSet<String> {
    let since = format!("--max-age={since}");
    run(repo, &["rev-list", &since, hash])
        .map(|out| out.lines().map(String::from).collect())
        .unwrap_or_default()
}

/// Folds each commit that landed later as a copy, like a pull request squashed
/// or cherry-picked into main, into the commits it copies.
///
/// Without this every change is listed twice: once when it was made on its
/// branch and once when it landed. A copy has a new author date, so
/// `in_period` keeps both.
///
/// The copied commits are a run that ends in some earlier commit and walks
/// back through its parents: one commit for a cherry-pick or the squash of a
/// one-commit branch, several for the squash of a longer one. A run matches
/// when its changes, from the parent of its first commit to its last, have
/// the same `patch_id(base, tip)` as the copy. Only runs that touch exactly the
/// copy's files are compared, and only runs that are not already in the
/// copy's own history, as `ancestors` says: on a linear history every earlier
/// commit is. That keeps most of them from ever reaching git. The commits of
/// the run stay listed, because each was work on its own day; the copy is
/// dropped and recorded as `landed` on the last one.
pub fn fold_landed(
    commits: Vec<Commit>,
    patch_id: impl Fn(&str, &str) -> Option<String>,
    ancestors: impl Fn(&str) -> HashSet<String>,
) -> Vec<Commit> {
    let index: HashMap<&str, usize> = commits
        .iter()
        .enumerate()
        .map(|(i, c)| (c.hash.as_str(), i))
        .collect();
    let mut ids: HashMap<(String, String), Option<String>> = HashMap::new();
    let mut id = |base: &str, tip: &str| {
        ids.entry((base.to_string(), tip.to_string()))
            .or_insert_with(|| patch_id(base, tip))
            .clone()
    };

    let mut by_age: Vec<usize> = (0..commits.len()).collect();
    by_age.sort_by_key(|&i| commits[i].timestamp);
    let foldable = |c: &Commit| !c.is_merge && c.parents.len() == 1 && !c.all_files.is_empty();

    let mut landed: HashMap<usize, Landed> = HashMap::new();
    let mut taken = vec![false; commits.len()];
    let mut folded = vec![false; commits.len()];
    for &copy in &by_age {
        let x = &commits[copy];
        if !foldable(x) || taken[copy] {
            continue;
        }
        let files: HashSet<&str> = x.all_files.iter().map(String::as_str).collect();
        let mut history: Option<HashSet<String>> = None;
        // The nearest earlier commits first: a copy lands after its work.
        let tips = by_age
            .iter()
            .rev()
            .copied()
            .filter(|&t| t != copy && commits[t].timestamp < x.timestamp);
        'tips: for tip in tips {
            let mut run = Vec::new();
            let mut touched: HashSet<&str> = HashSet::new();
            let mut at = tip;
            loop {
                let c = &commits[at];
                if at == copy
                    || !foldable(c)
                    || taken[at]
                    || folded[at]
                    || !c.all_files.iter().all(|f| files.contains(f.as_str()))
                {
                    break;
                }
                touched.extend(c.all_files.iter().map(String::as_str));
                run.push(at);
                let base = &c.parents[0];
                if touched.len() == files.len() {
                    let history = history.get_or_insert_with(|| ancestors(&x.hash));
                    if history.contains(&commits[tip].hash) {
                        continue 'tips;
                    }
                    let same = id(base, &commits[tip].hash);
                    if same.is_some() && same == id(&x.parents[0], &x.hash) {
                        landed.insert(
                            tip,
                            Landed {
                                hash: x.hash.clone(),
                                short: x.short.clone(),
                                date: x.date.clone(),
                                time: x.time.clone(),
                                refs: x.refs.clone(),
                                commits: run.len(),
                            },
                        );
                        for &i in &run {
                            taken[i] = true;
                        }
                        folded[copy] = true;
                        break 'tips;
                    }
                }
                match index.get(base.as_str()) {
                    Some(&parent) => at = parent,
                    None => break,
                }
            }
        }
    }

    commits
        .into_iter()
        .enumerate()
        .filter(|(i, _)| !folded[*i])
        .map(|(i, mut commit)| {
            commit.landed = landed.remove(&i);
            commit
        })
        .collect()
}

/// Whether a merge only brought another branch in (`Merge branch 'main' into
/// x`), as opposed to landing work, like merging a pull request.
pub fn is_sync_merge(commit: &Commit) -> bool {
    commit.is_merge
        && (commit.subject.starts_with("Merge branch ")
            || commit.subject.starts_with("Merge remote-tracking branch "))
}

/// Words that make a date more precise than a day, so it is left alone.
const SUB_DAY: &[&str] = &[
    "now", "noon", "midnight", "tea", "am", "pm", "hour", "hours", "minute", "minutes", "second",
    "seconds",
];

/// Fails when git did not understand `date`.
///
/// Git reads a date it cannot parse as the current time, without an error, so
/// `--since monday` (git wants `last monday`) would report nothing and not say
/// why. A date that resolves to now and does not ask for it, the way `now`,
/// `0 days ago` or `0 minutes ago` do, is taken as not understood.
pub fn check_date(repo: &Path, date: &str) -> Result<(), String> {
    let asks_for_now = date.split_whitespace().any(|word| word == "0");
    if asks_for_now || mentions_time(date) {
        return Ok(());
    }
    let (resolved, _) = window(repo, date, None)?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or_default();
    if (now - resolved).abs() <= 2 {
        return Err(format!(
            "git does not understand {date:?} as a date. Try one like 2026-09-15, \
             yesterday, \"last monday\" or \"2 weeks ago\""
        ));
    }
    Ok(())
}

/// Whether `date` is more precise than a day: it has a time of day, a
/// timestamp, or a word like `now` or `hours`.
fn mentions_time(date: &str) -> bool {
    let trimmed = date.trim();
    trimmed.contains(':')
        || trimmed.starts_with('@')
        || trimmed
            .split(|c: char| !c.is_ascii_alphanumeric())
            .any(|word| SUB_DAY.contains(&word.to_ascii_lowercase().as_str()))
}

/// A date with no time of day, like `2026-10-13`, `yesterday` or
/// `2 weeks ago`, made to cover whole days: from the start of that day, or up
/// to its end. Git would otherwise read it as that day at the current time of
/// day, which drops the morning, and makes `--since yesterday --until
/// yesterday` an empty instant.
pub fn whole_day(date: &str, end_of_day: bool) -> String {
    let trimmed = date.trim();
    if trimmed.is_empty() || mentions_time(trimmed) {
        return date.to_string();
    }
    let time = if end_of_day { "23:59:59" } else { "00:00:00" };
    format!("{trimmed} {time}")
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
        assert_eq!(whole_day("yesterday", false), "yesterday 00:00:00");
        assert_eq!(whole_day("yesterday", true), "yesterday 23:59:59");
        assert_eq!(whole_day("2 weeks ago", false), "2 weeks ago 00:00:00");
        assert_eq!(whole_day("2026-10-13 09:00", false), "2026-10-13 09:00");
        assert_eq!(whole_day("3 hours ago", false), "3 hours ago");
        assert_eq!(whole_day("now", true), "now");
        assert_eq!(whole_day("@1792000000", false), "@1792000000");
    }

    fn change(hash: &str, parent: &str, timestamp: i64, files: &[&str]) -> Commit {
        let files: Vec<String> = files.iter().map(|f| f.to_string()).collect();
        Commit {
            hash: hash.into(),
            short: hash.into(),
            timestamp,
            parents: vec![parent.into()],
            files_total: files.len(),
            files: files.clone(),
            all_files: files,
            ..Default::default()
        }
    }

    /// A fake `patch_id`: every range is different, except the pairs given.
    fn same_changes(pairs: &[(&str, &str, &str, &str)]) -> impl Fn(&str, &str) -> Option<String> {
        let pairs: Vec<(String, String)> = pairs
            .iter()
            .flat_map(|(b1, t1, b2, t2)| {
                let key = format!("{b1}..{t1}");
                [(key.clone(), key.clone()), (format!("{b2}..{t2}"), key)]
            })
            .collect();
        move |base, tip| {
            let range = format!("{base}..{tip}");
            let same = pairs.iter().find(|(r, _)| *r == range).map(|(_, id)| id);
            Some(same.cloned().unwrap_or(range))
        }
    }

    fn no_history(_: &str) -> HashSet<String> {
        HashSet::new()
    }

    fn hashes(commits: &[Commit]) -> Vec<&str> {
        commits.iter().map(|c| c.hash.as_str()).collect()
    }

    #[test]
    fn fold_landed_merges_a_copy_into_the_commit_it_copies() {
        // Newest first, as git log lists them. "b-landed" is "b" squashed into
        // main; "c", after "b" on its branch, touches the same file but makes a
        // different change.
        let commits = vec![
            change("b-landed", "main", 300, &["x.rs"]),
            change("c", "b", 250, &["x.rs"]),
            change("b", "a", 200, &["x.rs"]),
            change("a", "root", 100, &["y.rs"]),
        ];
        let kept = fold_landed(
            commits,
            same_changes(&[("a", "b", "main", "b-landed")]),
            no_history,
        );

        assert_eq!(hashes(&kept), ["c", "b", "a"]);
        let landed = kept[1].landed.as_ref().unwrap();
        assert_eq!((landed.hash.as_str(), landed.commits), ("b-landed", 1));
        assert!(kept[0].landed.is_none());
    }

    #[test]
    fn fold_landed_merges_a_squashed_branch_into_its_last_commit() {
        // "t1" and "t2" on a branch off "base", then squashed into main as "s".
        let commits = vec![
            change("s", "main", 300, &["x.rs", "y.rs"]),
            change("t2", "t1", 200, &["y.rs"]),
            change("t1", "base", 100, &["x.rs"]),
        ];
        let kept = fold_landed(
            commits,
            same_changes(&[("base", "t2", "main", "s")]),
            no_history,
        );

        assert_eq!(hashes(&kept), ["t2", "t1"]);
        let landed = kept[0].landed.as_ref().unwrap();
        assert_eq!((landed.hash.as_str(), landed.commits), ("s", 2));
        assert!(kept[1].landed.is_none());
    }

    #[test]
    fn fold_landed_keeps_a_commit_whose_changes_no_run_made() {
        let commits = vec![
            change("s", "main", 300, &["x.rs", "y.rs"]),
            change("t2", "t1", 200, &["y.rs"]),
            change("t1", "base", 100, &["x.rs"]),
        ];
        let kept = fold_landed(commits, same_changes(&[]), no_history);
        assert_eq!(hashes(&kept), ["s", "t2", "t1"]);
        assert!(kept.iter().all(|c| c.landed.is_none()));
    }

    #[test]
    fn fold_landed_skips_commits_already_in_the_copys_history() {
        // A revert of a revert brings back the same changes as "a", but "a"
        // is its ancestor: it was never a copy.
        let commits = vec![
            change("a-again", "revert", 300, &["x.rs"]),
            change("a", "root", 100, &["x.rs"]),
        ];
        let history = |_: &str| HashSet::from(["a".to_string(), "revert".to_string()]);
        let kept = fold_landed(
            commits,
            same_changes(&[("root", "a", "revert", "a-again")]),
            history,
        );
        assert_eq!(hashes(&kept), ["a-again", "a"]);
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
