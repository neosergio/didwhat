use std::collections::HashSet;
use std::path::Path;

use serde::Serialize;

use crate::git::{self, Commit, LogQuery};

#[derive(Debug, Serialize)]
pub struct Report {
    pub since: String,
    pub until: Option<String>,
    /// Authors passed with `--author`. Empty means each repository's own git
    /// identity was used; see `RepoReport::authors`.
    pub authors: Vec<String>,
    pub repos: Vec<RepoReport>,
}

#[derive(Debug, Default, Serialize)]
pub struct RepoReport {
    pub name: String,
    pub path: String,
    /// The authors that counted as "you" in this repository.
    pub authors: Vec<String>,
    pub commits: Vec<Commit>,
    /// Merges made by other people, which show what landed while you worked.
    /// Only collected with `--merges`.
    pub merges: Vec<Commit>,
    /// `git status` branch line, e.g. `main...origin/main [ahead 1]`.
    pub branch: Option<String>,
    pub uncommitted: Vec<String>,
    pub warnings: Vec<String>,
}

impl RepoReport {
    pub fn has_activity(&self) -> bool {
        !self.commits.is_empty() || !self.merges.is_empty()
    }

    pub fn is_ahead(&self) -> bool {
        self.branch.as_deref().is_some_and(|b| b.contains("[ahead"))
    }
}

pub struct Options {
    pub since: String,
    pub until: Option<String>,
    pub authors: Vec<String>,
    pub fetch: bool,
    pub merges: bool,
    pub status: bool,
    pub max_files: usize,
}

pub fn collect(path: &Path, name: &str, options: &Options) -> RepoReport {
    let mut report = RepoReport {
        name: name.to_string(),
        path: path.display().to_string(),
        ..Default::default()
    };

    if options.fetch {
        if let Err(e) = git::run(path, &["fetch", "--all", "--prune", "--quiet"]) {
            report.warnings.push(format!("fetch failed: {e}"));
        }
    }

    let patterns: Vec<String> = if options.authors.is_empty() {
        report.authors = git::identity(path);
        report
            .authors
            .iter()
            .map(|a| git::escape_pattern(a))
            .collect()
    } else {
        report.authors = options.authors.clone();
        options.authors.clone()
    };

    let (since, until) = match git::window(path, &options.since, options.until.as_deref()) {
        Ok(window) => window,
        Err(e) => {
            report.warnings.push(e);
            return report;
        }
    };

    if patterns.is_empty() {
        report
            .warnings
            .push("no git identity (user.name / user.email) here; pass --author".to_string());
    } else {
        match git::log(path, &query(options, &patterns, false)) {
            Ok(commits) => {
                let commits: Vec<Commit> = git::in_period(commits, since, until)
                    .into_iter()
                    .filter(|c| options.merges || !git::is_sync_merge(c))
                    .collect();
                report.commits = git::fold_landed(commits, |c| git::patch_id(path, &c.hash))
            }
            Err(e) => report.warnings.push(format!("git log failed: {e}")),
        }
    }

    if options.merges {
        let mine: HashSet<&str> = report.commits.iter().map(|c| c.hash.as_str()).collect();
        match git::log(path, &query(options, &[], true)) {
            Ok(merges) => {
                report.merges = git::in_period(merges, since, until)
                    .into_iter()
                    .filter(|m| !mine.contains(m.hash.as_str()))
                    .collect()
            }
            Err(e) => report
                .warnings
                .push(format!("git log (merges) failed: {e}")),
        }
    }

    if options.status {
        match git::status(path) {
            Ok((branch, changes)) => {
                report.branch = branch;
                report.uncommitted = changes;
            }
            Err(e) => report.warnings.push(format!("git status failed: {e}")),
        }
    }

    report
}

fn query<'a>(options: &'a Options, authors: &'a [String], merges_only: bool) -> LogQuery<'a> {
    LogQuery {
        since: &options.since,
        until: options.until.as_deref(),
        authors,
        merges_only,
        max_files: options.max_files,
    }
}
