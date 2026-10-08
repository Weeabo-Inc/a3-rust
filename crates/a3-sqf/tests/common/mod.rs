//! Shared test host and helpers.
#![allow(dead_code)]

use std::collections::HashMap;

use a3_sqf::{Host, ScriptError, Value, Vm};

/// A host with a manual clock that records output and errors.
#[derive(Default)]
pub struct TestHost {
    pub time: f32,
    pub tick: f32,
    pub log: Vec<String>,
    pub chat: Vec<String>,
    pub errors: Vec<String>,
    pub files: HashMap<String, String>,
}

impl Host for TestHost {
    fn time(&self) -> f32 {
        self.time
    }
    fn tick_time(&self) -> f32 {
        self.tick
    }
    fn diag_log(&mut self, text: &str) {
        self.log.push(text.to_string());
    }
    fn system_chat(&mut self, text: &str) {
        self.chat.push(text.to_string());
    }
    fn report_error(&mut self, error: &ScriptError) {
        self.errors.push(error.report.clone());
    }
    fn load_file(&mut self, path: &str) -> Result<String, String> {
        self.files
            .get(&path.to_ascii_lowercase())
            .cloned()
            .ok_or_else(|| format!("Script {path} not found"))
    }
}

pub fn vm() -> Vm<TestHost> {
    Vm::new(TestHost::default())
}

/// Evaluates `src` in a fresh VM and returns its value, panicking on error.
pub fn eval(src: &str) -> Value {
    let mut vm = vm();
    match vm.eval(src) {
        Ok(v) => v,
        Err(e) => panic!("{src}\n{}", e.report),
    }
}

/// Evaluates `src` and returns `str` of the result.
pub fn s(src: &str) -> String {
    eval(src).to_sqf_string()
}

/// Evaluates `src`, expecting an error; returns the report.
pub fn err(src: &str) -> String {
    let mut vm = vm();
    match vm.eval(src) {
        Ok(v) => panic!("{src}: expected an error, got {v}"),
        Err(e) => e.report,
    }
}
