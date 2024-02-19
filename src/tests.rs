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

fn verify_code(code: &str) -> anyhow::Result<()> {
    let mut vm = VirtualMachine::new();
    vm.compile(code)?;
    let mut stdout = Vec::<u8>::new();
    vm.run(&mut stdout)?;

    assert_eq!(
        BufReader::new(stdout.as_slice())
            .lines()
            .collect::<Result<Vec<String>, std::io::Error>>()?,
        collect_stdout_expectations(code),
    );

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
