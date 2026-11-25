# Skills to load at the start of every session

Load these before the first task, every session, without being asked:

- **`context-budget`**: keeps the working context inside its budget (what to read, what to summarize, when to
  stop and delegate). Load first, it governs how the rest of the session reads files.
- **`caveman`**: the full caveman style for prose this repo produces (commit messages, plan text, summaries).
  Use the complete style, not a light version.
- **`ponytail`**: the review and debt discipline for changes and deferrals.

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
