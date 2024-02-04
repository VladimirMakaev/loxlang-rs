use std::{fmt::Display, io::Write};

use thiserror::Error;
use tracing::debug;

use crate::parser::{parse, Ast, AstExpression, ParseError, Parser};

#[repr(u8)]
pub enum OpCode {
    CONSTANT = 1,
    ADD,
    MULTIPLY,
    SUBTRACT,
    NEGATE,
    PRINT,
    Unsupported,
}

impl OpCode {
    pub fn write_to(self, bytes: &mut ByteCode) {
        bytes.0.push(unsafe { std::mem::transmute(self) });
    }
}

#[derive(Error, Debug)]
pub enum VirtualMachineError {
    #[error("Compile error occured: source {source:?}")]
    CompileError {
        #[from]
        source: ParseError,
    },
    #[error("Invalid op code number '{0}'")]
    InvalidOpCode(u8),
    #[error("Unexpected end of byte code sequence detected")]
    UnexpectedEndOfByteCode,
    #[error("Expected operand on the stack but none found.")]
    MissingStackOperand,
    #[error("Unhandled error: {0:?}")]
    Unhandled(#[from] anyhow::Error),
}

#[derive(Default)]
pub struct ByteCode(Vec<u8>);

impl ByteCode {
    pub fn emit_const(&mut self, idx: usize) {
        self.0
            .push(unsafe { std::mem::transmute(OpCode::CONSTANT) });
        self.0.extend((idx as u16).to_ne_bytes())
    }

    pub fn emit(&mut self, op: OpCode) {
        op.write_to(self);
    }
}

#[derive(Clone, Copy)]
pub struct Value {
    val: f64,
}

impl Display for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.val)
    }
}

impl From<f64> for Value {
    fn from(value: f64) -> Self {
        Self { val: value }
    }
}

pub struct VirtualMachine {
    byte_code: ByteCode,
    ip: usize,
    stack: Vec<Value>,
    constants: Vec<Value>,
}

impl TryInto<OpCode> for u8 {
    type Error = VirtualMachineError;

    fn try_into(self) -> Result<OpCode, Self::Error> {
        if self > OpCode::Unsupported as u8 {
            Err(VirtualMachineError::InvalidOpCode(self))
        } else {
            Ok(unsafe { std::mem::transmute(self) })
        }
    }
}

impl VirtualMachine {
    pub fn new() -> Self {
        Self {
            byte_code: Default::default(),
            constants: Default::default(),
            ip: 0,
            stack: Default::default(),
        }
    }

    pub fn interpret(&mut self, code: &str) -> Result<(), VirtualMachineError> {
        let expression = parse(code)?;
        let mut byte_code = ByteCode::default();
        self.compile(&expression, &mut byte_code)?;
        _ = std::mem::replace(&mut self.byte_code, byte_code);
        self.run()
    }

    pub fn add_constant(&mut self, v: Value) {
        self.constants.push(v);
    }

    fn push(&mut self, v: Value) {
        debug!("on stack = {}", v);
        self.stack.push(v);
    }

    fn pop(&mut self) -> Result<Value, VirtualMachineError> {
        self.stack
            .pop()
            .ok_or_else(|| VirtualMachineError::MissingStackOperand)
    }

    pub fn run(&mut self) -> Result<(), VirtualMachineError> {
        while let Ok(op_code) = self.read_byte() {
            match TryInto::<OpCode>::try_into(op_code)? {
                OpCode::CONSTANT => {
                    let b1 = self.read_byte()?;
                    let b2 = self.read_byte()?;
                    let idx = u16::from_ne_bytes([b1, b2]);
                    self.push(self.constants[idx as usize]);
                }
                OpCode::ADD => {
                    let left = self.pop()?;
                    let right = self.pop()?;
                    debug!("ADD {} {}", left, right);
                    self.push((left.val + right.val).into());
                }
                OpCode::MULTIPLY => {
                    let left = self.pop()?;
                    let right = self.pop()?;
                    debug!("MULTIPLY {} {}", left, right);
                    self.push((left.val * right.val).into());
                }
                OpCode::SUBTRACT => {
                    let left = self.pop()?;
                    let right = self.pop()?;
                    debug!("SUBTRACT {} {}", left, right);
                    self.push((left.val - right.val).into());
                }
                OpCode::NEGATE => {
                    let left = self.pop()?;
                    debug!("NEGATE {}", left);
                    self.push((-left.val).into());
                }
                OpCode::PRINT => {
                    let param = self.pop()?;
                    println!("{}", param);
                }
                OpCode::Unsupported => return Err(VirtualMachineError::InvalidOpCode(op_code)),
            }
        }

        Ok(())
    }

    pub fn read_byte(&mut self) -> Result<u8, VirtualMachineError> {
        let result = self.byte_code.0.get(self.ip).map(ToOwned::to_owned);
        self.ip += 1;
        result.ok_or_else(|| VirtualMachineError::UnexpectedEndOfByteCode)
    }

    pub fn compile(
        &mut self,
        expr: &AstExpression,
        bytes: &mut ByteCode,
    ) -> Result<(), VirtualMachineError> {
        match expr.node() {
            crate::parser::Expression::Literal(crate::parser::AstLiteral::NumberLiteral(num)) => {
                bytes.emit_const(self.constants.len());
                self.add_constant(num.node.into());
            }
            crate::parser::Expression::Multiply { left, right } => {}
            crate::parser::Expression::Divide { left, right } => {}
            crate::parser::Expression::Add { left, right } => {
                self.compile(&left.as_ref(), bytes)?;
                self.compile(&right.as_ref(), bytes)?;
                bytes.emit(OpCode::ADD);
            }
            crate::parser::Expression::Subtract { left, right } => {}
            crate::parser::Expression::UnaryNegation { expr } => {
                self.compile(expr.as_ref(), bytes)?;
                bytes.emit(OpCode::NEGATE);
            }
            crate::parser::Expression::Grouping { expr } => {
                self.compile(&expr, bytes)?;
            }
        }

        Ok(())
    }
}
