use std::{fmt::Display, io::Write};

use logos::Logos;
use thiserror::Error;
use tracing::debug;

use crate::{
    byte_code::ByteCode,
    parser::{parse, Ast, AstExpression, LogicalExpression, ParseError, Parser},
    value::Value,
};

#[repr(u8)]
pub enum OpCode {
    CONSTANT = 1,
    ADD,
    MULTIPLY,
    SUBTRACT,
    NEGATE,
    NOT,
    PRINT,
    TRUE,
    FALSE,
    NIL,
    GREATER,
    LESS,
    Unsupported,
}

impl OpCode {
    pub fn write_to(self, bytes: &mut ByteCode) {}
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

    fn set_bytecode(&mut self, byte_code: ByteCode) {
        self.byte_code = byte_code;
    }

    pub fn interpret(&mut self, code: &str) -> Result<(), VirtualMachineError> {
        let expression = parse(code)?;
        let mut byte_code = ByteCode::default();
        self.compile(&expression, &mut byte_code)?;
        self.set_bytecode(byte_code);
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

    fn must_be_number() -> VirtualMachineError {
        todo!()
    }

    fn must_be_bool() -> VirtualMachineError {
        todo!()
    }

    fn eval_add(left: Value, right: Value) -> Result<Value, VirtualMachineError> {
        Ok((left.as_number().ok_or_else(Self::must_be_number)?
            + right.as_number().ok_or_else(Self::must_be_number)?)
        .into())
    }

    fn eval_mult(left: Value, right: Value) -> Result<Value, VirtualMachineError> {
        Ok((left.as_number().ok_or_else(Self::must_be_number)?
            * right.as_number().ok_or_else(Self::must_be_number)?)
        .into())
    }

    fn eval_sub(left: Value, right: Value) -> Result<Value, VirtualMachineError> {
        Ok((left.as_number().ok_or_else(Self::must_be_number)?
            - right.as_number().ok_or_else(Self::must_be_number)?)
        .into())
    }

    fn eval_negate(val: Value) -> Result<Value, VirtualMachineError> {
        Ok((-val.as_number().ok_or_else(Self::must_be_number)?).into())
    }

    fn run(&mut self) -> Result<(), VirtualMachineError> {
        while let Ok(op_code) = self.read_byte() {
            match TryInto::<OpCode>::try_into(op_code)? {
                OpCode::CONSTANT => {
                    let idx = self.read_u16()?;
                    self.push(self.constants[idx as usize].clone());
                }
                OpCode::ADD => {
                    let left = self.pop()?;
                    let right = self.pop()?;
                    debug!("ADD {} {}", left, right);
                    self.push(Self::eval_add(left, right)?);
                }
                OpCode::MULTIPLY => {
                    let left = self.pop()?;
                    let right = self.pop()?;
                    debug!("MULTIPLY {} {}", left, right);
                    self.push(Self::eval_mult(left, right)?);
                }
                OpCode::SUBTRACT => {
                    let left = self.pop()?;
                    let right = self.pop()?;
                    debug!("SUBTRACT {} {}", left, right);
                    self.push(Self::eval_sub(left, right)?);
                }
                OpCode::NEGATE => {
                    let value = self.pop()?;
                    debug!("NEGATE {}", value);
                    self.push(Self::eval_negate(value)?);
                }
                OpCode::NOT => {
                    let value = self.pop()?.as_bool().ok_or_else(Self::must_be_bool)?;
                    debug!("NOT {}", value);
                    self.push((!value).into());
                }
                OpCode::PRINT => {
                    let param = self.pop()?;
                    println!("{}", param);
                }
                OpCode::TRUE => self.push(true.into()),
                OpCode::FALSE => self.push(false.into()),
                OpCode::NIL => self.push(Value::Nil),
                OpCode::Unsupported => return Err(VirtualMachineError::InvalidOpCode(op_code)),
                OpCode::GREATER => {
                    let left = self.pop()?.as_number().ok_or_else(Self::must_be_number)?;
                    let right = self.pop()?.as_number().ok_or_else(Self::must_be_number)?;
                    debug!("GREATER {} {}", left, right);
                    self.push((left > right).into());
                }
                OpCode::LESS => todo!(),
            }
        }

        Ok(())
    }

    pub fn read_byte(&mut self) -> Result<u8, VirtualMachineError> {
        if self.ip >= self.byte_code.size() {
            return Err(VirtualMachineError::UnexpectedEndOfByteCode);
        }
        let result = self.byte_code.get_byte(self.ip);
        self.ip += 1;
        Ok(result)
    }

    pub fn read_u16(&mut self) -> Result<u16, VirtualMachineError> {
        if self.ip + 1 >= self.byte_code.size() {
            return Err(VirtualMachineError::UnexpectedEndOfByteCode);
        }
        let result = self.byte_code.get_u16(self.ip);
        self.ip += 2;
        Ok(result)
    }

    pub fn compile(
        &mut self,
        expr: &AstExpression,
        bytes: &mut ByteCode,
    ) -> Result<(), VirtualMachineError> {
        match expr.node() {
            crate::parser::Expression::Literal(crate::parser::AstLiteral::NumberLiteral(num)) => {
                bytes.emit_const(self.constants.len(), expr.start());
                self.add_constant((*num).into());
            }
            crate::parser::Expression::Literal(crate::parser::AstLiteral::BoolLiteral(x)) => {
                if *x {
                    bytes.emit(OpCode::TRUE, expr.start());
                } else {
                    bytes.emit(OpCode::FALSE, expr.start());
                }
            }
            crate::parser::Expression::Literal(crate::parser::AstLiteral::NilLiteral) => {
                bytes.emit(OpCode::NIL, expr.start());
            }
            crate::parser::Expression::Multiply { left, right } => {
                self.compile(&left.as_ref(), bytes)?;
                self.compile(&right.as_ref(), bytes)?;
                bytes.emit(OpCode::MULTIPLY, left.start());
            }
            crate::parser::Expression::Divide { left, right } => todo!(),
            crate::parser::Expression::Add { left, right } => {
                self.compile(&left.as_ref(), bytes)?;
                self.compile(&right.as_ref(), bytes)?;
                bytes.emit(OpCode::ADD, left.start());
            }
            crate::parser::Expression::Subtract { left, right } => {}
            crate::parser::Expression::UnaryNegation { expr } => {
                self.compile(expr.as_ref(), bytes)?;
                bytes.emit(OpCode::NEGATE, expr.start());
            }
            crate::parser::Expression::Grouping { expr } => {
                self.compile(&expr, bytes)?;
            }
            crate::parser::Expression::UnaryNot { expr } => {
                self.compile(expr, bytes)?;
                bytes.emit(OpCode::NOT, expr.start());
            }
            crate::parser::Expression::Logical(LogicalExpression::Greater { left, right }) => {
                self.compile(&right, bytes)?;
                self.compile(&left, bytes)?;
                bytes.emit(OpCode::GREATER, left.start());
            }
            crate::parser::Expression::Logical(LogicalExpression::Less { left, right }) => {
                self.compile(&right, bytes)?;
                self.compile(&left, bytes)?;
                bytes.emit(OpCode::LESS, left.start());
            }
        }

        Ok(())
    }
}
