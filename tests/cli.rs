//! End-to-end: real repositories built with git, scanned by the real binary.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use tempfile::TempDir;

/// A sandbox whose git ignores the user's global and system config, so hooks,
/// signing or identity rules on the machine running the tests don't apply.
struct Sandbox {
    dir: TempDir,
    global_config: PathBuf,
}

impl Sandbox {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let global_config = dir.path().join("gitconfig");
        fs::write(&global_config, "").unwrap();
        Sandbox { dir, global_config }
    }

    fn root(&self) -> &Path {
        self.dir.path()
    }

    fn command(&self, program: &str) -> Command {
        let mut cmd = Command::new(program);
        cmd.env("GIT_CONFIG_GLOBAL", &self.global_config)
            .env("GIT_CONFIG_NOSYSTEM", "1");
        cmd
    }

    fn git(&self, repo: &Path, args: &[&str]) {
        let status = self
            .command("git")
            .arg("-C")
            .arg(repo)
            .args(args)
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?} failed");
    }

    fn repo(&self, relative: &str) -> PathBuf {
        let path = self.root().join(relative);
        fs::create_dir_all(&path).unwrap();
        self.git(&path, &["init", "-q", "-b", "main"]);
        self.git(&path, &["config", "user.name", "Ada Lovelace"]);
        self.git(&path, &["config", "user.email", "ada@example.com"]);
        path
    }

    /// Commits `file` as `author` ("Name <email>") on `date`, at noon UTC so
    /// the day is the same in any time zone the tests run in.
    fn commit(&self, repo: &Path, file: &str, message: &str, author: &str, date: &str) {
        fs::write(repo.join(file), message).unwrap();
        self.git(repo, &["add", file]);
        let when = format!("{date}T12:00:00Z");
        let status = self
            .command("git")
            .arg("-C")
            .arg(repo)
            .args(["commit", "-q", "-m", message, "--author", author])
            .env("GIT_AUTHOR_DATE", &when)
            .env("GIT_COMMITTER_DATE", &when)
            .status()
            .unwrap();
        assert!(status.success());
    }

    fn didwhat(&self, args: &[&str]) -> String {
        let output = self
            .command(env!("CARGO_BIN_EXE_didwhat"))
            .current_dir(self.root())
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "didwhat failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap()
    }
}

const ADA: &str = "Ada Lovelace <ada@example.com>";
const BOB: &str = "Bob <bob@example.com>";

fn sample() -> Sandbox {
    let sandbox = Sandbox::new();

    // In date order: `git log --since` stops walking at the first commit older
    // than the cutoff, so history must not go back in time.
    let api = sandbox.repo("api");
    sandbox.commit(
        &api,
        "old.txt",
        "chore: before the period",
        ADA,
        "2025-12-01",
    );
    sandbox.commit(&api, "a.txt", "feat: parse invoices", ADA, "2026-03-02");
    sandbox.commit(&api, "b.txt", "fix: someone else's work", BOB, "2026-03-02");
    sandbox.commit(&api, "c.txt", "feat: invoice history", ADA, "2026-03-03");
    fs::write(api.join("draft.md"), "wip").unwrap();

    let web = sandbox.repo("clients/web");
    sandbox.commit(&web, "index.html", "init", BOB, "2026-03-01");
    sandbox.git(&web, &["checkout", "-q", "-b", "feature"]);
    sandbox.commit(&web, "page.html", "feat: landing page", BOB, "2026-03-03");
    sandbox.git(&web, &["checkout", "-q", "main"]);
    let status = sandbox
        .command("git")
        .arg("-C")
        .arg(&web)
        .args([
            "merge",
            "-q",
            "--no-ff",
            "feature",
            "-m",
            "Merge branch 'feature'",
        ])
        .env("GIT_AUTHOR_NAME", "Bob")
        .env("GIT_AUTHOR_EMAIL", "bob@example.com")
        .env("GIT_AUTHOR_DATE", "2026-03-03T12:00:00Z")
        .env("GIT_COMMITTER_DATE", "2026-03-03T12:00:00Z")
        .status()
        .unwrap();
    assert!(status.success());

    sandbox
}

#[test]
fn markdown_report_groups_your_commits_by_day() {
    let sandbox = sample();
    let md = sandbox.didwhat(&["--since", "2026-01-01", "--merges"]);

    let at = |s: &str| {
        md.find(s)
            .unwrap_or_else(|| panic!("missing {s:?} in\n{md}"))
    };
    assert!(at("## 2026-03-03") < at("## 2026-03-02"));
    assert!(at("feat: invoice history") < at("feat: parse invoices"));
    assert!(md.contains("### api"));
    assert!(md.contains("merged by Bob: Merge branch 'feature'"));
    assert!(md.contains("### clients/web"));
    assert!(md.contains("- `??` draft.md"));
    assert!(md.contains("- Repositories: 2 scanned, 2 with activity"));

    assert!(
        !md.contains("someone else's work"),
        "only your commits are listed"
    );
    assert!(
        !md.contains("feat: landing page"),
        "only your commits are listed"
    );
    assert!(!md.contains("before the period"));
}

#[test]
fn author_flag_overrides_the_repo_identity() {
    let sandbox = sample();
    let md = sandbox.didwhat(&["--since", "2026-01-01", "--author", "bob@example.com"]);
    assert!(md.contains("fix: someone else's work"));
    assert!(md.contains("feat: landing page"));
    assert!(!md.contains("feat: parse invoices"));
}

#[test]
fn json_report_carries_every_field() {
    let sandbox = sample();
    let out = sandbox.didwhat(&["--since", "2026-01-01", "--json"]);
    let report: serde_json::Value = serde_json::from_str(&out).unwrap();

    let repos = report["repos"].as_array().unwrap();
    assert_eq!(repos.len(), 2);
    let api = repos.iter().find(|r| r["name"] == "api").unwrap();
    assert_eq!(
        api["authors"],
        serde_json::json!(["Ada Lovelace", "ada@example.com"])
    );
    assert_eq!(api["commits"].as_array().unwrap().len(), 2);
    assert_eq!(api["commits"][0]["date"], "2026-03-03");
    assert_eq!(api["commits"][0]["files"], serde_json::json!(["c.txt"]));
    assert_eq!(api["uncommitted"], serde_json::json!(["?? draft.md"]));
}

#[test]
fn writes_the_report_to_a_file() {
    let sandbox = sample();
    let stdout = sandbox.didwhat(&["--since", "2026-01-01", "--output", "report.md"]);
    assert!(stdout.is_empty());
    let written = fs::read_to_string(sandbox.root().join("report.md")).unwrap();
    assert!(written.starts_with("# didwhat"));
}
