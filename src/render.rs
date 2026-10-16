use std::collections::BTreeMap;
use std::fmt::Write;

use crate::git::Commit;
use crate::report::Report;

/// Uncommitted changes listed per repository before the rest is summarized.
const MAX_CHANGES: usize = 20;

/// One day of the report: repository name -> [(commit, is it yours)].
type Day<'a> = BTreeMap<&'a str, Vec<(&'a Commit, bool)>>;

pub fn json(report: &Report) -> String {
    serde_json::to_string_pretty(report).expect("a report always serializes")
}

/// The report as Markdown: newest day first, then each repository that had
/// activity that day, then that day's commits in the order they were made.
pub fn markdown(report: &Report) -> String {
    let mut out = String::new();
    header(&mut out, report);

    let mut days: BTreeMap<&str, Day> = BTreeMap::new();
    for repo in &report.repos {
        let entries = repo
            .commits
            .iter()
            .map(|c| (c, true))
            .chain(repo.merges.iter().map(|c| (c, false)));
        for (commit, mine) in entries {
            days.entry(commit.date.as_str())
                .or_default()
                .entry(repo.name.as_str())
                .or_default()
                .push((commit, mine));
        }
    }

    if days.is_empty() {
        out.push_str("No commits in this period.\n\n");
    }
    for (date, repos) in days.iter().rev() {
        let _ = writeln!(out, "## {date}\n");
        for (name, entries) in repos {
            let _ = writeln!(out, "### {name}\n");
            let mut entries = entries.clone();
            entries.sort_by(|a, b| a.0.time.cmp(&b.0.time));
            for (commit, mine) in entries {
                commit_item(&mut out, commit, mine);
            }
            out.push('\n');
        }
    }

    pending(&mut out, report);
    quiet(&mut out, report);
    warnings(&mut out, report);
    while out.ends_with("\n\n") {
        out.pop();
    }
    out
}

fn header(out: &mut String, report: &Report) {
    out.push_str("# didwhat\n\n");
    let _ = writeln!(out, "- Since: {}", report.since);
    if let Some(until) = &report.until {
        let _ = writeln!(out, "- Until: {until}");
    }
    if report.authors.is_empty() {
        out.push_str("- Authors: each repository's git identity (user.name, user.email)\n");
    } else {
        let _ = writeln!(out, "- Authors: {}", report.authors.join(", "));
    }
    let active = report.repos.iter().filter(|r| r.has_activity()).count();
    let _ = writeln!(
        out,
        "- Repositories: {} scanned, {} with activity",
        report.repos.len(),
        active
    );
    out.push_str("- Times are local to the machine that ran the report\n\n");
}

fn commit_item(out: &mut String, commit: &Commit, mine: bool) {
    let _ = write!(out, "- {} `{}` ", commit.time, commit.short);
    if !mine {
        let _ = write!(out, "merged by {}: ", commit.author_name);
    }
    out.push_str(&commit.subject);
    let branches = branch_names(&commit.refs);
    if !branches.is_empty() {
        let _ = write!(out, " — *{}*", branches.join(", "));
    }
    out.push('\n');
    for file in &commit.files {
        let _ = writeln!(out, "  - {file}");
    }
    if commit.files_total > commit.files.len() {
        let _ = writeln!(
            out,
            "  - … and {} more",
            commit.files_total - commit.files.len()
        );
    }
}

/// Branch names from `%D`, without tags, `HEAD`, or a remote copy of a branch
/// that is already listed locally.
fn branch_names(refs: &[String]) -> Vec<String> {
    let names: Vec<&str> = refs
        .iter()
        .map(|r| r.strip_prefix("HEAD -> ").unwrap_or(r))
        .filter(|r| !r.starts_with("tag: ") && *r != "HEAD" && !r.ends_with("/HEAD"))
        .collect();
    names
        .iter()
        .filter(|r| {
            r.split_once('/')
                .is_none_or(|(_, local)| !names.contains(&local))
        })
        .map(|r| r.to_string())
        .collect()
}

fn pending(out: &mut String, report: &Report) {
    let repos: Vec<_> = report
        .repos
        .iter()
        .filter(|r| !r.uncommitted.is_empty() || r.is_ahead())
        .collect();
    if repos.is_empty() {
        return;
    }
    out.push_str("## Not pushed or not committed\n\n");
    for repo in repos {
        let _ = write!(out, "### {}", repo.name);
        if let Some(branch) = &repo.branch {
            let _ = write!(out, " — `{branch}`");
        }
        out.push_str("\n\n");
        for change in repo.uncommitted.iter().take(MAX_CHANGES) {
            let (code, path) = change.split_at(change.len().min(2));
            let _ = writeln!(out, "- `{}` {}", code.trim(), path.trim());
        }
        if repo.uncommitted.len() > MAX_CHANGES {
            let _ = writeln!(out, "- … and {} more", repo.uncommitted.len() - MAX_CHANGES);
        }
        out.push('\n');
    }
}

fn quiet(out: &mut String, report: &Report) {
    let names: Vec<&str> = report
        .repos
        .iter()
        .filter(|r| !r.has_activity())
        .map(|r| r.name.as_str())
        .collect();
    if !names.is_empty() {
        let _ = writeln!(out, "## No activity\n\n{}\n", names.join(", "));
    }
}

fn warnings(out: &mut String, report: &Report) {
    let lines: Vec<String> = report
        .repos
        .iter()
        .flat_map(|r| r.warnings.iter().map(move |w| format!("- {}: {w}", r.name)))
        .collect();
    if !lines.is_empty() {
        let _ = writeln!(out, "## Warnings\n\n{}\n", lines.join("\n"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::RepoReport;

    fn commit(date: &str, time: &str, subject: &str) -> Commit {
        Commit {
            short: "abc1234".into(),
            date: date.into(),
            time: time.into(),
            author_name: "Bob".into(),
            subject: subject.into(),
            ..Default::default()
        }
    }

    #[test]
    fn groups_by_day_newest_first_and_orders_each_day_by_time() {
        let report = Report {
            since: "2026-10-01".into(),
            until: None,
            authors: vec![],
            repos: vec![RepoReport {
                name: "api".into(),
                commits: vec![
                    commit("2026-10-02", "18:00", "late"),
                    commit("2026-10-02", "09:00", "early"),
                    commit("2026-10-01", "12:00", "first day"),
                ],
                merges: vec![commit("2026-10-02", "12:00", "Merge pull request #1")],
                ..Default::default()
            }],
        };
        let md = markdown(&report);
        let at = |s: &str| {
            md.find(s)
                .unwrap_or_else(|| panic!("missing {s:?} in\n{md}"))
        };
        assert!(at("## 2026-10-02") < at("## 2026-10-01"));
        assert!(at("early") < at("merged by Bob: Merge pull request #1"));
        assert!(at("Merge pull request #1") < at("late"));
    }

    #[test]
    fn branch_names_drop_tags_head_and_duplicate_remotes() {
        let refs: Vec<String> = [
            "HEAD -> main",
            "origin/main",
            "origin/HEAD",
            "tag: v1.0",
            "origin/feature",
        ]
        .into_iter()
        .map(String::from)
        .collect();
        assert_eq!(branch_names(&refs), ["main", "origin/feature"]);
    }

    #[test]
    fn lists_unpushed_and_uncommitted_work_and_quiet_repos() {
        let report = Report {
            since: "1 week ago".into(),
            until: None,
            authors: vec!["ada@example.com".into()],
            repos: vec![
                RepoReport {
                    name: "web".into(),
                    branch: Some("main...origin/main [ahead 2]".into()),
                    uncommitted: vec![" M src/app.ts".into(), "?? notes.md".into()],
                    ..Default::default()
                },
                RepoReport {
                    name: "docs".into(),
                    ..Default::default()
                },
            ],
        };
        let md = markdown(&report);
        assert!(md.contains("- Authors: ada@example.com"));
        assert!(md.contains("### web — `main...origin/main [ahead 2]`"));
        assert!(md.contains("- `M` src/app.ts"));
        assert!(md.contains("- `??` notes.md"));
        assert!(md.contains("## No activity\n\nweb, docs"));
    }
}
