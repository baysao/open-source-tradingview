//! `control`: start, stop and restart the two scheduled tasks.
//!
//! One operation, three shapes:
//!
//! - **start** on a running task is a no-op answer, not an error: the operator pressed a button
//!   whose goal already holds.
//! - **stop+start** is how "restart" is implemented, because schtasks has no restart verb. It is
//!   two commands and the tests pin their order.
//! - **stop** on a task that is not running ends the task's process if there is one, and answers
//!   "already stopped" otherwise.
//!
//! The kill path uses the Get-CimInstance-by-command-line pattern from `kill_fake_mcp.ps1` (L-034):
//! `taskkill /IM` would kill a process the operator started by hand, and a task's own process is
//! identified by what it is running, not by its image name alone. Both are recorded as argv.

use crate::runner::{CmdResult, CommandRunner};

use super::Failure;

/// Which service. Serialised as `bridge` / `tunnel` over the IPC boundary; the names are the
/// schtasks task names the host installer registers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SvcName {
    Bridge,
    Tunnel,
}

impl SvcName {
    /// The scheduled-task name alongside `bridge/host-install.ps1`'s `VTVBridge`/`VTVTunnel`.
    pub fn task_name(self) -> &'static str {
        match self {
            SvcName::Bridge => "VTVBridge",
            SvcName::Tunnel => "VTVTunnel",
        }
    }

    /// The image name the kill fallback looks for.
    pub fn image(self) -> &'static str {
        match self {
            SvcName::Bridge => "mt5-bridge.exe",
            SvcName::Tunnel => "cloudflared.exe",
        }
    }

    pub fn parse(raw: &str) -> Result<Self, Failure> {
        match raw {
            "bridge" => Ok(SvcName::Bridge),
            "tunnel" => Ok(SvcName::Tunnel),
            other => Err(Failure::Input(format!(
                "unknown service {other}: expected bridge or tunnel"
            ))),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SvcAction {
    Start,
    Stop,
    Restart,
}

impl SvcAction {
    pub fn parse(raw: &str) -> Result<Self, Failure> {
        match raw {
            "start" => Ok(SvcAction::Start),
            "stop" => Ok(SvcAction::Stop),
            "restart" => Ok(SvcAction::Restart),
            other => Err(Failure::Input(format!(
                "unknown action {other}: expected start, stop or restart"
            ))),
        }
    }
}

/// What the operation did, so the UI can say so instead of guessing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ControlOutcome {
    Started,
    /// Start was asked for and the task was already running.
    AlreadyRunning,
    Stopped,
    /// Stop was asked for and no process or running task was found.
    AlreadyStopped,
    Restarted,
}

impl ControlOutcome {
    /// The line the panel shows. Names the service, because the panel has two buttons.
    pub fn message(self, svc: SvcName) -> String {
        let name = match svc {
            SvcName::Bridge => "bridge",
            SvcName::Tunnel => "tunnel",
        };
        match self {
            ControlOutcome::Started => format!("{name} started"),
            ControlOutcome::AlreadyRunning => format!("{name} was already running"),
            ControlOutcome::Stopped => format!("{name} stopped"),
            ControlOutcome::AlreadyStopped => format!("{name} was already stopped"),
            ControlOutcome::Restarted => format!("{name} restarted"),
        }
    }
}

/// The schtasks state we care about: `Running`, `Ready` (registered, not running), or gone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskState {
    Running,
    Ready,
    Missing,
}

/// Extracts the state from `schtasks /Query /FO LIST /V` output.
///
/// The field is `Status:` and the value is one of `Running`, `Ready`, `Disabled`, `Queued`. Anything
/// unparseable is [`TaskState::Missing`] with `None`-style caution: the caller reports the query's
/// own cause rather than inventing "stopped".
pub fn parse_task_state(stdout: &str) -> TaskState {
    for line in stdout.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        if key.trim().eq_ignore_ascii_case("Status") {
            return match value.trim() {
                "Running" => TaskState::Running,
                "Ready" | "Disabled" | "Queued" => TaskState::Ready,
                _ => TaskState::Missing,
            };
        }
    }
    TaskState::Missing
}

/// `schtasks /Query /TN <name> /FO LIST /V`, the state probe every operation starts with.
pub fn query<R: CommandRunner>(runner: &R, svc: SvcName) -> Result<TaskState, Failure> {
    let mut spec = crate::runner::CmdSpec::new(
        "schtasks",
        &["/Query", "/TN", svc.task_name(), "/FO", "LIST", "/V"],
    );
    spec.timeout = std::time::Duration::from_secs(20);
    let res = runner.run(&spec);
    if res.ok() {
        Ok(parse_task_state(&res.stdout))
    } else {
        // A missing task answers exit 1 with "ERROR: The system cannot find the file specified." on
        // stderr; that is a state, not a failure of the operation, so it is reported as Missing and
        // the caller names the installer as the fix.
        let cause = res.cause();
        if cause.contains("cannot find") || cause.contains("does not exist") {
            Ok(TaskState::Missing)
        } else {
            Err(Failure::from_cmd("schtasks", &res))
        }
    }
}

fn schtasks<R: CommandRunner>(runner: &R, verb: &str, svc: SvcName) -> CmdResult {
    let spec = crate::runner::CmdSpec::new("schtasks", &[verb, "/TN", svc.task_name()]);
    runner.run(&spec)
}

/// Ends a process by image name AND command-line substring, the L-034 pattern.
///
/// Parameters travel in the PROCESS ENVIRONMENT and the constant script reads `$env:` — never as
/// trailing `-Command` tokens, which PowerShell concatenates INTO the script text (an argv-passed
/// image name would execute as its own statement), and never spliced into the string.
pub fn kill_process<R: CommandRunner>(
    runner: &R,
    svc: SvcName,
    needle: &str,
) -> Result<bool, Failure> {
    const SCRIPT: &str = "Get-CimInstance Win32_Process | \
        Where-Object { $_.Name -eq $env:VTV_IMAGE -and $_.CommandLine -like ('*' + $env:VTV_NEEDLE + '*') } | \
        ForEach-Object { Stop-Process -Id $_.ProcessId -Force; Write-Output $_.ProcessId }";
    let mut spec = crate::runner::CmdSpec::new(
        "powershell.exe",
        &["-NoProfile", "-NonInteractive", "-Command", SCRIPT],
    )
    .with_env("VTV_IMAGE", svc.image())
    .with_env("VTV_NEEDLE", needle);
    spec.timeout = std::time::Duration::from_secs(30);
    let res = runner.run(&spec);
    if !res.ok() {
        return Err(Failure::from_cmd("powershell.exe", &res));
    }
    Ok(!res.stdout.trim().is_empty())
}

/// start / stop / restart, with the no-op answers the AC-4 case list names.
///
/// * `needle` is the substring that identifies the process for the kill fallback: the bridge's own
///   exe path for the bridge, the tunnel URL for cloudflared. Passed as an argument, so it cannot
///   become part of an interpolated command string.
pub fn control<R: CommandRunner>(
    runner: &R,
    svc: SvcName,
    action: SvcAction,
    needle: &str,
) -> Result<ControlOutcome, Failure> {
    let state = query(runner, svc)?;
    match action {
        SvcAction::Start => start(runner, svc, state),
        SvcAction::Stop => stop(runner, svc, state, needle),
        SvcAction::Restart => {
            // schtasks has no restart: stop then start, always in that order (the tests pin it).
            let stopped = stop(runner, svc, state, needle)?;
            let started = start(runner, svc, TaskState::Ready)?;
            match (stopped, started) {
                (
                    ControlOutcome::Stopped | ControlOutcome::AlreadyStopped,
                    ControlOutcome::Started,
                ) => Ok(ControlOutcome::Restarted),
                (_, other) => Ok(other),
            }
        }
    }
}

fn start<R: CommandRunner>(
    runner: &R,
    svc: SvcName,
    state: TaskState,
) -> Result<ControlOutcome, Failure> {
    match state {
        TaskState::Running => Ok(ControlOutcome::AlreadyRunning),
        TaskState::Missing => Err(Failure::Command {
            program: "schtasks".to_string(),
            cause: format!(
                "the scheduled task {} does not exist; run install to register it",
                svc.task_name()
            ),
        }),
        TaskState::Ready => {
            let res = schtasks(runner, "/Run", svc);
            if res.ok() {
                Ok(ControlOutcome::Started)
            } else {
                Err(Failure::from_cmd("schtasks", &res))
            }
        }
    }
}

fn stop<R: CommandRunner>(
    runner: &R,
    svc: SvcName,
    state: TaskState,
    needle: &str,
) -> Result<ControlOutcome, Failure> {
    match state {
        TaskState::Missing => {
            // No task, but the process may still be there (started by hand, or the task was
            // deleted under it). The kill fallback is the only thing that can help.
            if kill_process(runner, svc, needle)? {
                Ok(ControlOutcome::Stopped)
            } else {
                Ok(ControlOutcome::AlreadyStopped)
            }
        }
        TaskState::Ready => {
            // Registered, not running: nothing to end. Still sweep the process, because a
            // half-broken host can have an orphan from a previous run.
            let _ = kill_process(runner, svc, needle)?;
            Ok(ControlOutcome::AlreadyStopped)
        }
        TaskState::Running => {
            let res = schtasks(runner, "/End", svc);
            if !res.ok() {
                return Err(Failure::from_cmd("schtasks", &res));
            }
            // /End asks the task to stop; the child may survive it (the tunnel does, because
            // cloudflared ignores the console signal). One sweep makes "stopped" true.
            let _ = kill_process(runner, svc, needle)?;
            Ok(ControlOutcome::Stopped)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::{CmdResult, Fake};

    fn running() -> CmdResult {
        CmdResult::success("TaskName:      \\VTVBridge\r\nStatus:        Running\r\n")
    }

    fn ready() -> CmdResult {
        CmdResult::success("TaskName:      \\VTVBridge\r\nStatus:        Ready\r\n")
    }

    fn missing() -> CmdResult {
        CmdResult::failure(1, "ERROR: The system cannot find the file specified.\r\n")
    }

    fn killed(pid: &str) -> CmdResult {
        CmdResult::success(&format!("{pid}\r\n"))
    }

    fn nothing_killed() -> CmdResult {
        CmdResult::success("")
    }

    #[test]
    fn start_on_a_running_task_is_a_no_op_answer_and_runs_one_command() {
        let fake = Fake::new(vec![running()]);
        let out = control(&fake, SvcName::Bridge, SvcAction::Start, "needle").expect("ok");
        assert_eq!(out, ControlOutcome::AlreadyRunning);
        assert_eq!(fake.calls(), 1, "only the state query; no /Run");
        assert_eq!(
            out.message(SvcName::Bridge),
            "bridge was already running",
            "the panel must not claim it started something"
        );
    }

    #[test]
    fn start_on_a_ready_task_runs_it() {
        let fake = Fake::new(vec![ready(), CmdResult::success("SUCCESS")]);
        let out = control(&fake, SvcName::Tunnel, SvcAction::Start, "needle").expect("ok");
        assert_eq!(out, ControlOutcome::Started);
        let specs = fake.recorded();
        assert_eq!(specs.len(), 2);
        assert_eq!(specs[0].args[0], "/Query");
        assert_eq!(specs[1].args, vec!["/Run", "/TN", "VTVTunnel"]);
    }

    #[test]
    fn stop_on_a_running_task_ends_it_then_sweeps_the_process() {
        let fake = Fake::new(vec![
            running(),
            CmdResult::success("SUCCESS"),
            killed("1234"),
        ]);
        let out = control(&fake, SvcName::Bridge, SvcAction::Stop, "mt5-bridge.exe").expect("ok");
        assert_eq!(out, ControlOutcome::Stopped);
        let specs = fake.recorded();
        assert_eq!(specs.len(), 3);
        assert_eq!(specs[1].args, vec!["/End", "/TN", "VTVBridge"]);
        assert_eq!(specs[2].program, "powershell.exe");
        assert!(
            !specs[2].args.iter().any(|a| a.contains("mt5-bridge.exe")),
            "the image name travels in the environment, never in the script text nor as a trailing \
             -Command token (which powershell concatenates INTO the script)"
        );
        assert_eq!(
            specs[2].args.len(),
            4,
            "a fixed argv shape: flags and the constant script"
        );
        assert!(specs[2]
            .env
            .iter()
            .any(|(k, v)| k == "VTV_IMAGE" && v == "mt5-bridge.exe"));
        assert!(specs[2]
            .env
            .iter()
            .any(|(k, v)| k == "VTV_NEEDLE" && v == "mt5-bridge.exe"));
    }

    #[test]
    fn stop_on_a_ready_task_is_already_stopped_even_after_a_sweep() {
        let fake = Fake::new(vec![ready(), nothing_killed()]);
        let out = control(&fake, SvcName::Tunnel, SvcAction::Stop, "needle").expect("ok");
        assert_eq!(out, ControlOutcome::AlreadyStopped);
    }

    #[test]
    fn stop_with_a_missing_task_reports_stopped_if_the_orphan_was_killed() {
        let fake = Fake::new(vec![missing(), killed("99")]);
        let out = control(&fake, SvcName::Bridge, SvcAction::Stop, "needle").expect("ok");
        assert_eq!(out, ControlOutcome::Stopped);
    }

    #[test]
    fn stop_with_a_missing_task_and_no_orphan_is_already_stopped() {
        let fake = Fake::new(vec![missing(), nothing_killed()]);
        let out = control(&fake, SvcName::Bridge, SvcAction::Stop, "needle").expect("ok");
        assert_eq!(out, ControlOutcome::AlreadyStopped);
    }

    #[test]
    fn start_on_a_missing_task_names_install_as_the_next_action() {
        let fake = Fake::new(vec![missing()]);
        let err = control(&fake, SvcName::Bridge, SvcAction::Start, "needle").expect_err("missing");
        assert!(err.cause().contains("run install"), "{}", err.cause());
    }

    #[test]
    fn restart_is_stop_then_start_in_that_order() {
        let fake = Fake::new(vec![
            running(),                     // the one query restart makes
            CmdResult::success("SUCCESS"), // /End
            killed("42"),                  // sweep
            CmdResult::success("SUCCESS"), // /Run
        ]);
        let out = control(&fake, SvcName::Bridge, SvcAction::Restart, "needle").expect("ok");
        assert_eq!(out, ControlOutcome::Restarted);
        let specs = fake.recorded();
        assert_eq!(specs.len(), 4, "one query, /End, the sweep, /Run");
        assert_eq!(specs[1].args, vec!["/End", "/TN", "VTVBridge"]);
        assert_eq!(specs[3].args, vec!["/Run", "/TN", "VTVBridge"]);
    }

    #[test]
    fn restart_when_nothing_was_running_still_ends_up_started() {
        let fake = Fake::new(vec![
            ready(),
            nothing_killed(),
            CmdResult::success("SUCCESS"),
        ]);
        let out = control(&fake, SvcName::Tunnel, SvcAction::Restart, "needle").expect("ok");
        assert_eq!(out, ControlOutcome::Restarted);
    }

    #[test]
    fn a_query_that_failed_for_another_reason_is_an_error_not_a_state() {
        let fake = Fake::new(vec![CmdResult::failure(5, "ERROR: Access is denied.\r\n")]);
        let err = query(&fake, SvcName::Bridge).expect_err("denied");
        assert_eq!(err.cause(), "ERROR: Access is denied.");
    }

    #[test]
    fn task_state_parsing_covers_running_ready_and_absent() {
        assert_eq!(parse_task_state("Status:  Running\r\n"), TaskState::Running);
        assert_eq!(parse_task_state("Status:  Ready\r\n"), TaskState::Ready);
        assert_eq!(parse_task_state("Status:  Disabled\r\n"), TaskState::Ready);
        assert_eq!(parse_task_state("TaskName: \\X\r\n"), TaskState::Missing);
        assert_eq!(parse_task_state(""), TaskState::Missing);
    }

    #[test]
    fn the_only_names_and_actions_accepted_are_the_known_ones() {
        assert_eq!(SvcName::parse("bridge").expect("ok"), SvcName::Bridge);
        assert_eq!(SvcName::parse("tunnel").expect("ok"), SvcName::Tunnel);
        assert!(SvcName::parse("bridge; rm -rf /").is_err());
        assert!(SvcName::parse("").is_err());
        assert_eq!(SvcAction::parse("restart").expect("ok"), SvcAction::Restart);
        assert!(SvcAction::parse("reboot").is_err());
    }

    #[test]
    fn the_task_names_match_the_host_installer() {
        // bridge/host-install.ps1:98-99 registers exactly these two names.
        assert_eq!(SvcName::Bridge.task_name(), "VTVBridge");
        assert_eq!(SvcName::Tunnel.task_name(), "VTVTunnel");
    }
}
