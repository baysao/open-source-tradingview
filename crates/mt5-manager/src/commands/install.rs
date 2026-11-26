//! `install`: install or repair the whole host stack, idempotently.
//!
//! The sequence is `bridge/host-install.ps1`'s, restated in argv (that script's measured steps are
//! the contract; this module does not invent a different order):
//!
//! 1. `mise install mt5-bridge` under `MISE_ENV=mt5` (the pinned release asset, `mise.mt5.toml`)
//! 2. `mise install cloudflared`
//! 3. `bridge.env`: written once. An existing `BRIDGE_TOKEN` is **kept** — regenerating it would
//!    silently orphan every hub registration that holds the old one (AC-5).
//! 4. the named tunnel: reuse when it exists, create when it does not
//! 5. the two scheduled tasks: registered when absent, updated in place when present
//! 6. start both and poll `/health` with one bounded deadline
//!
//! Decision 4 of the phase architecture: install and repair are the same path, so a half-broken
//! host is repaired by running this again.
//!
//! What this module will not do: `cloudflared tunnel login` needs a browser and the owner's
//! Cloudflare account (decision 5). When the certificate is absent the run stops and says exactly
//! which command the owner must run — it never imitates a login it cannot perform.

use std::time::Duration;

use crate::http::Http;
use crate::runner::{CmdResult, CommandRunner, Fs};
use crate::secret::Secret;

use super::status::HealthAnswer;
use super::Failure;

/// The named tunnel the manager owns. `mt5` is the name the host script's tasks assume.
pub const TUNNEL_NAME: &str = "mt5";
/// The host installer's bridge port (`bridge/host-install.ps1:83`).
pub const DEFAULT_PORT: u16 = 8797;
/// How long the post-install /health poll waits: 15 tries, one second apart (host-install.ps1:129).
pub const HEALTH_ATTEMPTS: u32 = 15;
/// The MCP probe's own deadline. Short on purpose: a terminal that is up answers the handshake in
/// milliseconds, and a dead port should not stall the install panel.
pub const MCP_PROBE_TIMEOUT_S: u64 = 5;

/// Where everything lives. Defaults match the host installer's layout
/// (`C:\Users\Administrator\project\vtradingview`), and the UI may override the root.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstallPaths {
    pub deploy_root: String,
    pub port: u16,
}

impl Default for InstallPaths {
    fn default() -> Self {
        Self {
            deploy_root: r"C:\Users\Administrator\project\vtradingview".to_string(),
            port: DEFAULT_PORT,
        }
    }
}

impl InstallPaths {
    pub fn bridge_dir(&self) -> String {
        format!(r"{}\bridge", self.deploy_root)
    }

    pub fn env_file(&self) -> String {
        format!(r"{}\bridge.env", self.bridge_dir())
    }

    pub fn log_dir(&self) -> String {
        format!(r"{}\logs", self.deploy_root)
    }

    pub fn tunnel_config(&self) -> String {
        // cloudflared reads `%USERPROFILE%\.cloudflared\config.yml` by default; the manager owns a
        // copy in the deploy root so a repair rewrites one file it can see.
        format!(r"{}\cloudflared\config.yml", self.deploy_root)
    }

    /// Where the bridge binary looks for its env: beside its own exe, with no argument to redirect it
    /// (`crates/mt5-bridge/src/main.rs:141`, `dotenvy::from_path(dir.join("bridge.env"))`).
    ///
    /// Derived from the path `mise which mt5-bridge` printed, so a mise layout change moves this
    /// with it. An empty exe path is a programming error upstream, not a valid root.
    pub fn env_beside_exe(&self, bridge_exe: &str) -> String {
        let dir = bridge_exe
            .rfind(['\\', '/'])
            .map(|i| &bridge_exe[..i])
            .unwrap_or("");
        if dir.is_empty() {
            self.env_file()
        } else {
            format!(r"{dir}\bridge.env")
        }
    }
}

/// The per-step truth: what happened, so the panel shows a list rather than one green tick.
///
/// Serialised for the IPC boundary: `manager.js` renders one line per step, and `Skipped` carries the
/// reason a step did not run (AC-5's "names the cause").
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub enum StepOutcome {
    /// Done for the first time.
    Installed,
    /// Already there; nothing changed.
    Kept,
    /// Present and updated in place.
    Updated,
    /// Deliberately not run, with the reason.
    Skipped(String),
    /// A measurement, not an action: the step ran and this is what it observed. Distinct from
    /// `Skipped` because the panel must not paint a reachable endpoint as a warning, and distinct
    /// from `Installed` because nothing was installed.
    Observed(String),
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct Step {
    pub name: &'static str,
    pub outcome: StepOutcome,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct InstallOutcome {
    pub steps: Vec<Step>,
    pub bridge_exe: String,
    pub tunnel_exe: String,
    /// `false` when the tunnel certificate was absent and the owner's login step is still owed.
    pub tunnel_ready: bool,
    pub health: Option<HealthAnswer>,
}

/// What the panel can show while an install runs.
///
/// This exists because the webview is NOT allowed an event bus (AC-2 forbids `core:event:*`), so the
/// backend cannot push progress into it. The UI polls this snapshot instead: the install writes it as
/// it goes and `install_progress` reads it. One mutex, no new permission class, and no second channel
/// that could carry a secret — every field is a count or text that is already shown at the end.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct Progress {
    /// 1-based index of the step now running.
    pub index: u32,
    pub total: u32,
    /// The step's name, the same string the finished list uses.
    pub label: String,
    /// The artifact or command detail for this step: for a mise step, the tool and the line mise
    /// printed about the file it fetched. Empty when the label is all there is to say.
    pub detail: String,
    /// `true` once the run has finished, so a poller knows to stop.
    pub done: bool,
}

/// How many steps a full pass reports. Kept beside the sequence so adding a step cannot silently
/// leave the bar short of 100%.
pub const STEPS: u32 = 8;

impl Progress {
    pub fn at(index: u32, label: &str, detail: &str) -> Self {
        Self {
            index,
            total: STEPS,
            label: label.to_string(),
            detail: detail.to_string(),
            done: false,
        }
    }

    pub fn finished(total: u32) -> Self {
        Self {
            index: total,
            total,
            label: "Finished".to_string(),
            detail: String::new(),
            done: true,
        }
    }

    /// A finished snapshot whose label carries the reason, for a run that stopped early.
    pub fn with_label(mut self, label: &str) -> Self {
        self.label = label.to_string();
        self.done = true;
        self
    }
}

/// The reporter an install calls as it moves. A no-op implementation keeps every existing test and
/// every non-GUI caller source-compatible.
pub trait ProgressSink {
    fn report(&self, p: Progress);
}

/// Discards progress: what a test or a scripted caller uses.
pub struct NoProgress;
impl ProgressSink for NoProgress {
    fn report(&self, _p: Progress) {}
}

/// The line mise printed about what it fetched, for the progress detail.
///
/// mise names the asset and prints where it went; that is the honest answer to "which file". When it
/// printed nothing (a cached hit, or a terse build) the detail falls back to the label rather than
/// inventing a filename the app cannot know.
pub fn download_detail(label: &str, res: &CmdResult) -> String {
    let line = res
        .stdout
        .lines()
        .rev()
        .chain(res.stderr.lines().rev())
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with("mise "));
    match line {
        Some(l) => format!("{label} — {l}"),
        None => format!("{label} — already present, nothing to download"),
    }
}

/// Sleeping, so the bounded /health poll is testable without taking 15 real seconds.
pub trait Delay {
    fn sleep(&self, d: Duration);
}

pub struct RealDelay;

impl Delay for RealDelay {
    fn sleep(&self, d: Duration) {
        std::thread::sleep(d);
    }
}

#[derive(Default)]
pub struct FakeDelay {
    pub slept: std::sync::Mutex<Vec<Duration>>,
}

impl Delay for FakeDelay {
    fn sleep(&self, d: Duration) {
        self.slept.lock().expect("fake delay").push(d);
    }
}

/// Reads `KEY=value` pairs out of an env file, ignoring comments and blanks.
pub fn parse_env(contents: &str) -> Vec<(String, String)> {
    contents
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                return None;
            }
            let (k, v) = line.split_once('=')?;
            Some((k.trim().to_string(), v.trim().to_string()))
        })
        .collect()
}

fn env_value(contents: &str, key: &str) -> Option<String> {
    parse_env(contents)
        .into_iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v)
}

/// `deploy_root` reaches two dangerous places: the `/TR` quoted string of both scheduled tasks and
/// the file paths written. Both are local-only surfaces (the webview is the app's own page), but a
/// root containing a quote or a shell metacharacter would break out of the /TR quoting, so it is
/// refused here instead of escaped everywhere downstream.
pub fn validate_deploy_root(root: &str) -> Result<(), Failure> {
    if root.trim().is_empty() {
        return Err(Failure::Input("the deploy root is empty".to_string()));
    }
    let bad: Vec<char> = root
        .chars()
        .filter(|c| {
            matches!(c, '"' | '\'' | '|' | '&' | '<' | '>' | '^' | '%' | '!') || (*c as u32) < 32
        })
        .collect();
    if !bad.is_empty() {
        return Err(Failure::Input(format!(
            "the deploy root must not contain {}",
            bad.iter()
                .map(|c| format!("'{c}'"))
                .collect::<Vec<_>>()
                .join(", ")
        )));
    }
    Ok(())
}

/// The terminal's MCP endpoint when the operator names none. Kept identical to
/// `crates/mt5-bridge/src/main.rs:75` and the retired oracle's default, so "unset" means the same
/// address on both sides of the pipe.
pub const DEFAULT_MCP_URL: &str = "http://127.0.0.1:22346/mcp";

/// Writes `bridge.env`, keeping any token already there. Returns the outcome and the token that is
/// now in force, so the caller can poll /health with the same value the bridge will read.
///
/// This is AC-5's "no overwrite": the existing file is the source of truth for its own secrets.
///
/// `mcp_url` is written explicitly even when it equals the compiled default: an operator with a
/// non-standard port needs the file to say so, and a file that states its address can be diffed
/// against `Tools > Options > MCP` instead of requiring the reader to know the default.
pub fn ensure_env<F: Fs>(
    fs: &F,
    paths: &InstallPaths,
    mcp_token: Option<&Secret>,
    mcp_url: Option<&str>,
) -> Result<(StepOutcome, Secret), Failure> {
    let url = normalize_mcp_url(mcp_url)?;
    let path = paths.env_file();
    if let Some(existing) = fs.read(&path) {
        let token = env_value(&existing, "BRIDGE_TOKEN").ok_or_else(|| {
            Failure::File(format!(
                "{path} exists but has no BRIDGE_TOKEN; fix or delete it, then install again"
            ))
        })?;
        if env_value(&existing, "MT5_MCP_TOKEN").is_none() {
            return Err(Failure::File(format!(
                "{path} has no MT5_MCP_TOKEN (MT5: Tools > Options > MCP); add it, then install again"
            )));
        }
        // The token is kept, not regenerated: a registration elsewhere depends on this exact value.
        let _ = mcp_token;
        // The address IS updated when the operator names one: it is configuration, not a secret, and
        // a repair is the obvious moment to correct a port that moved. Unset leaves the file alone.
        if let Some(wanted) = mcp_url.map(str::trim).filter(|u| !u.is_empty()) {
            let wanted = normalize_mcp_url(Some(wanted))?;
            if env_value(&existing, "MT5_MCP_URL").as_deref() != Some(wanted.as_str()) {
                let mut lines: Vec<String> = existing
                    .lines()
                    .filter(|l| !l.starts_with("MT5_MCP_URL="))
                    .map(str::to_string)
                    .collect();
                lines.push(format!("MT5_MCP_URL={wanted}"));
                fs.write(&path, &format!("{}\n", lines.join("\n")))?;
            }
        }
        return Ok((StepOutcome::Kept, Secret::new(token)));
    }

    let mcp = mcp_token.ok_or_else(|| {
        Failure::Input(
            "the MT5 MCP token is required on the first install (MT5: Tools > Options > MCP)"
                .to_string(),
        )
    })?;
    if mcp.is_empty() {
        return Err(Failure::Input(
            "the MT5 MCP token is empty (MT5: Tools > Options > MCP)".to_string(),
        ));
    }
    let token = Secret::mint();
    let contents = format!(
        "BRIDGE_TOKEN={}\nMT5_MCP_TOKEN={}\nMT5_MCP_URL={}\nBRIDGE_BIND=127.0.0.1\nBRIDGE_PORT={}\n",
        token.expose(),
        mcp.expose(),
        url,
        paths.port
    );
    fs.write(&path, &contents)?;
    Ok((StepOutcome::Installed, token))
}

/// The `MT5_MCP_TOKEN` currently in `bridge.env`, for a check that runs after the write.
fn read_env_token<F: Fs>(fs: &F, paths: &InstallPaths) -> Option<Secret> {
    let body = fs.read(&paths.env_file())?;
    env_value(&body, "MT5_MCP_TOKEN").map(Secret::new)
}

/// Validates an operator-supplied MCP address, defaulting when absent.
///
/// Loopback only, `http` only, `/mcp` path: the terminal binds `127.0.0.1` and serves a plain
/// endpoint, so anything else is a typo that would otherwise surface as a silent "cannot read bars"
/// hours later. The scheme is deliberately NOT https — this is a same-machine call, and the bridge's
/// own MCP client speaks plain HTTP to it.
pub fn normalize_mcp_url(raw: Option<&str>) -> Result<String, Failure> {
    let value = raw.map(str::trim).filter(|v| !v.is_empty());
    let Some(value) = value else {
        return Ok(DEFAULT_MCP_URL.to_string());
    };
    let parsed = url::Url::parse(value).map_err(|e| {
        Failure::Input(format!(
            "the MT5 MCP URL does not parse ({e}); it looks like http://127.0.0.1:22346/mcp"
        ))
    })?;
    if parsed.scheme() != "http" {
        return Err(Failure::Input(format!(
            "the MT5 MCP URL must be http:// (the terminal serves plain HTTP on loopback), not {}://",
            parsed.scheme()
        )));
    }
    let host = parsed.host_str().unwrap_or_default();
    if !matches!(host, "127.0.0.1" | "localhost" | "[::1]" | "::1") {
        return Err(Failure::Input(format!(
            "the MT5 MCP URL must point at this machine (127.0.0.1/localhost), not {host}: the terminal binds loopback only"
        )));
    }
    // A trailing slash is the same endpoint (`/mcp/` and `/mcp` both reach the handler), so it is
    // normalized rather than refused — refusing it would fail an operator over a harmless keystroke.
    if parsed.path().trim_end_matches('/') != "/mcp" {
        return Err(Failure::Input(format!(
            "the MT5 MCP URL path must be /mcp, not {}",
            parsed.path()
        )));
    }
    if parsed.port().is_none() {
        return Err(Failure::Input(
            "the MT5 MCP URL needs an explicit port (22346 unless the terminal was changed)"
                .to_string(),
        ));
    }
    Ok(parsed.to_string().trim_end_matches('/').to_string())
}

/// `mise install <tool>` with the process-local env the host script sets.
fn mise_tool<R: CommandRunner>(
    runner: &R,
    tool: &str,
    env: &[(&str, &str)],
) -> Result<CmdResult, Failure> {
    let mut spec = crate::runner::CmdSpec::new("mise", &["install", tool]);
    spec.timeout = Duration::from_secs(300);
    for (k, v) in env {
        spec = spec.with_env(k, v);
    }
    let res = runner.run(&spec);
    if res.ok() {
        Ok(res)
    } else {
        Err(Failure::from_cmd("mise", &res))
    }
}

fn mise_which<R: CommandRunner>(
    runner: &R,
    tool: &str,
    env: &[(&str, &str)],
) -> Result<String, Failure> {
    let mut spec = crate::runner::CmdSpec::new("mise", &["which", tool]);
    for (k, v) in env {
        spec = spec.with_env(k, v);
    }
    let res = runner.run(&spec);
    let path = res.stdout.trim().to_string();
    if res.ok() && !path.is_empty() {
        Ok(path)
    } else {
        Err(Failure::Command {
            program: "mise".to_string(),
            cause: if res.ok() {
                format!("mise which {tool} printed nothing")
            } else {
                res.cause()
            },
        })
    }
}

/// `cloudflared tunnel list --output json`, and whether `name` is in it.
///
/// A separate function so the reuse decision is testable: the JSON shape is cloudflared's
/// (`[{"id": "...", "name": "mt5", ...}]`), and an unparseable list is treated as "not found" —
/// creating a tunnel that exists answers a loud error, which is a better failure than a silent skip.
pub fn tunnel_exists(list_json: &str, name: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(list_json)
        .ok()
        .and_then(|v| {
            v.as_array().map(|rows| {
                rows.iter()
                    .any(|r| r.get("name").and_then(|n| n.as_str()) == Some(name))
            })
        })
        .unwrap_or(false)
}

/// Registers or updates one scheduled task.
///
/// `schtasks /Create /F` is the argv equivalent of `Set-ScheduledTask` vs `Register-ScheduledTask`:
/// `/F` silently overwrites, so one command serves both "absent" and "present". The distinction the
/// AC asks for is preserved in the *outcome* ([`StepOutcome::Installed`] vs `Updated`) and in the
/// query that decides it — what must not happen is a second registration, and `/F` cannot create one.
///
/// Restart-on-failure (AC-2) needs a second call: `schtasks /Create` has no flag for it — `RestartCount`
/// and `RestartInterval` live only in the task's XML/Settings, and `/Change` cannot set them either.
/// The constant PowerShell script below is the argv-only form (the S1 lesson): the task name is the
/// only variable and it travels in `VTV_TASK`, never concatenated into the script string.
fn register_task<R: CommandRunner>(
    runner: &R,
    existing: bool,
    name: &str,
    exe: &str,
    args: Option<&str>,
) -> Result<StepOutcome, Failure> {
    let mut argv = vec![
        "/Create".to_string(),
        "/TN".to_string(),
        name.to_string(),
        "/SC".to_string(),
        "ONSTART".to_string(),
        "/TR".to_string(),
    ];
    // /TR takes one quoted command string; the exe path may contain spaces, so it is quoted here
    // and nowhere else. No user input reaches this string: exe comes from `mise which`.
    argv.push(match args {
        Some(a) => format!("\"{exe}\" {a}"),
        None => format!("\"{exe}\""),
    });
    argv.extend([
        "/RU".to_string(),
        "SYSTEM".to_string(),
        "/RL".to_string(),
        "HIGHEST".to_string(),
        // Overwrite in place, so a re-run repairs rather than duplicating.
        "/F".to_string(),
    ]);
    let arg_refs: Vec<&str> = argv.iter().map(String::as_str).collect();
    let res = runner.run(&crate::runner::CmdSpec::new("schtasks", &arg_refs));
    if !res.ok() {
        return Err(Failure::from_cmd("schtasks", &res));
    }

    // The hardening AC-2 asks for: restart on failure, no execution time limit, and start when the
    // trigger was missed. `RestartCount` caps the retries so a permanently broken bridge does not
    // spin forever.
    const SETTINGS: &str = "Set-ScheduledTask -TaskName $env:VTV_TASK -Settings \
        (New-ScheduledTaskSettingsSet -RestartCount 3 -RestartInterval (New-TimeSpan -Minutes 1) \
        -StartWhenAvailable -ExecutionTimeLimit ([TimeSpan]::Zero)) | Out-Null";
    let mut spec = crate::runner::CmdSpec::new(
        "powershell.exe",
        &["-NoProfile", "-NonInteractive", "-Command", SETTINGS],
    )
    .with_env("VTV_TASK", name);
    spec.timeout = std::time::Duration::from_secs(60);
    let settings = runner.run(&spec);
    if !settings.ok() {
        return Err(Failure::from_cmd("powershell.exe", &settings));
    }

    Ok(if existing {
        StepOutcome::Updated
    } else {
        StepOutcome::Installed
    })
}

fn task_exists<R: CommandRunner>(runner: &R, name: &str) -> bool {
    runner
        .run(&crate::runner::CmdSpec::new(
            "schtasks",
            &["/Query", "/TN", name],
        ))
        .ok()
}

/// The bounded /health poll: one deadline for the whole wait, dying on the first answer.
pub fn poll_health<H: Http, D: Delay>(
    http: &H,
    delay: &D,
    port: u16,
    token: &Secret,
) -> Option<HealthAnswer> {
    for attempt in 0..HEALTH_ATTEMPTS {
        if attempt > 0 {
            delay.sleep(Duration::from_secs(1));
        }
        if let Ok(answer) = super::status::local_health(http, port, token) {
            if answer.ok() {
                return Some(answer);
            }
        }
    }
    None
}

/// The whole install / repair pass.
pub fn install_stack<R: CommandRunner, F: Fs, H: Http, D: Delay>(
    runner: &R,
    fs: &F,
    http: &H,
    delay: &D,
    paths: &InstallPaths,
    mcp_token: Option<&Secret>,
    mcp_url: Option<&str>,
) -> Result<InstallOutcome, Failure> {
    install_stack_reporting(
        runner,
        fs,
        http,
        delay,
        paths,
        mcp_token,
        mcp_url,
        &NoProgress,
    )
}

/// The same pass, reporting each step as it starts so a UI can show where it is.
#[allow(clippy::too_many_arguments)]
pub fn install_stack_reporting<R: CommandRunner, F: Fs, H: Http, D: Delay, P: ProgressSink>(
    runner: &R,
    fs: &F,
    http: &H,
    delay: &D,
    paths: &InstallPaths,
    mcp_token: Option<&Secret>,
    mcp_url: Option<&str>,
    progress: &P,
) -> Result<InstallOutcome, Failure> {
    validate_deploy_root(&paths.deploy_root)?;
    let mut steps = Vec::new();

    // 1. the bridge binary and the tunnel binary, both from the mise registry.
    let mt5_env: &[(&str, &str)] = &[("MISE_ENV", "mt5")];
    progress.report(Progress::at(
        1,
        "mise install mt5-bridge",
        "looking up the pinned bridge release",
    ));
    let bridge_res = mise_tool(runner, "mt5-bridge", mt5_env)?;
    let bridge_detail = download_detail("mise install mt5-bridge", &bridge_res);
    let bridge_exe = mise_which(runner, "mt5-bridge", mt5_env)?;
    steps.push(Step {
        name: "mise install mt5-bridge",
        outcome: StepOutcome::Installed,
    });

    progress.report(Progress::at(2, "mise install cloudflared", &bridge_detail));
    let tunnel_res = mise_tool(runner, "cloudflared", &[])?;
    let tunnel_detail = download_detail("mise install cloudflared", &tunnel_res);
    let tunnel_exe = mise_which(runner, "cloudflared", &[])?;
    steps.push(Step {
        name: "mise install cloudflared",
        outcome: StepOutcome::Installed,
    });

    // 2. bridge.env, keeping any token already there (AC-5).
    progress.report(Progress::at(3, "bridge.env", &tunnel_detail));
    let (env_outcome, token) = ensure_env(fs, paths, mcp_token, mcp_url)?;
    steps.push(Step {
        name: "bridge.env",
        outcome: env_outcome,
    });

    // 2b. the copy the BINARY actually reads. `crates/mt5-bridge/src/main.rs:141` loads
    // `bridge.env` from the exe's own directory and parses no arguments at all, so a task pointed
    // at the deploy root would exit 1 with "BRIDGE_TOKEN is required". The deploy-root file stays
    // canonical (it is what the panel and the /health poll read); this mirror is rewritten on every
    // install, which also restores it after `mise install` replaces the exe's directory.
    let env_body = fs.read(&paths.env_file()).ok_or_else(|| {
        Failure::Input(format!("{} vanished after being written", paths.env_file()))
    })?;
    let exe_env = paths.env_beside_exe(&bridge_exe);
    if exe_env != paths.env_file() {
        fs.write(&exe_env, &env_body)?;
        steps.push(Step {
            name: "bridge.env beside the binary",
            outcome: StepOutcome::Installed,
        });
    }

    progress.report(Progress::at(
        4,
        "cloudflared tunnel",
        "checking the named tunnel",
    ));
    // 3. the named tunnel: reuse when it exists.
    let list = runner.run(&crate::runner::CmdSpec::new(
        "cloudflared",
        &["tunnel", "list", "--output", "json"],
    ));
    let cert_present = list.ok();
    let tunnel_ready = cert_present;
    if !cert_present {
        // No certificate: the owner's login step is owed (architecture decision 5). Say which
        // command, verbatim, and stop before pretending to create a tunnel.
        steps.push(Step {
            name: "cloudflared tunnel",
            outcome: StepOutcome::Skipped(
                "run `cloudflared tunnel login` in a browser on this host once, then install again"
                    .to_string(),
            ),
        });
    } else if tunnel_exists(&list.stdout, TUNNEL_NAME) {
        steps.push(Step {
            name: "cloudflared tunnel",
            outcome: StepOutcome::Kept,
        });
    } else {
        let create = runner.run(&crate::runner::CmdSpec::new(
            "cloudflared",
            &["tunnel", "create", TUNNEL_NAME],
        ));
        if !create.ok() {
            return Err(Failure::from_cmd("cloudflared", &create));
        }
        steps.push(Step {
            name: "cloudflared tunnel",
            outcome: StepOutcome::Installed,
        });
    }
    if tunnel_ready {
        // The config the scheduled task reads. Rewritten each run (it is derived, and a stale copy
        // pointing at another tunnel is worse than a rewrite).
        let cfg = format!(
            "# Written by the BridgeManager install. Derived: rewritten on every run.\n\
             tunnel: {TUNNEL_NAME}\n\
             credentials-file: {root}\\.cloudflared\\{TUNNEL_NAME}.json\n\
             ingress:\n  \
             - service: http://127.0.0.1:{port}\n  \
             - service: http_status:404\n",
            root = paths.deploy_root,
            port = paths.port
        );
        fs.write(&paths.tunnel_config(), &cfg)?;
    }

    progress.report(Progress::at(
        5,
        "scheduled tasks",
        "registering VTVBridge and VTVTunnel",
    ));
    // 4. the two scheduled tasks: updated when present, registered when absent.
    // The bridge task takes NO arguments: the binary reads `bridge.env` beside its exe and parses
    // nothing else (step 2b mirrors the file there).
    let bridge_existed = task_exists(runner, "VTVBridge");
    let out = register_task(runner, bridge_existed, "VTVBridge", &bridge_exe, None)?;
    steps.push(Step {
        name: "task VTVBridge",
        outcome: out,
    });

    if tunnel_ready {
        let tunnel_existed = task_exists(runner, "VTVTunnel");
        let out = register_task(
            runner,
            tunnel_existed,
            "VTVTunnel",
            &tunnel_exe,
            Some(&format!(
                "tunnel --config \"{}\" --no-autoupdate run {TUNNEL_NAME}",
                paths.tunnel_config()
            )),
        )?;
        steps.push(Step {
            name: "task VTVTunnel",
            outcome: out,
        });
    } else {
        steps.push(Step {
            name: "task VTVTunnel",
            outcome: StepOutcome::Skipped("the tunnel is not registered yet".to_string()),
        });
    }

    progress.report(Progress::at(
        6,
        "starting the services",
        "schtasks /Run for both tasks",
    ));
    // 5. start both, then one bounded /health poll.
    for task in ["VTVBridge", "VTVTunnel"] {
        if task == "VTVTunnel" && !tunnel_ready {
            continue;
        }
        let run = runner.run(&crate::runner::CmdSpec::new(
            "schtasks",
            &["/Run", "/TN", task],
        ));
        // A task that was already running answers non-zero on some builds. The /health poll below is
        // the real check, so this is recorded and not fatal.
        steps.push(Step {
            name: if run.ok() {
                "start"
            } else {
                "start (reported non-zero)"
            },
            outcome: StepOutcome::Installed,
        });
    }
    progress.report(Progress::at(
        7,
        "bridge /health",
        "one bounded poll (up to 15 s)",
    ));
    let health = poll_health(http, delay, paths.port, &token);
    steps.push(Step {
        name: "bridge /health",
        outcome: if health.is_some() {
            StepOutcome::Installed
        } else {
            StepOutcome::Skipped(format!(
                "no /health answer in {HEALTH_ATTEMPTS}s — is the MT5 terminal running with MCP enabled?"
            ))
        },
    });

    // 6. the terminal's MCP endpoint AND the token, checked together before anything is claimed.
    //
    // The bridge answers `/health` even when the terminal's MCP is dead: like the oracle it only
    // raises on a call. Reporting "installed" on that alone is the false green the owner hit — a
    // healthy bridge that can never fetch a bar. Sending the token distinguishes "address wrong"
    // (fix the port) from "token wrong" (copy it again), which a tokenless probe cannot.
    let mcp_addr = normalize_mcp_url(mcp_url)?;
    let check_token = read_env_token(fs, paths).unwrap_or(token.clone());
    progress.report(Progress::at(
        8,
        "MT5 MCP address and token",
        &format!("probing {mcp_addr} with the stored token"),
    ));
    steps.push(Step {
        name: "MT5 MCP address and token",
        outcome: match probe_mcp(http, &mcp_addr) {
            Err(why) => StepOutcome::Skipped(why),
            Ok(_) => match probe_token(http, &mcp_addr, &check_token) {
                Ok(status) => {
                    StepOutcome::Observed(format!("token accepted (the terminal answered {status})"))
                }
                Err(TokenProbe::NoToken) => StepOutcome::Skipped(
                    "address reachable; no token to test yet — paste the MT5 MCP token and install again"
                        .to_string(),
                ),
                Err(TokenProbe::Refused(why)) | Err(TokenProbe::Unreachable(why)) => {
                    StepOutcome::Skipped(why)
                }
            },
        },
    });

    progress.report(Progress::finished(STEPS));

    Ok(InstallOutcome {
        steps,
        bridge_exe,
        tunnel_exe,
        tunnel_ready,
        health,
    })
}

/// One tokenless request to the terminal's own MCP address, judged by what that server answers.
///
/// Deliberately reuses the `Http` seam with a GET-free POST: the endpoint rejects an unauthenticated
/// call with 401, which is the proof we want and needs no secret. A 2xx (a permissive local build)
/// is equally proof. Everything else — refused, timeout, 404 — is a wrong address, and the message
/// says which.
pub fn probe_mcp<H: Http>(http: &H, url: &str) -> Result<String, String> {
    match http.post_json(url, MCP_INIT_BODY, Duration::from_secs(MCP_PROBE_TIMEOUT_S)) {
        Ok(resp) if resp.status == 401 || (200..300).contains(&resp.status) => {
            Ok(format!("answered {}", resp.status))
        }
        Ok(resp) => Err(format!(
            "{} answered {} — is that the terminal's MCP address (Tools > Options > MCP)?",
            url, resp.status
        )),
        Err(e) => Err(format!(
            "{} did not answer ({}) — is the MT5 terminal running with MCP enabled, on that port?",
            url,
            e.cause()
        )),
    }
}

/// The JSON-RPC handshake every MCP server answers. One constant so the probe and the token check
/// cannot drift into asking different questions.
const MCP_INIT_BODY: &str = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"BridgeManager","version":"0.3.0"}}}"#;

/// Checks the TOKEN, once the address is known to answer. The two failures feel identical to a
/// tokenless probe, and their fixes differ: **Refused** (401/403 with the bearer sent) means the
/// address is right and the credential is stale — copy it again from Tools > Options > MCP;
/// **Unreachable** means the address itself. `NoToken` is not a failure: the panel shows a neutral
/// dot and waits.
pub fn probe_token<H: Http>(http: &H, url: &str, token: &Secret) -> Result<u16, TokenProbe> {
    if token.is_empty() {
        return Err(TokenProbe::NoToken);
    }
    let bearer = format!("Bearer {}", token.expose());
    match http.post_json_auth(
        url,
        MCP_INIT_BODY,
        &bearer,
        Duration::from_secs(MCP_PROBE_TIMEOUT_S),
    ) {
        Ok(resp) if (200..300).contains(&resp.status) => Ok(resp.status),
        // An MCP server that authenticated us but rejected the handshake parameters still proves the
        // token: 400/422 with a JSON-RPC error body is a working credential.
        Ok(resp) if matches!(resp.status, 400 | 406 | 422) && looks_like_jsonrpc(&resp.body) => {
            Ok(resp.status)
        }
        Ok(resp) if resp.status == 401 || resp.status == 403 => Err(TokenProbe::Refused(format!(
            "the terminal answered {} with the token sent: the address is right, the token is not.              Copy it again from MT5: Tools > Options > MCP",
            resp.status
        ))),
        Ok(resp) => Err(TokenProbe::Unreachable(format!(
            "{} answered {} — is that the terminal's MCP address (Tools > Options > MCP)?",
            url, resp.status
        ))),
        Err(e) => Err(TokenProbe::Unreachable(format!(
            "{} did not answer ({}) — is the MT5 terminal running with MCP enabled, on that port?",
            url,
            e.cause()
        ))),
    }
}

/// Why a token probe did not end in "accepted". The panel paints these differently: `NoToken` is
/// neutral, `Refused` is red on the token dot only, `Unreachable` is red on the address dot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TokenProbe {
    /// No credential was supplied, so there is nothing to test.
    NoToken,
    /// The endpoint answered and rejected the credential: an address problem would not do this.
    Refused(String),
    /// The address did not behave like the terminal's MCP endpoint.
    Unreachable(String),
}

fn looks_like_jsonrpc(body: &str) -> bool {
    body.contains("\"jsonrpc\"") || body.contains("\"error\"")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::{Fake as FakeHttp, HttpError, HttpResponse};
    use crate::runner::Fake as FakeRunner;
    use crate::runner::FakeFs;

    fn paths() -> InstallPaths {
        InstallPaths::default()
    }

    fn ok(s: &str) -> CmdResult {
        CmdResult::success(s)
    }

    fn token() -> Secret {
        Secret::new("mcp-token".to_string())
    }

    fn health_ok() -> Result<HttpResponse, HttpError> {
        Ok(HttpResponse {
            status: 200,
            body: r#"{"ok":true,"server":"Exness-MT5Real28"}"#.to_string(),
        })
    }

    #[test]
    fn a_first_install_writes_the_env_and_registers_both_tasks() {
        let runner = FakeRunner::new(vec![
            ok(""),                              // mise install mt5-bridge
            ok("C:\\mise\\mt5-bridge.exe\r\n"),  // mise which
            ok(""),                              // mise install cloudflared
            ok("C:\\mise\\cloudflared.exe\r\n"), // mise which
            ok("[]"),                            // tunnel list: empty
            ok("SUCCESS"),                       // tunnel create mt5
            CmdResult::failure(1, "ERROR: The system cannot find the file specified.\r\n"), // VTVBridge?
            ok("SUCCESS"), // create VTVBridge
            ok(""),        // its restart-on-failure settings
            CmdResult::failure(1, "ERROR: The system cannot find the file specified.\r\n"), // VTVTunnel?
            ok("SUCCESS"), // create VTVTunnel
            ok(""),        // its restart-on-failure settings
            ok("SUCCESS"), // start bridge
            ok("SUCCESS"), // start tunnel
        ]);
        let fs = FakeFs::default();
        let http = FakeHttp::with_gets(vec![health_ok()]);
        let delay = FakeDelay::default();
        let out = install_stack(&runner, &fs, &http, &delay, &paths(), Some(&token()), None)
            .expect("install ok");
        assert!(out.health.is_some());
        assert!(out.tunnel_ready);
        assert_eq!(out.bridge_exe, "C:\\mise\\mt5-bridge.exe");

        let env = fs.read(&paths().env_file()).expect("env written");
        assert!(env.contains("BRIDGE_TOKEN="));
        assert!(env.contains("MT5_MCP_TOKEN=mcp-token"));
        assert!(env.contains("BRIDGE_PORT=8797"));

        let names: Vec<&str> = out.steps.iter().map(|s| s.name).collect();
        assert!(names.contains(&"task VTVBridge"));
        assert!(names.contains(&"task VTVTunnel"));
        let bridge_step = out
            .steps
            .iter()
            .find(|s| s.name == "task VTVBridge")
            .unwrap();
        assert_eq!(bridge_step.outcome, StepOutcome::Installed);
    }

    #[test]
    fn a_second_run_keeps_the_token_and_updates_the_tasks_instead_of_registering_them() {
        let existing = "BRIDGE_TOKEN=the-original-token\nMT5_MCP_TOKEN=orig-mcp\nBRIDGE_BIND=127.0.0.1\nBRIDGE_PORT=8797\n";
        let fs = FakeFs::with_file(&paths().env_file(), existing);
        let runner = FakeRunner::new(vec![
            ok(""),
            ok("C:\\mise\\mt5-bridge.exe\r\n"),
            ok(""),
            ok("C:\\mise\\cloudflared.exe\r\n"),
            ok(r#"[{"id":"abc","name":"mt5"}]"#), // tunnel exists: reuse
            ok("TaskName: \\VTVBridge\r\n"),      // exists
            ok("SUCCESS"),                        // /Create /F
            ok(""),                               // restart-on-failure settings
            ok("TaskName: \\VTVTunnel\r\n"),      // exists
            ok("SUCCESS"),
            ok(""), // settings
            ok("SUCCESS"),
            ok("SUCCESS"),
        ]);
        let http = FakeHttp::with_gets(vec![health_ok()]);
        let delay = FakeDelay::default();
        let out =
            install_stack(&runner, &fs, &http, &delay, &paths(), None, None).expect("repair ok");

        // The env file is the source of truth for its own secrets: never rewritten. config.yml is
        // derived, so rewriting that one is correct.
        assert!(
            !fs.written().contains(&paths().env_file()),
            "bridge.env must not be rewritten when it exists: {:?}",
            fs.written()
        );
        let env = fs.read(&paths().env_file()).expect("still there");
        assert!(env.contains("BRIDGE_TOKEN=the-original-token"));

        for name in ["task VTVBridge", "task VTVTunnel"] {
            let step = out.steps.iter().find(|s| s.name == name).expect(name);
            assert_eq!(step.outcome, StepOutcome::Updated, "{name}");
        }
        let tunnel_step = out
            .steps
            .iter()
            .find(|s| s.name == "cloudflared tunnel")
            .unwrap();
        assert_eq!(tunnel_step.outcome, StepOutcome::Kept);

        // The /health poll used the token the env file holds, not a fresh one.
        assert_eq!(
            out.health.expect("answered").server().as_deref(),
            Some("Exness-MT5Real28")
        );
    }

    #[test]
    fn no_tunnel_certificate_stops_the_tunnel_half_and_names_the_owners_command() {
        let runner = FakeRunner::new(vec![
            ok(""),
            ok("C:\\mise\\mt5-bridge.exe\r\n"),
            ok(""),
            ok("C:\\mise\\cloudflared.exe\r\n"),
            CmdResult::failure(1, "Cannot determine default configuration path\r\n"), // tunnel list
            CmdResult::failure(1, "ERROR: The system cannot find the file specified.\r\n"),
            ok("SUCCESS"), // create VTVBridge
            ok(""),        // its restart-on-failure settings
            ok("SUCCESS"), // start bridge
        ]);
        let fs = FakeFs::default();
        let http = FakeHttp::with_gets(vec![health_ok()]);
        let delay = FakeDelay::default();
        let out = install_stack(&runner, &fs, &http, &delay, &paths(), Some(&token()), None)
            .expect("partial install is not an error");
        assert!(!out.tunnel_ready);
        let tunnel_step = out
            .steps
            .iter()
            .find(|s| s.name == "cloudflared tunnel")
            .unwrap();
        match &tunnel_step.outcome {
            StepOutcome::Skipped(why) => assert!(why.contains("cloudflared tunnel login"), "{why}"),
            other => panic!("expected a skip, got {other:?}"),
        }
        let task = out
            .steps
            .iter()
            .find(|s| s.name == "task VTVTunnel")
            .unwrap();
        assert!(matches!(task.outcome, StepOutcome::Skipped(_)));
        assert!(
            fs.read(&paths().tunnel_config()).is_none(),
            "no config for a tunnel that is not registered"
        );
    }

    #[test]
    fn an_existing_env_without_a_token_is_an_error_not_a_regeneration() {
        let fs = FakeFs::with_file(&paths().env_file(), "MT5_MCP_TOKEN=x\n");
        let err = ensure_env(&fs, &paths(), Some(&token()), None).expect_err("broken env");
        assert!(err.cause().contains("no BRIDGE_TOKEN"), "{}", err.cause());
        assert!(fs.written().is_empty());
    }

    #[test]
    fn a_first_install_without_an_mcp_token_is_refused_before_writing() {
        let fs = FakeFs::default();
        let err = ensure_env(&fs, &paths(), None, None).expect_err("no mcp token");
        assert!(
            err.cause().contains("Tools > Options > MCP"),
            "{}",
            err.cause()
        );
        assert!(fs.written().is_empty());
    }

    #[test]
    fn the_health_poll_gives_up_on_one_deadline_and_does_not_spin_free() {
        let http = FakeHttp::with_gets(vec![
            Err(HttpError::Timeout),
            Err(HttpError::Timeout),
            Err(HttpError::Timeout),
        ]);
        let delay = FakeDelay::default();
        let out = poll_health(&http, &delay, 8797, &Secret::new("t".to_string()));
        assert!(out.is_none());
        assert_eq!(
            http.total_calls(),
            HEALTH_ATTEMPTS as usize,
            "the poll stops at the deadline, not at the first failure"
        );
        assert!(
            delay
                .slept
                .lock()
                .unwrap()
                .iter()
                .all(|d| *d == Duration::from_secs(1)),
            "one second between tries, never a tighter loop"
        );
    }

    #[test]
    fn the_health_poll_stops_at_the_first_good_answer() {
        let http = FakeHttp::with_gets(vec![Err(HttpError::Timeout), health_ok()]);
        let delay = FakeDelay::default();
        let out = poll_health(&http, &delay, 8797, &Secret::new("t".to_string()));
        assert!(out.is_some());
        assert_eq!(http.total_calls(), 2);
        assert_eq!(delay.slept.lock().unwrap().len(), 1);
    }

    #[test]
    fn every_registered_task_gets_restart_on_failure_settings() {
        // AC-2's "restart-on-failure": schtasks /Create cannot set it, so a second, argv-only call
        // must follow every registration. Pin the count and the carrier: two tasks mean two
        // settings calls, and the task name travels in the environment, never in the script text
        // (the S1 lesson: trailing -Command tokens are concatenated into the script).
        let runner = FakeRunner::new(vec![
            ok(""),
            ok("C:\\mise\\mt5-bridge.exe\r\n"),
            ok(""),
            ok("C:\\mise\\cloudflared.exe\r\n"),
            ok("[]"),
            ok("SUCCESS"),
            CmdResult::failure(1, "not found\r\n"),
            ok("SUCCESS"),
            ok(""),
            CmdResult::failure(1, "not found\r\n"),
            ok("SUCCESS"),
            ok(""),
            ok("SUCCESS"),
            ok("SUCCESS"),
        ]);
        let fs = FakeFs::default();
        let http = FakeHttp::with_gets(vec![health_ok()]);
        let out = install_stack(
            &runner,
            &fs,
            &http,
            &FakeDelay::default(),
            &paths(),
            Some(&token()),
            None,
        )
        .expect("install ok");
        assert!(out.health.is_some());

        let specs = runner.recorded();
        let settings: Vec<&crate::runner::CmdSpec> = specs
            .iter()
            .filter(|s| s.program == "powershell.exe")
            .collect();
        assert_eq!(settings.len(), 2, "one settings call per registered task");
        let carried: Vec<&str> = settings
            .iter()
            .filter_map(|s| {
                s.env
                    .iter()
                    .find(|(k, _)| k == "VTV_TASK")
                    .map(|(_, v)| v.as_str())
            })
            .collect();
        assert_eq!(carried, vec!["VTVBridge", "VTVTunnel"]);
        for spec in settings {
            let script = spec.args.join(" ");
            assert!(
                script.contains("RestartCount") && script.contains("ExecutionTimeLimit"),
                "{script}"
            );
            assert!(
                !script.contains("VTVBridge") && !script.contains("VTVTunnel"),
                "the task name must not reach the script text: {script}"
            );
        }
    }

    #[test]
    fn the_task_gets_an_env_file_the_binary_can_actually_read() {
        // The binary loads `bridge.env` beside its own exe and parses no arguments
        // (crates/mt5-bridge/src/main.rs:141), so the deploy-root copy is NOT what it reads when
        // `mise which` puts the exe elsewhere. Without this mirror the task starts, finds no
        // BRIDGE_TOKEN, and exits 1 — a silent "installed" bridge that never answers /health.
        let runner = FakeRunner::new(vec![
            ok(""),
            ok("C:\\mise\\installs\\mt5-bridge\\0.3.0\\mt5-bridge.exe\r\n"),
            ok(""),
            ok("C:\\mise\\cloudflared.exe\r\n"),
            ok("[]"),
            ok("SUCCESS"),
            CmdResult::failure(1, "not found\r\n"),
            ok("SUCCESS"),
            ok(""),
            CmdResult::failure(1, "not found\r\n"),
            ok("SUCCESS"),
            ok(""),
            ok("SUCCESS"),
            ok("SUCCESS"),
        ]);
        let fs = FakeFs::default();
        let http = FakeHttp::with_gets(vec![health_ok()]);
        install_stack(
            &runner,
            &fs,
            &http,
            &FakeDelay::default(),
            &paths(),
            Some(&token()),
            None,
        )
        .expect("install ok");

        let exe_env = r"C:\mise\installs\mt5-bridge\0.3.0\bridge.env";
        let written = fs.read(exe_env).expect("env mirrored beside the exe");
        assert!(written.contains("BRIDGE_TOKEN="), "{written}");
        assert!(written.contains("MT5_MCP_TOKEN=mcp-token"), "{written}");
        assert_eq!(
            written,
            fs.read(&paths().env_file()).expect("canonical copy"),
            "the mirror is a copy, not a second source of truth"
        );
        // And the bridge task's /TR is the bare exe: the binary would ignore anything appended.
        // (The tunnel task keeps its --config; cloudflared does read that flag.)
        let specs = runner.recorded();
        let mut trs: Vec<(String, String)> = Vec::new();
        for spec in specs.iter().filter(|s| s.program == "schtasks") {
            if !spec.args.iter().any(|a| a == "/Create") {
                continue;
            }
            let tn = spec
                .args
                .iter()
                .position(|a| a == "/TN")
                .and_then(|i| spec.args.get(i + 1))
                .cloned()
                .unwrap_or_default();
            let tr = spec
                .args
                .iter()
                .position(|a| a == "/TR")
                .and_then(|i| spec.args.get(i + 1))
                .cloned()
                .unwrap_or_default();
            trs.push((tn, tr));
        }
        let bridge_tr = trs
            .iter()
            .find(|(tn, _)| tn == "VTVBridge")
            .map(|(_, tr)| tr.as_str())
            .expect("the bridge task");
        assert_eq!(
            bridge_tr, "\"C:\\mise\\installs\\mt5-bridge\\0.3.0\\mt5-bridge.exe\"",
            "the bridge reads no arguments"
        );
    }

    #[test]
    fn tunnel_reuse_is_decided_by_name_from_cloudflares_json() {
        assert!(tunnel_exists(
            r#"[{"id":"a","name":"other"},{"id":"b","name":"mt5"}]"#,
            "mt5"
        ));
        assert!(!tunnel_exists(r#"[{"id":"a","name":"other"}]"#, "mt5"));
        assert!(!tunnel_exists("", "mt5"));
        assert!(!tunnel_exists("not json", "mt5"));
    }

    #[test]
    fn the_mcp_address_defaults_and_refuses_what_the_terminal_cannot_serve() {
        // Unset means the compiled default, the same address the bridge binary falls back to
        // (crates/mt5-bridge/src/main.rs:75). An empty string is "unset", not "empty URL".
        assert_eq!(normalize_mcp_url(None).unwrap(), DEFAULT_MCP_URL);
        assert_eq!(normalize_mcp_url(Some("")).unwrap(), DEFAULT_MCP_URL);
        assert_eq!(normalize_mcp_url(Some("   ")).unwrap(), DEFAULT_MCP_URL);
        // A moved port is the reason this field exists.
        assert_eq!(
            normalize_mcp_url(Some("http://127.0.0.1:22347/mcp")).unwrap(),
            "http://127.0.0.1:22347/mcp"
        );
        assert_eq!(
            normalize_mcp_url(Some("http://localhost:22346/mcp/")).unwrap(),
            "http://localhost:22346/mcp"
        );
        // Everything the terminal cannot serve is refused with a message naming the shape.
        for (bad, needle) in [
            ("https://127.0.0.1:22346/mcp", "must be http://"),
            ("http://10.0.0.5:22346/mcp", "must point at this machine"),
            ("http://127.0.0.1:22346/", "must be /mcp"),
            ("http://127.0.0.1/mcp", "explicit port"),
            ("not a url", "does not parse"),
        ] {
            let err = normalize_mcp_url(Some(bad)).expect_err(bad);
            assert!(
                err.cause().contains(needle),
                "{bad}: expected {needle:?}, got {:?}",
                err.cause()
            );
        }
    }

    #[test]
    fn the_env_file_states_the_mcp_address_it_was_installed_with() {
        let fs = FakeFs::default();
        ensure_env(
            &fs,
            &paths(),
            Some(&token()),
            Some("http://127.0.0.1:22347/mcp"),
        )
        .expect("first install");
        let env = fs.read(&paths().env_file()).expect("written");
        assert!(
            env.contains("MT5_MCP_URL=http://127.0.0.1:22347/mcp"),
            "{env}"
        );
        // And the default is written too, never implied: a file that states its address can be
        // diffed against Tools > Options > MCP.
        let fs2 = FakeFs::default();
        ensure_env(&fs2, &paths(), Some(&token()), None).expect("first install");
        let env2 = fs2.read(&paths().env_file()).expect("written");
        assert!(
            env2.contains(&format!("MT5_MCP_URL={DEFAULT_MCP_URL}")),
            "{env2}"
        );
    }

    #[test]
    fn a_repair_corrects_a_moved_mcp_port_without_touching_the_tokens() {
        // The owner's case: bridge.env exists from an earlier install, the terminal's port changed.
        // The address is configuration, so a repair updates it; the two secrets stay byte-identical.
        let existing = "BRIDGE_TOKEN=keep-me\nMT5_MCP_TOKEN=keep-mcp\nMT5_MCP_URL=http://127.0.0.1:22346/mcp\nBRIDGE_BIND=127.0.0.1\nBRIDGE_PORT=8797\n";
        let fs = FakeFs::with_file(&paths().env_file(), existing);
        let (outcome, tok) =
            ensure_env(&fs, &paths(), None, Some("http://127.0.0.1:22347/mcp")).expect("repair");
        assert_eq!(outcome, StepOutcome::Kept);
        assert_eq!(tok.expose(), "keep-me");
        let env = fs.read(&paths().env_file()).expect("still there");
        assert!(
            env.contains("MT5_MCP_URL=http://127.0.0.1:22347/mcp"),
            "{env}"
        );
        assert!(env.contains("BRIDGE_TOKEN=keep-me"), "{env}");
        assert!(env.contains("MT5_MCP_TOKEN=keep-mcp"), "{env}");
        assert_eq!(
            env.matches("MT5_MCP_URL=").count(),
            1,
            "the old line is replaced, not appended: {env}"
        );
    }

    #[test]
    fn a_repair_with_no_address_named_leaves_the_existing_one_alone() {
        let existing = "BRIDGE_TOKEN=t\nMT5_MCP_TOKEN=m\nMT5_MCP_URL=http://127.0.0.1:22347/mcp\nBRIDGE_BIND=127.0.0.1\nBRIDGE_PORT=8797\n";
        let fs = FakeFs::with_file(&paths().env_file(), existing);
        ensure_env(&fs, &paths(), None, None).expect("repair");
        assert!(
            fs.written().is_empty(),
            "an unchanged env must not be rewritten: {:?}",
            fs.written()
        );
    }

    #[test]
    fn the_mcp_probe_accepts_only_an_answer_that_proves_the_endpoint() {
        // 401 is the measured real answer from MetaTrader5-MCP for a tokenless call: it proves the
        // server is up and speaking MCP without needing the secret.
        let http = FakeHttp::with_posts(vec![Ok(HttpResponse {
            status: 401,
            body: String::new(),
        })]);
        assert!(probe_mcp(&http, DEFAULT_MCP_URL).is_ok());
        assert_eq!(http.post_calls().len(), 1);

        // A permissive local build answering 200 is equally proof.
        let http = FakeHttp::with_posts(vec![Ok(HttpResponse {
            status: 200,
            body: "{}".into(),
        })]);
        assert!(probe_mcp(&http, DEFAULT_MCP_URL).is_ok());

        // A wrong port: something answers, but it is not the terminal.
        let http = FakeHttp::with_posts(vec![Ok(HttpResponse {
            status: 404,
            body: String::new(),
        })]);
        let err = probe_mcp(&http, "http://127.0.0.1:22347/mcp").expect_err("404");
        assert!(err.contains("404"), "{err}");
        assert!(err.contains("Tools > Options > MCP"), "{err}");

        // Nothing listening at all.
        let http = FakeHttp::with_posts(vec![Err(HttpError::Transport("refused".into()))]);
        let err = probe_mcp(&http, DEFAULT_MCP_URL).expect_err("refused");
        assert!(err.contains("refused"), "{err}");
    }

    #[test]
    fn probe_token_separates_a_wrong_address_from_a_wrong_token() {
        let tok = Secret::new("good-token".to_string());
        // Token accepted: 2xx.
        let http = FakeHttp::with_posts(vec![Ok(HttpResponse {
            status: 200,
            body: r#"{"jsonrpc":"2.0","id":1,"result":{}}"#.into(),
        })]);
        assert_eq!(probe_token(&http, DEFAULT_MCP_URL, &tok).unwrap(), 200);
        // And the token actually travelled: a tokenless probe would have got 401 instead.
        let auths = http.auth_calls();
        assert_eq!(auths.len(), 1);
        assert_eq!(auths[1 - 1].1, "Bearer good-token");
        assert_eq!(auths[0].0, DEFAULT_MCP_URL);

        // Token rejected: 401 WITH the bearer sent. The address is right, so the verdict must be
        // Refused, not Unreachable — that is the whole point of sending the credential.
        let http = FakeHttp::with_posts(vec![Ok(HttpResponse {
            status: 401,
            body: String::new(),
        })]);
        match probe_token(&http, DEFAULT_MCP_URL, &tok) {
            Err(TokenProbe::Refused(m)) => assert!(m.contains("the token is not"), "{m}"),
            other => panic!("expected Refused, got {other:?}"),
        }

        // A server that dislikes our handshake but authenticated us is still a working token: 400
        // with a JSON-RPC error body (some builds reject protocolVersion).
        let http = FakeHttp::with_posts(vec![Ok(HttpResponse {
            status: 400,
            body: r#"{"jsonrpc":"2.0","error":{"code":-32600,"message":"bad request"}}"#.into(),
        })]);
        assert_eq!(probe_token(&http, DEFAULT_MCP_URL, &tok).unwrap(), 400);

        // Address wrong: something else on the port answers 404.
        let http = FakeHttp::with_posts(vec![Ok(HttpResponse {
            status: 404,
            body: String::new(),
        })]);
        match probe_token(&http, DEFAULT_MCP_URL, &tok) {
            Err(TokenProbe::Unreachable(m)) => assert!(m.contains("404"), "{m}"),
            other => panic!("expected Unreachable, got {other:?}"),
        }

        // Nothing listening.
        let http = FakeHttp::with_posts(vec![Err(HttpError::Transport("refused".into()))]);
        match probe_token(&http, DEFAULT_MCP_URL, &tok) {
            Err(TokenProbe::Unreachable(m)) => assert!(m.contains("refused"), "{m}"),
            other => panic!("expected Unreachable, got {other:?}"),
        }

        // No token: neutral no-token verdict, and no request leaves the process.
        let http = FakeHttp::with_posts(vec![]);
        assert_eq!(
            probe_token(&http, DEFAULT_MCP_URL, &Secret::new(String::new())),
            Err(TokenProbe::NoToken)
        );
        assert!(http.post_calls().is_empty(), "no token means no request");
    }

    #[test]
    fn the_address_probe_accepts_401_as_reachable() {
        // The terminal answers a tokenless initialize with 401 (measured against MetaTrader5-MCP),
        // which is proof the port and path are right — the URL dot's green, before any token exists.
        let http = FakeHttp::with_posts(vec![Ok(HttpResponse {
            status: 401,
            body: String::new(),
        })]);
        assert!(probe_mcp(&http, DEFAULT_MCP_URL).is_ok());
        assert_eq!(http.post_calls().len(), 1, "exactly one tokenless probe");

        // A dead port is the red branch.
        let http = FakeHttp::with_posts(vec![Err(HttpError::Transport("refused".into()))]);
        let err = probe_mcp(&http, DEFAULT_MCP_URL).expect_err("refused");
        assert!(err.contains("did not answer"), "{err}");
    }

    #[test]
    fn install_reports_the_mcp_endpoint_as_its_own_step() {
        let runner = FakeRunner::new(vec![
            ok(""),
            ok("C:\\mise\\mt5-bridge.exe\r\n"),
            ok(""),
            ok("C:\\mise\\cloudflared.exe\r\n"),
            ok("[]"),
            ok("SUCCESS"),
            CmdResult::failure(1, "not found\r\n"),
            ok("SUCCESS"),
            ok(""),
            CmdResult::failure(1, "not found\r\n"),
            ok("SUCCESS"),
            ok(""),
            ok("SUCCESS"),
            ok("SUCCESS"),
        ]);
        let fs = FakeFs::default();
        // Two POSTs are consumed by the MCP step: the tokenless address probe, then the token probe.
        // Both answer 401 (the measured real behavior) — reachable address, stale token.
        let http = FakeHttp::with_gets_and_posts(
            vec![health_ok()],
            vec![
                Ok(HttpResponse {
                    status: 401,
                    body: String::new(),
                }),
                Ok(HttpResponse {
                    status: 401,
                    body: String::new(),
                }),
            ],
        );
        let out = install_stack(
            &runner,
            &fs,
            &http,
            &FakeDelay::default(),
            &paths(),
            Some(&token()),
            None,
        )
        .expect("install ok");

        let step = out
            .steps
            .iter()
            .find(|s| s.name == "MT5 MCP address and token")
            .expect("the MCP step exists");
        match &step.outcome {
            // The fake answers 401 to the AUTHENTICATED call, which is the "address right, token
            // wrong" branch: the step must say so rather than claim the token works.
            StepOutcome::Skipped(detail) => {
                assert!(detail.contains("the token is not"), "{detail}")
            }
            other => panic!("expected the token verdict, got {other:?}"),
        }
        // It probed the terminal's own address, not the bridge's port.
        let posts = http.post_calls();
        let url = posts.last().expect("one probe").0.clone();
        assert_eq!(url, DEFAULT_MCP_URL);
    }

    #[test]
    fn the_env_parser_ignores_comments_and_blanks() {
        let parsed = parse_env("# c\n\nA=1\nB = two \n");
        assert_eq!(
            parsed,
            vec![
                ("A".to_string(), "1".to_string()),
                ("B".to_string(), "two".to_string())
            ]
        );
    }

    #[test]
    fn a_deploy_root_with_quoting_or_metacharacters_is_refused() {
        for bad in [
            r#"C:\say \"hi\""#,
            r"C:\a|b",
            r"C:\a&b",
            r"C:\a<b",
            "C:\u{1}b",
            "",
            "   ",
        ] {
            let err = validate_deploy_root(bad).expect_err(bad);
            assert!(matches!(err, Failure::Input(_)), "{bad}: {err}");
        }
        validate_deploy_root(r"C:\Users\Administrator\project\vtradingview").expect("default ok");
        validate_deploy_root(r"D:\vtv bridge").expect("spaces are fine");
    }

    #[test]
    fn the_paths_match_the_host_installers_layout() {
        let p = paths();
        assert_eq!(p.port, 8797, "bridge/host-install.ps1:83");
        assert!(p.env_file().ends_with(r"bridge\bridge.env"));
        assert!(p.tunnel_config().ends_with(r"cloudflared\config.yml"));
    }
}
