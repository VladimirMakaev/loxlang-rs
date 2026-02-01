//! Lox Test Suite - datatest-stable harness
//!
//! This test harness automatically discovers and runs all .lox test files
//! from the official Crafting Interpreters test suite.

use std::path::Path;

mod common;

/// Run a single Lox test file
///
/// Reads the file, parses expectations from comments, runs the code,
/// and verifies the output matches expectations.
fn run_lox_test(path: &Path) -> datatest_stable::Result<()> {
    let code = std::fs::read_to_string(path)?;

    // Check for nontest marker - skip these files silently
    if code.contains("// nontest") {
        return Ok(());
    }

    common::verify_lox_file(&code, path).map_err(|e| e.into())
}

datatest_stable::harness! {
    { test = run_lox_test, root = "tests/craftinginterpreters", pattern = r".*\.lox$" },
}
