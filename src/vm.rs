use std::{borrow::Cow, collections::HashMap, fmt::Display, io::Write, rc::Rc};

use logos::Logos;
use thiserror::Error;
use tracing::debug;

use crate::{
    byte_code::ByteCode,
    parser::{parse, Ast, AstExpression, LogicalExpression, ParseError, Parser},
    value::{ObjectRef, StringObject, Value, ValueTypes},
};

#[derive(strum::Display, Debug)]
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
    #[error(
        "Instruction {instruction} expected stack operand of type '{expected}'. Got : '{actual}"
    )]
    UnexpectedStackOperandType {
        instruction: OpCode,
        expected: ValueTypes,
        actual: ValueTypes,
    },
    #[error("Unhandled error: {0:?}")]
    Unhandled(#[from] anyhow::Error),
}

pub struct VirtualMachine<'vm> {
    byte_code: ByteCode,
    ip: usize,
    stack: Vec<Value>,
    constants: Vec<Value>,
    all_objects: Vec<ObjectRef>,
    all_strings: HashMap<&'vm str, usize>,
    all_strings2: HashMap<Rc<String>, usize>,
}

pub struct DS<'a> {
    all_strings: Vec<String>,
    by_name: HashMap<&'a str, usize>,
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

impl<'vm> VirtualMachine<'vm> {
    pub fn new() -> Self {
        Self {
            byte_code: Default::default(),
            constants: Default::default(),
            ip: 0,
            stack: Default::default(),
            all_objects: Default::default(),
            all_strings: Default::default(),
            all_strings2: Default::default(),
        }
    }

    fn set_bytecode(&mut self, byte_code: ByteCode) {
        self.byte_code = byte_code;
    }

    pub fn interpret(&mut self, code: &'vm str) -> Result<(), VirtualMachineError> {
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

    fn pop_number(&mut self, op_code: OpCode) -> Result<f64, VirtualMachineError> {
        let value = self.pop()?;
        Ok(value
            .as_number()
            .ok_or_else(|| VirtualMachineError::UnexpectedStackOperandType {
                instruction: op_code,
                expected: ValueTypes::Number,
                actual: value.into(),
            })?)
    }

    fn pop_bool(&mut self, op_code: OpCode) -> Result<bool, VirtualMachineError> {
        let value = self.pop()?;
        Ok(value
            .as_bool()
            .ok_or_else(|| VirtualMachineError::UnexpectedStackOperandType {
                instruction: op_code,
                expected: ValueTypes::Bool,
                actual: value.into(),
            })?)
    }

    fn get_or_create_string(&mut self, val: impl Into<Cow<'vm, str>>) -> Value {
        let str_val: Cow<'vm, str> = val.into();
        if let Some(object_id) = self.all_strings.get(str_val.as_ref()) {
            Value::Object(self.all_objects[*object_id].clone())
        } else {
            let object_id = self.all_objects.len();
            let str_val = Rc::new(str_val.into_owned());
            self.all_objects
                .push(StringObject::new_ref(object_id, str_val.clone()));
            self.all_strings2.insert(str_val, object_id);

            Value::Object(self.all_objects[object_id].clone())
        }
    }

    fn run(&mut self) -> Result<(), VirtualMachineError> {
        while let Ok(op_code) = self.read_byte() {
            match TryInto::<OpCode>::try_into(op_code)? {
                OpCode::CONSTANT => {
                    let idx = self.read_u16()?;
                    //self.push(self.constants[idx as usize].clone());
                }
                OpCode::ADD => {
                    let left = self.pop_number(OpCode::ADD)?;
                    let right = self.pop_number(OpCode::ADD)?;
                    debug!("ADD {} {}", left, right);
                    self.push((left + right).into());
                }
                OpCode::MULTIPLY => {
                    let left = self.pop_number(OpCode::MULTIPLY)?;
                    let right = self.pop_number(OpCode::MULTIPLY)?;
                    debug!("MULTIPLY {} {}", left, right);
                    self.push((left * right).into());
                }
                OpCode::SUBTRACT => {
                    let left = self.pop_number(OpCode::MULTIPLY)?;
                    let right = self.pop_number(OpCode::MULTIPLY)?;
                    debug!("SUBTRACT {} {}", left, right);
                    self.push((left - right).into());
                }
                OpCode::NEGATE => {
                    let value = self.pop_number(OpCode::NEGATE)?;
                    debug!("NEGATE {}", value);
                    self.push((-value).into());
                }
                OpCode::NOT => {
                    let value = self.pop_bool(OpCode::NOT)?;
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
                OpCode::GREATER => {
                    let left = self.pop_number(OpCode::GREATER)?;
                    let right = self.pop_number(OpCode::GREATER)?;
                    debug!("GREATER {} {}", left, right);
                    self.push((left > right).into());
                }
                OpCode::LESS => {
                    let left = self.pop_number(OpCode::LESS)?;
                    let right = self.pop_number(OpCode::LESS)?;
                    debug!("LESS {} {}", left, right);
                    self.push((left < right).into());
                }
                OpCode::Unsupported => return Err(VirtualMachineError::InvalidOpCode(op_code)),
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
