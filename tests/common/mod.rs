//! Shared test utilities for the Lox test suite
//!
//! This module provides expectation parsing and verification utilities
//! for running the official Crafting Interpreters test suite.

use std::io::BufRead;
use std::path::Path;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use regex::Regex;

use loxlang_rs::codemap::Codemap;
use loxlang_rs::vm::{DisplayError, VirtualMachine};

/// Test execution timeout in seconds
const TEST_TIMEOUT_SECS: u64 = 2;

/// Represents a compile-time error expectation from a test file
#[derive(Debug, Clone)]
pub struct CompileError {
    pub line: usize,
    pub token: String,
    pub message: String,
}

/// Parsed expectations from a Lox test file
#[derive(Debug, Default)]
pub struct TestExpectations {
    /// Expected stdout lines (from `// expect: <output>` comments)
    pub stdout_lines: Vec<String>,
    /// Expected runtime error message (from `// expect runtime error: <message>`)
    pub runtime_error: Option<String>,
    /// Expected compile errors (from `// Error at '<token>': <message>`)
    pub compile_errors: Vec<CompileError>,
    /// Expected exit code (parsed from test file if specified)
    pub exit_code: Option<i32>,
    /// Whether this file should be skipped (has `// nontest` marker)
    pub is_nontest: bool,
}

/// Parse all expectations from a Lox source file
///
/// Parses the following comment patterns:
/// - `// expect: <output>` - expected stdout line
/// - `// expect runtime error: <message>` - expected runtime error
/// - `// Error at '<token>': <message>` - compile error on current line
/// - `// [line N] Error at '<token>': <message>` - compile error on specified line
/// - `// [c line N] Error at '<token>': <message>` - clox-specific error (we treat as normal)
/// - `// nontest` - skip this file
pub fn parse_expectations(code: &str) -> TestExpectations {
    let mut expectations = TestExpectations::default();

    // Check for nontest marker
    if code.contains("// nontest") {
        expectations.is_nontest = true;
        return expectations;
    }

    // Regex patterns for different expectation types
    let stdout_regex = Regex::new(r"//\s*expect:\s*(.*)").unwrap();
    let runtime_error_regex = Regex::new(r"//\s*expect runtime error:\s*(.+)").unwrap();

    // Compile error with explicit line number: // [line N] Error at 'token': message
    let explicit_line_error_regex =
        Regex::new(r"//\s*\[line (\d+)\]\s*Error at '([^']+)':\s*(.+)").unwrap();

    // Compile error at end: // [line N] Error at end: message
    let at_end_error_regex =
        Regex::new(r"//\s*\[line (\d+)\]\s*Error at end:\s*(.+)").unwrap();

    // Lexer error without token: // [line N] Error: message
    let lexer_error_regex =
        Regex::new(r"//\s*\[line (\d+)\]\s*Error:\s*(.+)").unwrap();

    // clox-specific error (treat same as explicit): // [c line N] Error at 'token': message
    let clox_error_regex =
        Regex::new(r"//\s*\[c line (\d+)\]\s*Error at '([^']+)':\s*(.+)").unwrap();

    // Compile error on current line: // Error at 'token': message
    let current_line_error_regex = Regex::new(r"//\s*Error at '([^']+)':\s*(.+)").unwrap();

    // Parse line by line
    let reader = std::io::BufReader::new(code.as_bytes());
    for (line_no_0idx, line_result) in reader.lines().enumerate() {
        let line = match line_result {
            Ok(l) => l,
            Err(_) => continue,
        };
        let line_no = line_no_0idx + 1; // Convert to 1-indexed

        // Check for stdout expectations
        if let Some(captures) = stdout_regex.captures(&line) {
            if let Some(m) = captures.get(1) {
                expectations.stdout_lines.push(m.as_str().to_string());
            }
        }

        // Check for runtime error expectation
        if let Some(captures) = runtime_error_regex.captures(&line) {
            if let Some(m) = captures.get(1) {
                expectations.runtime_error = Some(m.as_str().to_string());
            }
        }

        // Check for explicit line compile error: [line N] Error at 'token': message
        if let Some(captures) = explicit_line_error_regex.captures(&line) {
            let error_line: usize = captures.get(1).unwrap().as_str().parse().unwrap_or(line_no);
            let token = captures.get(2).unwrap().as_str().to_string();
            let message = captures.get(3).unwrap().as_str().to_string();
            expectations.compile_errors.push(CompileError {
                line: error_line,
                token,
                message,
            });
            continue; // Don't also match the current-line pattern
        }

        // Check for "Error at end" compile error: [line N] Error at end: message
        if let Some(captures) = at_end_error_regex.captures(&line) {
            let error_line: usize = captures.get(1).unwrap().as_str().parse().unwrap_or(line_no);
            let message = captures.get(2).unwrap().as_str().to_string();
            expectations.compile_errors.push(CompileError {
                line: error_line,
                token: "end".to_string(),
                message,
            });
            continue;
        }

        // Check for lexer error: [line N] Error: message (no token)
        if let Some(captures) = lexer_error_regex.captures(&line) {
            let error_line: usize = captures.get(1).unwrap().as_str().parse().unwrap_or(line_no);
            let message = captures.get(2).unwrap().as_str().to_string();
            expectations.compile_errors.push(CompileError {
                line: error_line,
                token: "".to_string(), // Empty token for lexer errors
                message,
            });
            continue;
        }

        // Check for clox-specific error: [c line N] Error at 'token': message
        if let Some(captures) = clox_error_regex.captures(&line) {
            let error_line: usize = captures.get(1).unwrap().as_str().parse().unwrap_or(line_no);
            let token = captures.get(2).unwrap().as_str().to_string();
            let message = captures.get(3).unwrap().as_str().to_string();
            expectations.compile_errors.push(CompileError {
                line: error_line,
                token,
                message,
            });
            continue; // Don't also match the current-line pattern
        }

        // Check for current-line compile error: Error at 'token': message
        if let Some(captures) = current_line_error_regex.captures(&line) {
            let token = captures.get(1).unwrap().as_str().to_string();
            let message = captures.get(2).unwrap().as_str().to_string();
            expectations.compile_errors.push(CompileError {
                line: line_no,
                token,
                message,
            });
        }
    }

    expectations
}

/// Verify a Lox test file against its expectations
///
/// Compiles and runs the code, comparing results against parsed expectations.
/// Returns Ok(()) if all expectations are met, or an error description.
/// Times out after TEST_TIMEOUT_SECS seconds.
pub fn verify_lox_file(code: &str, path: &Path) -> Result<(), String> {
    let code = code.to_string();
    let path = path.to_path_buf();

    let (tx, rx) = mpsc::channel();

    thread::spawn(move || {
        let result = verify_lox_file_inner(&code, &path);
        let _ = tx.send(result);
    });

    match rx.recv_timeout(Duration::from_secs(TEST_TIMEOUT_SECS)) {
        Ok(result) => result,
        Err(mpsc::RecvTimeoutError::Timeout) => {
            Err(format!("Test timed out after {} seconds", TEST_TIMEOUT_SECS))
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            Err("Test thread panicked".to_string())
        }
    }
}

/// Inner verification function that does the actual work
fn verify_lox_file_inner(code: &str, path: &Path) -> Result<(), String> {
    let expectations = parse_expectations(code);

    // Skip nontest files
    if expectations.is_nontest {
        return Ok(());
    }

    let codemap = Codemap::new(code);
    let mut vm = VirtualMachine::new();

    // Attempt compilation
    match vm.compile(code) {
        Ok(()) => {
            // Compilation succeeded - should not have expected compile errors
            if !expectations.compile_errors.is_empty() {
                return Err(format!(
                    "Expected {} compile error(s) but compilation succeeded.\nExpected errors:\n{}",
                    expectations.compile_errors.len(),
                    expectations
                        .compile_errors
                        .iter()
                        .map(|e| format!("  [line {}] Error at '{}': {}", e.line, e.token, e.message))
                        .collect::<Vec<_>>()
                        .join("\n")
                ));
            }

            // Run the code
            let mut stdout = Vec::<u8>::new();
            match vm.run(&mut stdout) {
                Ok(()) => {
                    // Execution succeeded - should not have expected runtime error
                    if let Some(expected_error) = &expectations.runtime_error {
                        return Err(format!(
                            "Expected runtime error '{}' but execution succeeded",
                            expected_error
                        ));
                    }

                    // Compare stdout
                    let actual_lines: Vec<String> = std::io::BufReader::new(stdout.as_slice())
                        .lines()
                        .filter_map(|l| l.ok())
                        .collect();

                    if actual_lines != expectations.stdout_lines {
                        return Err(format!(
                            "Output mismatch in {:?}\n\nExpected:\n{}\n\nActual:\n{}",
                            path,
                            expectations
                                .stdout_lines
                                .iter()
                                .map(|s| format!("  {}", s))
                                .collect::<Vec<_>>()
                                .join("\n"),
                            actual_lines
                                .iter()
                                .map(|s| format!("  {}", s))
                                .collect::<Vec<_>>()
                                .join("\n")
                        ));
                    }

                    Ok(())
                }
                Err(runtime_error) => {
                    // Execution failed with runtime error
                    let error_message = runtime_error.to_string();

                    if let Some(expected_error) = &expectations.runtime_error {
                        // Check if error message contains expected error
                        if error_message.contains(expected_error) {
                            Ok(())
                        } else {
                            Err(format!(
                                "Runtime error mismatch in {:?}\n\nExpected error containing:\n  {}\n\nActual error:\n  {}",
                                path, expected_error, error_message
                            ))
                        }
                    } else {
                        Err(format!(
                            "Unexpected runtime error in {:?}:\n{}",
                            path, error_message
                        ))
                    }
                }
            }
        }
        Err(compile_error) => {
            // Compilation failed
            if expectations.compile_errors.is_empty() && expectations.runtime_error.is_none() {
                // No errors expected but got compile error
                let error_display = DisplayError {
                    code,
                    err: &compile_error,
                    codemap: &codemap,
                };
                return Err(format!(
                    "Unexpected compile error in {:?}:\n{}",
                    path, error_display
                ));
            }

            // Format actual compile errors for comparison
            let error_display = DisplayError {
                code,
                err: &compile_error,
                codemap: &codemap,
            };
            let actual_error = error_display.to_string();

            // Check if expected compile errors match
            for expected in &expectations.compile_errors {
                let expected_pattern = if expected.token.is_empty() {
                    // Lexer error: "[line N] Error: message" format (no "at 'token'")
                    format!(
                        "[line {}] Error: {}",
                        expected.line, expected.message
                    )
                } else if expected.token == "end" {
                    // "Error at end:" format (no quotes around end)
                    format!(
                        "[line {}] Error at end: {}",
                        expected.line, expected.message
                    )
                } else {
                    // Normal "Error at 'token':" format
                    format!(
                        "[line {}] Error at '{}': {}",
                        expected.line, expected.token, expected.message
                    )
                };
                if !actual_error.contains(&expected_pattern) {
                    return Err(format!(
                        "Compile error mismatch in {:?}\n\nExpected error containing:\n  {}\n\nActual error:\n  {}",
                        path, expected_pattern, actual_error
                    ));
                }
            }

            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::parse_expectations;

    #[test]
    fn test_parse_stdout_expectations() {
        let code = r#"
print "hello"; // expect: hello
print "world"; // expect: world
"#;
        let expectations = parse_expectations(code);
        assert_eq!(expectations.stdout_lines, vec!["hello", "world"]);
        assert!(expectations.runtime_error.is_none());
        assert!(expectations.compile_errors.is_empty());
    }

    #[test]
    fn test_parse_runtime_error() {
        let code = r#"
nil + 1; // expect runtime error: Operands must be two numbers or two strings.
"#;
        let expectations = parse_expectations(code);
        assert!(expectations.stdout_lines.is_empty());
        assert_eq!(
            expectations.runtime_error,
            Some("Operands must be two numbers or two strings.".to_string())
        );
    }

    #[test]
    fn test_parse_compile_error_current_line() {
        let code = r#"
// Error at 'nil': Expect variable name.
var nil = "value";
"#;
        let expectations = parse_expectations(code);
        assert_eq!(expectations.compile_errors.len(), 1);
        assert_eq!(expectations.compile_errors[0].line, 2);
        assert_eq!(expectations.compile_errors[0].token, "nil");
        assert_eq!(
            expectations.compile_errors[0].message,
            "Expect variable name."
        );
    }

    #[test]
    fn test_parse_compile_error_explicit_line() {
        let code = r#"
// [line 3] Error at 'nil': Expect variable name.
var nil = "value";
"#;
        let expectations = parse_expectations(code);
        assert_eq!(expectations.compile_errors.len(), 1);
        assert_eq!(expectations.compile_errors[0].line, 3);
    }

    #[test]
    fn test_parse_nontest() {
        let code = r#"
// nontest
print "this should be skipped";
"#;
        let expectations = parse_expectations(code);
        assert!(expectations.is_nontest);
    }
}
