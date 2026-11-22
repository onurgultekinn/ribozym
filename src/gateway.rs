//! ToolGateway (§12) — the ONE door between agents and the OS. No agent may
//! spawn a subprocess directly; the gateway enforces, in order:
//! **normalize → allowlist → isolated env → run → output cap → log** (§12.1).
//!
//! Key rules implemented here:
//! * argv-form execution only — no shell, so injection paths are closed (§12.2);
//! * allowlist matching is FULL argument-vector equality, never prefix:
//!   `cargo check` ✔ but `cargo check --config build.rs=...` ✘ (§12.2);
//! * child env is scrubbed of `*_KEY` / `*_TOKEN` / `*_SECRET` (§12.3);
//! * output is capped (`output_cap_kb`) with a truncation marker;
//! * `preflight` reports missing toolchains BEFORE a run starts (§12.4).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::Deserialize;

use crate::config::project::ToolchainSpec;
use crate::error::{Error, Result};

/// Gateway decision for one requested command line (§12.1).
#[derive(Debug, Clone, PartialEq)]
pub enum Check {
    /// Command admitted; `argv` is the normalized argument vector and
    /// `toolchain` the entry whose policy governs the run.
    Allowed {
        toolchain: String,
        argv: Vec<String>,
    },
    /// Command rejected — maps to the TOOLCHAIN_NOT_ALLOWED event (§12.3).
    Denied { reason: DenyReason },
}

#[derive(Debug, Clone, PartialEq)]
pub enum DenyReason {
    EmptyCommand,
    NotAllowlisted { argv: Vec<String> },
}

/// Sanitized child environment: parent env minus secret-shaped names (§12.3).
/// PATH/HOME/LANG style variables survive; `*_KEY`, `*_TOKEN`, `*_SECRET` die.
pub fn scrub_env(env: &BTreeMap<String, String>) -> Vec<(String, String)> {
    env.iter()
        .filter(|(k, _)| {
            let up = k.to_ascii_uppercase();
            !(up.ends_with("_KEY")
                || up.ends_with("_TOKEN")
                || up.ends_with("_SECRET")
                || up == "API_KEY")
        })
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}

/// Whitespace-normalize a command line into argv (no shell semantics, §12.2).
pub fn normalize(cmd: &str) -> Vec<String> {
    cmd.split_whitespace().map(|s| s.to_string()).collect()
}

/// Result of one executed command.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ToolOutput {
    pub argv: Vec<String>,
    pub toolchain: String,
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    pub duration_ms: u64,
    /// True when output exceeded `output_cap_kb` and was truncated.
    pub truncated: bool,
    /// True when the command was killed after `timeout_s`.
    pub timed_out: bool,
}

/// Execution abstraction so tests can assert on argv/env without spawning (§21.1).
pub trait CommandRunner: Send + Sync {
    /// `argv[0]` is the program; no shell is ever involved.
    fn run(
        &self,
        argv: &[String],
        cwd: &Path,
        env: &[(String, String)],
        timeout: Duration,
    ) -> RunnerOutcome;
}

#[derive(Debug, Clone, PartialEq)]
pub struct RunnerOutcome {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    pub timed_out: bool,
}

/// Real runner — `std::process::Command`, argv form, env replacement, timeout.
pub struct SystemRunner;

impl CommandRunner for SystemRunner {
    fn run(
        &self,
        argv: &[String],
        cwd: &Path,
        env: &[(String, String)],
        timeout: Duration,
    ) -> RunnerOutcome {
        let Some(program) = argv.first() else {
            return RunnerOutcome {
                exit_code: -1,
                stdout: String::new(),
                stderr: "SPAWN_FAILED: empty argv".into(),
                timed_out: false,
            };
        };
        let mut cmd = std::process::Command::new(program);
        cmd.args(&argv[1..])
            .current_dir(cwd)
            .env_clear()
            .envs(env.iter().map(|(k, v)| (k, v)))
            // Explicitly piped: without this, `child.stdout.take()` yields
            // None and ALL tool output silently lands on the parent console.
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        run_capped(&mut cmd, timeout)
    }
}

/// Read a piped child stream to EOF on its own thread. Both pipes need a
/// reader concurrently — draining them sequentially deadlocks once the child
/// fills the second pipe's OS buffer while the first is still open.
fn read_pipe_to_end<R: std::io::Read + Send + 'static>(
    pipe: Option<R>,
) -> std::thread::JoinHandle<String> {
    std::thread::spawn(move || {
        let mut buf = String::new();
        if let Some(mut p) = pipe {
            let _ = std::io::Read::read_to_string(&mut p, &mut buf);
        }
        buf
    })
}

fn run_capped(cmd: &mut std::process::Command, timeout: Duration) -> RunnerOutcome {
    let started = Instant::now();
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            return RunnerOutcome {
                exit_code: -1,
                stdout: String::new(),
                stderr: format!("SPAWN_FAILED: {e}"),
                timed_out: false,
            }
        }
    };
    let out = read_pipe_to_end(child.stdout.take());
    let err = read_pipe_to_end(child.stderr.take());
    let status = loop {
        match child.try_wait() {
            Ok(Some(st)) => break Some(st),
            Ok(None) => {
                if started.elapsed() >= timeout {
                    let _ = child.kill();
                    let _ = child.wait(); // reap: no zombie
                    break None;
                }
                std::thread::sleep(Duration::from_millis(2));
            }
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = out.join();
                let _ = err.join();
                return RunnerOutcome {
                    exit_code: -1,
                    stdout: String::new(),
                    stderr: format!("WAIT_FAILED: {e}"),
                    timed_out: false,
                };
            }
        }
    };
    // Pipes hit EOF when the child exited (or was killed and reaped above),
    // so joining both readers now cannot block.
    let (stdout, stderr) = (
        out.join().unwrap_or_default(),
        err.join().unwrap_or_default(),
    );
    match status {
        Some(st) => RunnerOutcome {
            exit_code: st.code().unwrap_or(-1),
            stdout,
            stderr,
            timed_out: false,
        },
        None => RunnerOutcome {
            exit_code: -1,
            stdout,
            stderr,
            timed_out: true,
        },
    }
}

/// Locate an executable on PATH without spawning it (preflight, §12.4).
pub fn which(program: &str, env: &BTreeMap<String, String>) -> Option<PathBuf> {
    let path = env.get("PATH").or_else(|| env.get("Path"))?;
    let exts: Vec<&str> = if cfg!(windows) {
        vec![".exe", ".cmd", ".bat"]
    } else {
        vec![""]
    };
    for dir in std::env::split_paths(path) {
        for ext in &exts {
            let cand = dir.join(format!("{program}{ext}"));
            if cand.is_file() {
                return Some(cand);
            }
        }
    }
    None
}

pub struct ToolGateway {
    toolchains: BTreeMap<String, ToolchainSpec>,
    workdir: PathBuf,
    runner: std::sync::Arc<dyn CommandRunner>,
    env: BTreeMap<String, String>,
}

impl ToolGateway {
    pub fn new(
        toolchains: BTreeMap<String, ToolchainSpec>,
        workdir: PathBuf,
        runner: std::sync::Arc<dyn CommandRunner>,
        env: BTreeMap<String, String>,
    ) -> Self {
        Self {
            toolchains,
            workdir,
            runner,
            env,
        }
    }

    /// Normalize + full-argv allowlist check (§12.2). Pure — never executes.
    pub fn check(&self, cmd: &str) -> Check {
        let argv = normalize(cmd);
        if argv.is_empty() {
            return Check::Denied {
                reason: DenyReason::EmptyCommand,
            };
        }
        for (name, spec) in &self.toolchains {
            for allowed in &spec.allowed {
                if normalize(allowed) == argv {
                    return Check::Allowed {
                        toolchain: name.clone(),
                        argv,
                    };
                }
            }
        }
        Check::Denied {
            reason: DenyReason::NotAllowlisted { argv },
        }
    }

    /// The full pass: check → (only on allow) execute in the isolated env.
    /// Denied commands NEVER reach the runner — proven by test (§12.1).
    pub fn run(&self, cmd: &str) -> Result<ToolOutput> {
        let (toolchain, argv) = match self.check(cmd) {
            Check::Allowed { toolchain, argv } => (toolchain, argv),
            Check::Denied { reason } => {
                let detail = match &reason {
                    DenyReason::EmptyCommand => "empty command".to_string(),
                    DenyReason::NotAllowlisted { argv } => argv.join(" "),
                };
                return Err(Error::ToolchainNotAllowed(format!(
                    "TOOLCHAIN_NOT_ALLOWED: {detail}"
                )));
            }
        };
        let spec = &self.toolchains[&toolchain];
        let child_env = if spec.env_scrub == "deny" {
            scrub_env(&self.env)
        } else {
            self.env
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect()
        };
        let started = Instant::now();
        let out = self.runner.run(
            &argv,
            &self.workdir,
            &child_env,
            Duration::from_secs(spec.timeout_s),
        );
        let cap = (spec.output_cap_kb * 1024) as usize;
        let (stdout, stdout_trunc) = cap_str(&out.stdout, cap);
        let (stderr, stderr_trunc) = cap_str(&out.stderr, cap);
        Ok(ToolOutput {
            argv,
            toolchain,
            exit_code: out.exit_code,
            stdout,
            stderr,
            duration_ms: started.elapsed().as_millis() as u64,
            truncated: stdout_trunc || stderr_trunc,
            timed_out: out.timed_out,
        })
    }

    /// Run-start preflight (§12.4): every allowlisted tool's program binary
    /// must resolve on PATH. Returns the list of missing program names;
    /// non-empty → caller must fail with TOOLCHAIN_MISSING before starting.
    pub fn preflight(&self) -> Vec<String> {
        let mut missing: Vec<String> = Vec::new();
        for spec in self.toolchains.values() {
            for allowed in &spec.allowed {
                let argv = normalize(allowed);
                let Some(program) = argv.first() else {
                    continue;
                };
                if !missing.iter().any(|m| m == program) && which(program, &self.env).is_none() {
                    missing.push(program.clone());
                }
            }
        }
        missing
    }
}

fn cap_str(s: &str, cap: usize) -> (String, bool) {
    if s.len() <= cap {
        (s.to_string(), false)
    } else {
        // Truncate on a char boundary, never panic inside UTF-8.
        let mut end = cap;
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        (s[..end].to_string(), true)
    }
}

/// UTF-8 boundary test for byte-cap cuts (mirrors `str::is_char_boundary`
/// for a &[u8] slice; used by the web door's output cap).
pub fn is_char_boundary(bytes: &[u8], at: usize) -> bool {
    at < bytes.len() && (bytes[at] < 0x80 || bytes[at] >= 0xC0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Default)]
    struct FakeRunner {
        seen: Mutex<Vec<Vec<String>>>,
        seen_env: Mutex<Vec<Vec<(String, String)>>>,
    }

    impl CommandRunner for FakeRunner {
        fn run(
            &self,
            argv: &[String],
            _cwd: &Path,
            env: &[(String, String)],
            _timeout: Duration,
        ) -> RunnerOutcome {
            self.seen.lock().unwrap().push(argv.to_vec());
            self.seen_env.lock().unwrap().push(env.to_vec());
            RunnerOutcome {
                exit_code: 0,
                stdout: "ok".into(),
                stderr: String::new(),
                timed_out: false,
            }
        }
    }

    fn rust_spec() -> ToolchainSpec {
        ToolchainSpec {
            allowed: vec![
                "cargo check".into(),
                "cargo test".into(),
                "cargo clippy".into(),
                "cargo fmt".into(),
                "cargo build --release".into(),
            ],
            fs_read: vec!["workdir/**".into()],
            fs_write: vec!["workdir/**".into()],
            env_scrub: "deny".into(),
            timeout_s: 300,
            output_cap_kb: 1,
        }
    }

    fn sample_env() -> BTreeMap<String, String> {
        let mut e = BTreeMap::new();
        e.insert("PATH".into(), std::env::var("PATH").unwrap_or_default());
        e.insert("HOME".into(), "/home/tester".into());
        e.insert("OPENROUTER_API_KEY".into(), "sk-secret".into());
        e.insert("GH_TOKEN".into(), "gih-secret".into());
        e.insert("MY_SECRET".into(), "s".into());
        e
    }

    fn gateway(runner: std::sync::Arc<dyn CommandRunner>) -> ToolGateway {
        let mut m = BTreeMap::new();
        m.insert("rust".to_string(), rust_spec());
        ToolGateway::new(m, PathBuf::from("."), runner, sample_env())
    }

    // --- allowlist (§12.2/12.3) ---

    #[test]
    fn full_vector_match_admits_known_commands() {
        let g = gateway(std::sync::Arc::new(FakeRunner::default()));
        assert!(
            matches!(g.check("cargo check"), Check::Allowed { toolchain, .. } if toolchain == "rust")
        );
        assert!(matches!(
            g.check("cargo build --release"),
            Check::Allowed { .. }
        ));
    }

    #[test]
    fn extra_flags_are_rejected_not_prefix_matched() {
        let g = gateway(std::sync::Arc::new(FakeRunner::default()));
        // Classic escalation attempt: allowed program+subcommand, new flag.
        for evil in [
            "cargo check --config build.rs=hack",
            "cargo test -- --exact", // trailing args change the vector
            "cargo build --release --frozen",
            "cargo clippy --fix",
        ] {
            assert!(
                matches!(
                    &g.check(evil),
                    Check::Denied {
                        reason: DenyReason::NotAllowlisted { .. }
                    }
                ),
                "{evil} must be denied"
            );
        }
    }

    #[test]
    fn path_prefixes_and_separators_do_not_smuggle() {
        let g = gateway(std::sync::Arc::new(FakeRunner::default()));
        // Same words, extra slash/dot tricks: still different vectors.
        for evil in [
            "cargo  check",
            "cargo ./check",
            "cargo/check",
            "cargo check ",
        ] {
            // `cargo  check` normalizes to the allowed vector (whitespace fold)
            let r = g.check(evil);
            let ok_norm = evil == "cargo  check" || evil == "cargo check ";
            assert_eq!(matches!(r, Check::Allowed { .. }), ok_norm, "{evil}");
        }
    }

    #[test]
    fn denied_never_reaches_the_runner() {
        let fake = std::sync::Arc::new(FakeRunner::default());
        let g = gateway(fake.clone());
        assert!(g.run("cargo publish").is_err());
        assert!(g.run("rm -rf /").is_err());
        assert!(g.run("").is_err());
        assert_eq!(fake.seen.lock().unwrap().len(), 0);
    }

    // --- argv-fuzz (§22 Faz 3 exit criteria) ---

    /// Deterministic fuzz: random command lines built from shell-metacharacter
    /// alphabets must (a) never panic, (b) be admitted only when the normalized
    /// vector exactly equals an allowlist entry.
    #[test]
    fn fuzz_only_exact_allowlist_vectors_are_admitted() {
        // xorshift64* — deterministic, no external rng dependency.
        let mut s = 0x9E3779B97F4A7C15u64;
        let mut next = move || {
            s ^= s >> 12;
            s ^= s << 25;
            s ^= s >> 27;
            s.wrapping_mul(0x2545F4914F6CDD1D)
        };
        let alphabet = [
            "cargo",
            "check",
            "test",
            "--release",
            "--config",
            "build.rs=x",
            ";",
            "&&",
            "||",
            "|",
            "$(",
            "`",
            ">",
            "<",
            "\\",
            "..",
            "/",
            "--",
            "--exact",
            "publish",
            "rm",
            "-rf",
            "*",
            "~",
            "$HOME",
            "'",
            "\"",
            "%PATH%",
            "\n",
            " ",
            "\t",
            "curl",
        ];
        let g = gateway(std::sync::Arc::new(FakeRunner::default()));
        let allowed_sets: Vec<Vec<String>> =
            rust_spec().allowed.iter().map(|a| normalize(a)).collect();
        for _ in 0..2000 {
            let n = (next() % 6) as usize + 1;
            let cmd: String = (0..n)
                .map(|_| {
                    let i = (next() >> 3) as usize % alphabet.len();
                    alphabet[i]
                })
                .collect::<Vec<_>>()
                .join(" ");
            let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| g.check(&cmd)));
            let check = res.expect("fuzz must never panic");
            let argv = normalize(&cmd);
            let exact_allowed = allowed_sets.contains(&argv);
            assert_eq!(
                matches!(check, Check::Allowed { .. }),
                exact_allowed,
                "admission must equal exact-vector membership for: {cmd:?}"
            );
        }
    }

    // --- env scrubbing (§12.3) ---

    #[test]
    fn child_env_drops_secrets_keeps_path_home() {
        let e = scrub_env(&sample_env());
        let names: Vec<&str> = e.iter().map(|(k, _)| k.as_str()).collect();
        assert!(names.contains(&"PATH"));
        assert!(names.contains(&"HOME"));
        assert!(!names.contains(&"OPENROUTER_API_KEY"));
        assert!(!names.contains(&"GH_TOKEN"));
        assert!(!names.contains(&"MY_SECRET"));
    }

    #[test]
    fn run_scrubs_env_even_for_allowed_commands() {
        let fake = std::sync::Arc::new(FakeRunner::default());
        let g = gateway(fake.clone());
        g.run("cargo check").unwrap();
        let env = fake.seen_env.lock().unwrap()[0].clone();
        let names: Vec<&str> = env.iter().map(|(k, _)| k.as_str()).collect();
        assert!(!names.contains(&"OPENROUTER_API_KEY"));
        assert!(names.contains(&"PATH"));
    }

    // --- output cap + truncation (§12.3) ---

    #[test]
    fn output_cap_truncates_with_flag() {
        struct Big;
        impl CommandRunner for Big {
            fn run(
                &self,
                _a: &[String],
                _c: &Path,
                _e: &[(String, String)],
                _t: Duration,
            ) -> RunnerOutcome {
                RunnerOutcome {
                    exit_code: 0,
                    stdout: "x".repeat(5000),
                    stderr: String::new(),
                    timed_out: false,
                }
            }
        }
        let g = gateway(std::sync::Arc::new(Big));
        let out = g.run("cargo check").unwrap();
        assert!(out.truncated);
        assert_eq!(out.stdout.len(), 1024); // cap = 1 KiB
    }

    // --- preflight (§12.4) ---

    #[test]
    fn preflight_reports_missing_programs() {
        let mut m = BTreeMap::new();
        let mut spec = rust_spec();
        spec.allowed = vec![
            "this-tool-does-not-exist-ribozym test".into(),
            "cargo check".into(),
        ];
        m.insert("rust".to_string(), spec);
        let g = ToolGateway::new(
            m,
            PathBuf::from("."),
            std::sync::Arc::new(FakeRunner::default()),
            sample_env(),
        );
        let missing = g.preflight();
        assert!(missing
            .iter()
            .any(|p| p == "this-tool-does-not-exist-ribozym"));
        // cargo is on PATH here (tests run via cargo) — must not be missing.
        assert!(!missing.iter().any(|p| p == "cargo"));
    }

    // --- timeout plumbing (§12.3) ---

    /// Regression: the runner previously forgot `.stdout(piped())`, so a real
    /// child's output was silently dropped. Capture through the real
    /// SystemRunner must actually see bytes.
    #[test]
    fn system_runner_captures_real_output() {
        let mut env = BTreeMap::new();
        env.insert("PATH".into(), std::env::var("PATH").unwrap_or_default());
        if cfg!(windows) {
            env.insert(
                "SystemRoot".into(),
                std::env::var("SystemRoot").unwrap_or_default(),
            );
        }
        let argv: Vec<String> = ["cargo", "--version"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let out = SystemRunner.run(
            &argv,
            &PathBuf::from("."),
            &scrub_env(&env),
            Duration::from_secs(30),
        );
        assert_eq!(out.exit_code, 0, "cargo --version failed: {:?}", out.stderr);
        assert!(
            out.stdout.contains("cargo"),
            "stdout was dropped by the runner (piped stdio regression): {out:?}",
        );
    }

    #[test]
    fn system_runner_kills_on_timeout() {
        if !cfg!(windows) {
            // quick smoke: `sleep` is unix; keep Windows CI deterministic by
            // using ping -n below only on windows.
            return;
        }
        let mut env = BTreeMap::new();
        env.insert("PATH".into(), std::env::var("PATH").unwrap_or_default());
        let argv = vec!["ping".into(), "-n".into(), "30".into(), "127.0.0.1".into()];
        let out = SystemRunner.run(
            &argv,
            &PathBuf::from("."),
            &scrub_env(&env),
            Duration::from_millis(300),
        );
        assert!(out.timed_out);
    }
}
