use std::collections::HashMap;
use std::sync::Mutex;

use anyhow::{anyhow, Result};
use async_trait::async_trait;

use super::{CommandOutput, Executor};

/// Test double. Returns scripted output per program and records every call.
#[derive(Default)]
pub struct FakeExecutor {
    scripted: Mutex<HashMap<String, CommandOutput>>,
    calls: Mutex<Vec<(String, Vec<String>)>>,
}

impl FakeExecutor {
    pub fn new() -> Self {
        Self::default()
    }

    /// Script the output returned for every invocation of `program`.
    pub fn expect(&self, program: &str, output: CommandOutput) {
        self.scripted
            .lock()
            .expect("scripted lock")
            .insert(program.to_string(), output);
    }

    /// Every (program, args) pair seen, in order.
    pub fn calls(&self) -> Vec<(String, Vec<String>)> {
        self.calls.lock().expect("calls lock").clone()
    }
}

#[async_trait]
impl Executor for FakeExecutor {
    async fn run(&self, program: &str, args: &[String]) -> Result<CommandOutput> {
        self.calls
            .lock()
            .expect("calls lock")
            .push((program.to_string(), args.to_vec()));

        self.scripted
            .lock()
            .expect("scripted lock")
            .get(program)
            .cloned()
            .ok_or_else(|| anyhow!("unexpected command: {program}"))
    }
}
