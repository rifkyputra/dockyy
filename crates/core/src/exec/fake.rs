use std::collections::HashMap;
use std::sync::Mutex;

use anyhow::{anyhow, Result};
use async_trait::async_trait;

use super::{CommandOutput, Executor};

/// Test double. Returns scripted output and records every call.
///
/// Two levels of scripting, checked in order:
///
/// 1. [`expect_call`](Self::expect_call) — matches an exact `(program, args)` pair.
/// 2. [`expect`](Self::expect) — matches any invocation of `program`.
///
/// The exact form exists because nearly every call the engine makes is
/// `systemctl <verb>`. With per-program scripting alone, every `systemctl` call
/// in a test returns the same output, which makes "daemon-reload succeeds but
/// start fails" — the shape every per-stage compensation test needs —
/// impossible to express.
#[derive(Default)]
pub struct FakeExecutor {
    by_call: Mutex<HashMap<(String, Vec<String>), CommandOutput>>,
    by_program: Mutex<HashMap<String, CommandOutput>>,
    calls: Mutex<Vec<(String, Vec<String>)>>,
    stdins: Mutex<Vec<String>>,
}

impl FakeExecutor {
    pub fn new() -> Self {
        Self::default()
    }

    /// Script the output returned for every invocation of `program`, whatever
    /// its arguments. Use [`expect_call`](Self::expect_call) when a test needs
    /// two invocations of the same program to behave differently.
    pub fn expect(&self, program: &str, output: CommandOutput) {
        self.by_program
            .lock()
            .expect("by_program lock")
            .insert(program.to_string(), output);
    }

    /// Script the output for one exact `(program, args)` pair. Takes precedence
    /// over [`expect`](Self::expect).
    pub fn expect_call(&self, program: &str, args: &[&str], output: CommandOutput) {
        let key = (
            program.to_string(),
            args.iter().map(|a| a.to_string()).collect(),
        );
        self.by_call
            .lock()
            .expect("by_call lock")
            .insert(key, output);
    }

    /// Every `(program, args)` pair seen, in order.
    pub fn calls(&self) -> Vec<(String, Vec<String>)> {
        self.calls.lock().expect("calls lock").clone()
    }

    fn record_call(&self, program: &str, args: &[String]) {
        self.calls
            .lock()
            .expect("calls lock")
            .push((program.to_string(), args.to_vec()));
    }

    fn scripted(&self, program: &str, args: &[String]) -> Result<CommandOutput> {
        let key = (program.to_string(), args.to_vec());
        if let Some(output) = self
            .by_call
            .lock()
            .expect("by_call lock")
            .get(&key)
            .cloned()
        {
            return Ok(output);
        }
        self.by_program
            .lock()
            .expect("by_program lock")
            .get(program)
            .cloned()
            .ok_or_else(|| anyhow!("unexpected command: {program} {}", args.join(" ")))
    }

    /// Stdin values received, in order. Deliberately separate from `calls()` so
    /// a secret value is never in the general call log.
    pub fn stdins(&self) -> Vec<String> {
        self.stdins.lock().expect("stdins lock").clone()
    }
}

#[async_trait]
impl Executor for FakeExecutor {
    async fn run(&self, program: &str, args: &[String]) -> Result<CommandOutput> {
        self.record_call(program, args);
        self.scripted(program, args)
    }

    async fn run_with_stdin(
        &self,
        program: &str,
        args: &[String],
        stdin: &str,
    ) -> Result<CommandOutput> {
        self.record_call(program, args);
        self.stdins
            .lock()
            .expect("stdins lock")
            .push(stdin.to_string());
        self.scripted(program, args)
    }
}
