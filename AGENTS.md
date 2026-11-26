# Skills to load at the start of every session

Load these before the first task, every session, without being asked:

- **`context-budget`**: keeps the working context inside its budget (what to read, what to summarize, when to
  stop and delegate). Load first, it governs how the rest of the session reads files.
- **`caveman`**: the full caveman style for prose this repo produces (commit messages, plan text, summaries).
  Use the complete style, not a light version.
- **`ponytail`**: the review and debt discipline for changes and deferrals.

- For Rust projects, use `justrust` instead of calling cargo directly: `justrust check`, `justrust test`, `justrust build`, `justrust clippy`, and `justrust run` take exactly cargo's arguments (`-p`, `--lib`, test filters, `-- --nocapture`, `--release`, ...) and return cargo's exit code.
- Browser testing: prefer the obscura MCP (mcp__obscura__browser_*) for all browser testing, page inspection, screenshots, and interaction. Fall back to the playwright MCP (mcp__playwright__browser_*) only when obscura is unavailable, errors, or lacks the needed capability (e.g. drag/drop, file upload, dialog handling, media emulation, resize); say why when falling back.
- Code and file search: (1) Try ygrep FIRST for all code and file searches, before Grep, Glob, or Task explore. (2) If ygrep returns no results, use agentgrep (load the agentgrep skill before the first search); never use rg, grep -rn, find, or ls -R for discovery. agentgrep commands: exact string/symbol/regex -> `agentgrep grep <query>` (--regex, --type rs, --glob 'src/**/*.ts'); topic or file by name -> `agentgrep find <terms...>` (--max-files N); file structure before reading -> `agentgrep outline <file>`; where X is defined/called/rendered/handled -> `agentgrep trace subject:<x> relation:<r>`. Workflow: unfamiliar question -> find, then outline, then read only the line range it names; known symbol -> grep, then read the enclosing symbol's range. Gotchas: dot-dirs are skipped by default, so add --hidden for .paul/, .claude/, .github/, and never report 'not found' before retrying with --hidden and a wider --path; no match still exits 0, so read the `matches: N` line, not $?; a budget error (30 s, 64 MiB, 100,000 matches) is a failure, not a partial result, so narrow --path, --glob or --type and rerun; never pipe agentgrep output through head or tail. (3) Fall back to rg / Glob only for: exact enumeration of every file matching a glob, multiline regex, per-file counts, context lines (-A/-B/-C), raw path:line:text for a pipeline, or when `agentgrep --version` fails (say so first).

<!-- CARL-MANAGED: Do not remove this section -->
## CARL Integration

Follow all rules in <carl-rules> blocks from system-reminders.
These are dynamically injected based on context and MUST be obeyed.
<!-- END CARL-MANAGED -->

## Hosted-mode checks

Run these when touching the server or the pages; every one can fail, so run it before claiming done.

- **One port:** HTTP and `/ws` on 8770. `curl http://127.0.0.1:8770/api/health` is the liveness probe.
- **Admin gate:** `/admin/*` answers 404 (never 403) to every signed-out or non-admin session.
- **Test-idp build has three guards:** `--features test-idp`, a debug build, and `VTV_TEST_IDP=1`.
  A release build can never serve `/__test-idp/*`.
- **Design gates run served, never `file://`:** `python .paul/runs/gate_all.py --check-receipts`
  (receipts in `.paul/ds-receipts/04-03/`). A page that fetches at load measures empty from `file://`.
- **Claim-bearing commands run bare:** a pipe reports its last stage's exit status, not the check's.

