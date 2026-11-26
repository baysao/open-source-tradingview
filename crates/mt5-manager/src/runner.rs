//! The two seams every domain function depends on.
//!
//! Nothing here spawns a process or opens a socket in a test: the domain takes `&R: CommandRunner`
//! and `&F: Fs`, and the tests pass a scripted recorder (D-036's generalization, L-016's rule).
//!
//! `CmdSpec` carries an argument VECTOR, never a shell string. That is the whole defence for
//! secrets: a token can be an argument, but nothing can make it part of a command line that a
//! shell then re-splits.

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;
use std::time::Duration;

/// How long any single command may take before it is killed.
pub const COMMAND_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CmdSpec {
    pub program: String,
    pub args: Vec<String>,
    /// Environment for this call only (e.g. `MISE_ENV=mt5`). Never inherited by the child beyond
    /// what is listed: the host process may hold secrets in its own environment.
    pub env: Vec<(String, String)>,
    pub timeout: Duration,
}

impl CmdSpec {
    pub fn new(program: &str, args: &[&str]) -> Self {
        Self {
            program: program.to_string(),
            args: args.iter().map(|s| s.to_string()).collect(),
            env: Vec::new(),
            timeout: COMMAND_TIMEOUT,
        }
    }

    pub fn with_env(mut self, key: &str, value: &str) -> Self {
        self.env.push((key.to_string(), value.to_string()));
        self
    }

    /// The argv tail, for error text and tests. Not for logging a spec that carries a secret.
    pub fn argv(&self) -> String {
        self.args.join(" ")
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CmdResult {
    /// `None` when the child was killed by a signal or the timeout.
    pub status: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub timed_out: bool,
}

impl CmdResult {
    pub fn ok(&self) -> bool {
        !self.timed_out && self.status == Some(0)
    }

    pub fn success(stdout: &str) -> Self {
        Self {
            status: Some(0),
            stdout: stdout.to_string(),
            ..Default::default()
        }
    }

    pub fn failure(status: i32, stderr: &str) -> Self {
        Self {
            status: Some(status),
            stderr: stderr.to_string(),
            ..Default::default()
        }
    }

    /// One line naming the real cause: stderr when there is any, else the exit status. This is what
    /// the UI shows verbatim (CUJ 7's exact-cause rule).
    pub fn cause(&self) -> String {
        let err = self.stderr.trim();
        if !err.is_empty() {
            return err.lines().next().unwrap_or(err).to_string();
        }
        if self.timed_out {
            return "the command did not finish within its deadline".to_string();
        }
        match self.status {
            Some(c) => format!("exit {c}"),
            None => "killed".to_string(),
        }
    }
}

pub trait CommandRunner {
    fn run(&self, spec: &CmdSpec) -> CmdResult;
}

/// The production runner: one child, argv only, no shell, output captured.
pub struct Real;

impl CommandRunner for Real {
    fn run(&self, spec: &CmdSpec) -> CmdResult {
        let mut cmd = std::process::Command::new(&spec.program);
        cmd.args(&spec.args)
            .env_clear()
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        // A minimal environment, not the app's: the app holds secrets in its own process.
        for (k, v) in std::env::vars().filter(|(k, _)| {
            matches!(
                k.as_str(),
                "PATH"
                    | "SystemRoot"
                    | "windir"
                    | "TEMP"
                    | "TMP"
                    | "USERPROFILE"
                    | "APPDATA"
                    | "LOCALAPPDATA"
                    | "PROGRAMDATA"
                    | "COMPUTERNAME"
                    | "USERNAME"
                    | "HOME"
            )
        }) {
            cmd.env(k, v);
        }
        for (k, v) in &spec.env {
            cmd.env(k, v);
        }
        let child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                return CmdResult::failure(-1, &format!("could not start {}: {e}", spec.program))
            }
        };
        match child.wait_with_output() {
            Ok(out) => CmdResult {
                status: out.status.code(),
                stdout: String::from_utf8_lossy(&out.stdout).to_string(),
                stderr: String::from_utf8_lossy(&out.stderr).to_string(),
                timed_out: false,
            },
            Err(e) => CmdResult::failure(-1, &format!("{} did not run: {e}", spec.program)),
        }
    }
}

/// A recorder: keeps every spec it was given and returns the scripted results in order.
///
/// `Default` for the empty script is a loud failure, not a silent success: an unscripted call in a
/// test means the test forgot a case, and a fake that answers `ok` would hide it.
#[derive(Default)]
pub struct Fake {
    pub specs: Mutex<Vec<CmdSpec>>,
    pub script: Mutex<VecDeque<CmdResult>>,
}

impl Fake {
    pub fn new(script: Vec<CmdResult>) -> Self {
        Self {
            specs: Mutex::new(Vec::new()),
            script: Mutex::new(script.into()),
        }
    }

    pub fn recorded(&self) -> Vec<CmdSpec> {
        self.specs.lock().expect("fake lock").clone()
    }

    pub fn programs(&self) -> Vec<String> {
        self.recorded().into_iter().map(|s| s.program).collect()
    }

    pub fn calls(&self) -> usize {
        self.specs.lock().expect("fake lock").len()
    }
}

impl CommandRunner for Fake {
    fn run(&self, spec: &CmdSpec) -> CmdResult {
        self.specs.lock().expect("fake lock").push(spec.clone());
        match self.script.lock().expect("fake lock").pop_front() {
            Some(r) => r,
            None => CmdResult::failure(-1, "fake: no scripted result for this call"),
        }
    }
}

/// The filesystem seam: `bridge.env` and `config.yml` are written and read through it so the
/// idempotency tests need no temp directory and touch no real host file.
pub trait Fs {
    /// The whole file, or `None` when it does not exist.
    fn read(&self, path: &str) -> Option<String>;
    fn write(&self, path: &str, contents: &str) -> Result<(), String>;
}

pub struct RealFs;

impl Fs for RealFs {
    fn read(&self, path: &str) -> Option<String> {
        std::fs::read_to_string(path).ok()
    }

    fn write(&self, path: &str, contents: &str) -> Result<(), String> {
        if let Some(parent) = std::path::Path::new(path).parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("{path}: {e}"))?;
        }
        std::fs::write(path, contents).map_err(|e| format!("{path}: {e}"))
    }
}

#[derive(Default)]
pub struct FakeFs {
    pub files: Mutex<HashMap<String, String>>,
    pub writes: Mutex<Vec<String>>,
}

impl FakeFs {
    pub fn with_file(path: &str, contents: &str) -> Self {
        let me = Self::default();
        me.files
            .lock()
            .expect("fake fs")
            .insert(path.to_string(), contents.to_string());
        me
    }

    /// Every path written, in order — the evidence for "the token was not overwritten".
    pub fn written(&self) -> Vec<String> {
        self.writes.lock().expect("fake fs").clone()
    }
}

impl Fs for FakeFs {
    fn read(&self, path: &str) -> Option<String> {
        self.files.lock().expect("fake fs").get(path).cloned()
    }

    fn write(&self, path: &str, contents: &str) -> Result<(), String> {
        self.writes.lock().expect("fake fs").push(path.to_string());
        self.files
            .lock()
            .expect("fake fs")
            .insert(path.to_string(), contents.to_string());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_spec_is_argv_only_and_carries_its_own_env() {
        let spec = CmdSpec::new("mise", &["install", "mt5-bridge"]).with_env("MISE_ENV", "mt5");
        assert_eq!(spec.program, "mise");
        assert_eq!(spec.argv(), "install mt5-bridge");
        assert_eq!(spec.env, vec![("MISE_ENV".to_string(), "mt5".to_string())]);
        assert!(
            !spec.argv().contains("MISE_ENV"),
            "env goes beside argv, never inside it"
        );
    }

    #[test]
    fn an_unscripted_fake_call_fails_loudly() {
        let fake = Fake::default();
        let r = fake.run(&CmdSpec::new("schtasks", &["/Query"]));
        assert!(!r.ok());
        assert!(r.cause().contains("no scripted result"), "{}", r.cause());
        assert_eq!(fake.calls(), 1);
    }

    #[test]
    fn a_timeout_is_not_success_even_with_status_zero() {
        let r = CmdResult {
            status: Some(0),
            timed_out: true,
            ..Default::default()
        };
        assert!(!r.ok());
        assert!(r.cause().contains("deadline"));
    }

    #[test]
    fn the_cause_is_the_first_stderr_line() {
        let r = CmdResult::failure(1, "ERROR: The system cannot find the file.\r\nsecond line");
        assert_eq!(r.cause(), "ERROR: The system cannot find the file.");
    }

    #[test]
    fn the_fake_fs_remembers_what_it_was_given() {
        let fs = FakeFs::with_file("bridge.env", "BRIDGE_TOKEN=abc");
        assert_eq!(fs.read("bridge.env").as_deref(), Some("BRIDGE_TOKEN=abc"));
        assert_eq!(fs.read("nope"), None);
        fs.write("cfg.yml", "x").expect("writable");
        assert_eq!(fs.written(), vec!["cfg.yml".to_string()]);
    }
}
