use std::io::{BufRead, BufReader};

use regex::Regex;

use test_case::test_case;

use crate::vm::VirtualMachine;

const ASSIGNMENT_ASSOSIATIVIY: &'static str = include_str!("../tests/assignment/associativity.lox");
const ASSIGNMENT_GLOBAL: &'static str = include_str!("../tests/assignment/global.lox");
const ASSIGNMENT_LOCAL: &'static str = include_str!("../tests/assignment/local.lox");

use pretty_assertions::assert_eq;
#[test_case(ASSIGNMENT_ASSOSIATIVIY)]
#[test_case(ASSIGNMENT_GLOBAL)]
#[test_case(ASSIGNMENT_LOCAL)]
pub fn test_assignment(code: &str) -> anyhow::Result<()> {
    verify_code(code)
}

const CONTROL_FLOW_SIMPLE_IF: &'static str = include_str!("../tests/control_flow/simple_if.lox");
const CONTROL_FLOW_IF_ELSE: &'static str = include_str!("../tests/control_flow/if_else.lox");
const CONTROL_FLOW_AND_OR: &'static str = include_str!("../tests/control_flow/and_or.lox");

#[test_case(CONTROL_FLOW_SIMPLE_IF)]
#[test_case(CONTROL_FLOW_IF_ELSE)]
#[test_case(CONTROL_FLOW_AND_OR)]
pub fn test_control_flow(code: &str) -> anyhow::Result<()> {
    verify_code(code)
}

const BLOCK_EMPTY: &'static str = include_str!("../tests/block/empty.lox");

#[test_case(BLOCK_EMPTY)]
pub fn test_block(code: &str) -> anyhow::Result<()> {
    verify_code(code)
}

const WHILE_SYNTAX: &'static str = include_str!("../tests/while/syntax.lox");

#[test_case(WHILE_SYNTAX)]
pub fn test_while(code: &str) -> anyhow::Result<()> {
    verify_code(code)
}

const FOR_SYNTAX: &'static str = include_str!("../tests/for/syntax.lox");
const FOR_SCOPE: &'static str = include_str!("../tests/for/scope.lox");

#[test_case(FOR_SYNTAX)]
#[test_case(FOR_SCOPE)]
pub fn test_for(code: &str) -> anyhow::Result<()> {
    verify_code(code)
}

const TOPLEVEL_PRINT: &'static str = include_str!("../tests/toplevel/print.lox");
#[test_case(TOPLEVEL_PRINT)]
pub fn test_toplevel(code: &str) -> anyhow::Result<()> {
    verify_code(code)
}

const FUN_SYNTAX: &'static str = include_str!("../tests/fun/syntax.lox");
const FUN_MUTUAL_RECURSION: &'static str = include_str!("../tests/fun/mutual_recursion.lox");
const FUN_MUTUAL_RECURSION_LOCAL: &'static str =
    include_str!("../tests/fun/mutual_recursion_local.lox");
const FUN_SIMPLE_RECURSION: &'static str = include_str!("../tests/fun/simple_recursion.lox");
const FUN_WITH_LOCALS: &'static str = include_str!("../tests/fun/fun_with_locals.lox");

#[test_case(FUN_SYNTAX)]
#[test_case(FUN_MUTUAL_RECURSION)]
#[test_case(FUN_MUTUAL_RECURSION_LOCAL)]
#[test_case(FUN_SIMPLE_RECURSION)]
#[test_case(FUN_WITH_LOCALS)]
pub fn test_fun(code: &str) -> anyhow::Result<()> {
    verify_code(code)
}

const CLOSURE_SIMPLE: &str = include_str!("../tests/closure/simple.lox");
const CLOSURE_CLOSED_UPVALUES: &str = include_str!("../tests/closure/closed_upvalues.lox");

#[test_case(CLOSURE_SIMPLE)]
#[test_case(CLOSURE_CLOSED_UPVALUES)]
pub fn test_closure(code: &str) -> anyhow::Result<()> {
    verify_code(code)
}

fn verify_code(code: &str) -> anyhow::Result<()> {
    let mut vm = VirtualMachine::new();
    vm.compile(code)?;
    let mut stdout = Vec::<u8>::new();
    let res = vm.run(&mut stdout);

    match res {
        Ok(_) => {
            assert_eq!(
                BufReader::new(stdout.as_slice())
                    .lines()
                    .collect::<Result<Vec<String>, std::io::Error>>()?,
                collect_stdout_expectations(code),
            );
        }
        Err(e) => {
            let err_expectations = collect_stderr_expectations(code);
            if err_expectations.is_empty() {
                assert!(false, "Unexpected runtime error:\n\n{}", e);
            }
            let err_message = e.to_string();
            for expectation in err_expectations.into_iter() {
                assert!(
                    err_message.contains(&expectation),
                    "Got:\n------------------\n{err_message}\n-----------------\nExpected to contain:\n---------------\n{expectation}\n--------------------"
                );
            }
        }
    }

    Ok(())
}

fn collect_stdout_expectations(code: &str) -> Vec<String> {
    let mut result = Vec::new();
    let regex = Regex::new("//[ ]*expect: ?(.*)").unwrap();
    for capture in regex.captures_iter(code) {
        if let Some(m) = capture.get(1) {
            result.push(m.as_str().into());
        }
    }
    result
}

fn collect_stderr_expectations(code: &str) -> Vec<String> {
    let mut result = Vec::new();
    let regex = Regex::new("//[ ]*expect runtime error: (.+)").unwrap();
    for capture in regex.captures_iter(code) {
        if let Some(m) = capture.get(1) {
            result.push(m.as_str().into());
        }
    }
    result
}
