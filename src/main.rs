use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;
use std::thread;

use clap::Parser;

use didwhat::report::{self, Options, Report};
use didwhat::{discover, git, render};

/// What did I do? Your commits across every git repository under a folder,
/// grouped by day.
#[derive(Parser)]
#[command(version, about)]
struct Cli {
    /// Folders to scan for git repositories
    #[arg(default_value = ".")]
    paths: Vec<PathBuf>,

    /// Start of the period: anything git understands (2026-09-15, "2 weeks ago")
    #[arg(short, long, default_value = "7 days ago")]
    since: String,

    /// End of the period, same formats as --since
    #[arg(short, long)]
    until: Option<String>,

    /// Who counts as you: a name or email pattern, repeatable. Defaults to
    /// each repository's own user.name and user.email
    #[arg(short, long = "author", value_name = "PATTERN")]
    authors: Vec<String>,

    /// Run `git fetch --all` in every repository first
    #[arg(short, long)]
    fetch: bool,

    /// How many folder levels to search for repositories
    #[arg(long, default_value_t = 3)]
    depth: usize,

    /// Print JSON instead of Markdown
    #[arg(long)]
    json: bool,

    /// Write the report to this file instead of printing it
    #[arg(short, long, value_name = "FILE")]
    output: Option<PathBuf>,

    /// Files listed under each commit
    #[arg(long, default_value_t = 8, value_name = "N")]
    max_files: usize,

    /// Also list merges made by other people, and your own merges that only
    /// bring another branch in (`Merge branch 'main' into …`)
    #[arg(long)]
    merges: bool,

    /// Leave out uncommitted and unpushed work
    #[arg(long)]
    no_status: bool,
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    if !git::is_available() {
        eprintln!("didwhat: git was not found on PATH");
        return ExitCode::from(2);
    }

    let mut repos: Vec<(String, PathBuf)> = Vec::new();
    for root in &cli.paths {
        if !root.is_dir() {
            eprintln!("didwhat: {} is not a folder", root.display());
            return ExitCode::from(2);
        }
        for repo in discover::find_repos(root, cli.depth) {
            if !repos.iter().any(|(_, seen)| *seen == repo) {
                repos.push((discover::display_name(root, &repo), repo));
            }
        }
    }
    if repos.is_empty() {
        eprintln!(
            "didwhat: no git repositories found (searched {} levels deep)",
            cli.depth
        );
        return ExitCode::from(1);
    }

    let options = Options {
        since: git::whole_day(&cli.since, false),
        until: cli.until.as_deref().map(|u| git::whole_day(u, true)),
        authors: cli.authors.clone(),
        fetch: cli.fetch,
        merges: cli.merges,
        status: !cli.no_status,
        max_files: cli.max_files,
    };

    eprintln!(
        "didwhat: {} {} repositories…",
        if cli.fetch {
            "fetching and reading"
        } else {
            "reading"
        },
        repos.len()
    );
    let reports = thread::scope(|scope| {
        let handles: Vec<_> = repos
            .iter()
            .map(|(name, path)| {
                let options = &options;
                scope.spawn(move || report::collect(path, name, options))
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().expect("a repository scan panicked"))
            .collect()
    });

    let report = Report {
        since: cli.since,
        until: cli.until,
        authors: cli.authors,
        repos: reports,
    };
    let text = if cli.json {
        render::json(&report)
    } else {
        render::markdown(&report)
    };

    match cli.output {
        Some(path) => {
            if let Err(e) = fs::write(&path, format!("{text}\n")) {
                eprintln!("didwhat: could not write {}: {e}", path.display());
                return ExitCode::from(2);
            }
            eprintln!("didwhat: report written to {}", path.display());
        }
        None => println!("{text}"),
    }
    ExitCode::SUCCESS
}
