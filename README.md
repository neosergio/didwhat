# didwhat

**What did I do?** Your commits across every git repository under a folder,
grouped by day, in one command.

```
didwhat --since 2026-09-15 ~/code/client
```

```markdown
## 2026-10-14

### risk-engine

- 10:12 `57a5e82` feat: record when an invoice enters and leaves scope — *past-due-history*
  - etl/scope.py
  - web/src/InvoiceHistory.tsx

### web

- 18:05 `e6c97e3` fix: keep the worklist view when coming back from an invoice — *main*

## Not pushed or not committed

### web — `main...origin/main [ahead 1]`

- `M` src/worklist.tsx
```

It exists for one situation: you work on several machines, some of them not
yours (a client's laptop, an employer's), and at the end of the week you need to
know what you actually did there — for a standup, a timesheet, or a worklog.

## What it reports

For every repository it finds below the folders you give it:

- **Your commits** on every branch, remote and tag, newest day first, with the
  branch they are on and the files they touched.
- **With `--merges`, merges by other people**, so you see what landed while
  you worked. Off by default: in a busy repository they drown out your own
  work.
- **Work that has not left the machine:** uncommitted changes and branches that
  are ahead of their remote.

Days and times are in the local time zone of the machine that runs it, so a
commit made at 23:30 is listed on the day it was made, not the UTC day.

A commit counts on the day it was **authored**. Rebasing a branch today does not
bring its old commits into today's report, and the copies a rebase leaves on
other branches are listed once. A date with no time, like `2026-09-15`, means
the whole day: plain git would read it as that day at the current time and miss
the morning.

## Who counts as you

By default, each repository's own `user.name` and `user.email`. That is usually
right even when a client repository uses a corporate identity, because it is
the identity you commit with there.

When it is not, pass one or more `--author` patterns. Git matches them against
both the name and the email:

```
didwhat --since "2 weeks ago" --author sergio@client.com --author "Sergio Infante"
```

## Examples

The last seven days, in every repository under the current folder:

```
didwhat
```

A closed period, saved to a file for a timesheet or a worklog. Dates without a
time cover the whole day at both ends:

```
didwhat --since 2026-09-01 --until 2026-09-30 ~/code/client -o september.md
```

Several folders in one report, for example two clients:

```
didwhat --since "2 weeks ago" ~/code/client-a ~/code/client-b
```

On Windows, from PowerShell, with the report written outside the repositories
so it does not show up as an untracked file in them:

```powershell
didwhat --since 2026-09-29 C:\code\client -o $HOME\Desktop\didwhat.md
```

On a laptop you have not used for a while, fetch first so commits you pushed
from another machine are in the report too:

```
didwhat --fetch --since 2026-10-01 ~/code/client
```

Only the commits, without the uncommitted and unpushed work:

```
didwhat --no-status
```

Repositories nested deeper than three folder levels:

```
didwhat --depth 5 ~/code
```

### With `--json`

The JSON has one entry per repository in `repos`, each with its `commits`
(`date`, `time`, `subject`, `refs`, `files`…), `merges`, `branch`,
`uncommitted` and `warnings`. With [jq](https://jqlang.org):

Commits per day:

```sh
didwhat --json --since 2026-10-01 ~/code \
  | jq -r '[.repos[].commits[]] | group_by(.date) | reverse[] | "\(.[0].date) \(length)"'
```

Commits per repository, leaving out the ones with none:

```sh
didwhat --json --since 2026-10-01 ~/code \
  | jq -r '.repos[] | select(.commits | length > 0) | "\(.name): \(.commits | length)"'
```

One line per commit, ready to paste into a standup:

```sh
didwhat --json --since 2026-10-15 ~/code \
  | jq -r '.repos[] | .name as $repo | .commits[] | "\(.date) [\($repo)] \(.subject)"'
```

Repositories with work that has not left the machine:

```sh
didwhat --json ~/code \
  | jq -r '.repos[] | select((.uncommitted | length > 0) or ((.branch // "") | test("ahead"))) | .name'
```

## Options

| Option | |
|---|---|
| `[PATHS]...` | Folders to scan. Default: the current folder |
| `-s, --since` | Start of the period, anything git understands (`2026-09-15`, `"2 weeks ago"`). Default: `"7 days ago"` |
| `-u, --until` | End of the period |
| `-a, --author` | Who counts as you; repeatable |
| `-f, --fetch` | `git fetch --all` every repository first. Never prompts for credentials: a fetch that needs them is reported as a warning |
| `--depth` | Folder levels to search for repositories. Default: 3 |
| `--json` | JSON instead of Markdown |
| `-o, --output` | Write to a file instead of printing |
| `--max-files` | Files listed per commit. Default: 8 |
| `--merges` | Also list other people's merges, and your own merges that only bring another branch in |
| `--no-status` | Leave out uncommitted and unpushed work |

## Requirements

Only `git` on the `PATH`. didwhat is a single binary with no runtime, which is
the point: it has to run on machines where you cannot install Python or Node.

## Install

Every release on GitHub ships prebuilt binaries for Windows, macOS (Apple
Silicon and Intel) and Linux (x86_64 and ARM), with one-line installers. No
Rust toolchain needed.

Windows (PowerShell):

```powershell
powershell -ExecutionPolicy Bypass -c "irm https://github.com/neosergio/didwhat/releases/latest/download/didwhat-installer.ps1 | iex"
```

macOS and Linux:

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/neosergio/didwhat/releases/latest/download/didwhat-installer.sh | sh
```

Or download the archive for your platform from the
[releases page](https://github.com/neosergio/didwhat/releases) and put the
binary anywhere on your `PATH`. That is the way to go on a locked-down machine
where running an installer script is not allowed.

From source, with a Rust toolchain:

```
cargo install --git https://github.com/neosergio/didwhat
```

## License

MIT
