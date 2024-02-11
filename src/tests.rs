use std::io::{BufRead, BufReader};

use regex::Regex;

use test_case::test_case;

use crate::vm::VirtualMachine;

const ASSIGNMENT_ASSOSIATIVIY: &'static str = include_str!("../tests/assignment/associativity.lox");
const ASSIGNMENT_GLOBAL: &'static str = include_str!("../tests/assignment/global.lox");

use pretty_assertions::assert_eq;
#[test_case(ASSIGNMENT_ASSOSIATIVIY)]
#[test_case(ASSIGNMENT_GLOBAL)]
pub fn test_assignment(code: &str) -> anyhow::Result<()> {
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
    let regex = Regex::new("// expect: ?(.*)").unwrap();
    for capture in regex.captures_iter(code) {
        if let Some(m) = capture.get(1) {
            //dbg!(m);
            result.push(m.as_str().into());
        }
    }
    result
}
