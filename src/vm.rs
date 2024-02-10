use std::{
    collections::hash_map::DefaultHasher,
    fmt::{Display, Formatter},
    hash::BuildHasherDefault,
    io::Write,
    process::id,
};

use hashbrown::HashMap;
use logos::Span;
use thiserror::Error;
use tracing::debug;

use crate::{
    byte_code::ByteCode,
    interner::{DefaultInterner, Interner, Key},
    parser::{parse, AstExpression, AstStmt, LogicalExpression, ParseError, StmtDeclaration},
    value::{ObjectType, ObjectValue, Value, ValueTypes},
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
    GET_GLOBAL,
    DEFINE_GLOBAL,
    Unsupported,
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
    #[error("Global variable {name} is not declared")]
    UndeclaredVariable { name: String },
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

pub struct VirtualMachine {
    byte_code: ByteCode,
    ip: usize,
    stack: Vec<Value>,
    constants: Vec<Value>,
    interner: DefaultInterner,
    globals: HashMap<usize, usize>,
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
            interner: Interner::new(BuildHasherDefault::<DefaultHasher>::default()),
            globals: Default::default(),
        }
    }

    fn set_bytecode(&mut self, byte_code: ByteCode) {
        self.byte_code = byte_code;
    }

    pub fn interpret(&mut self, code: &str) -> Result<(), VirtualMachineError> {
        let smts = parse(code)?;
        let mut byte_code = ByteCode::default();
        for smt in smts.iter() {
            self.compile_statement(smt, &mut byte_code)?;
        }
        self.set_bytecode(byte_code);
        self.run()
    }

    fn add_constant(&mut self, v: Value) {
        self.constants.push(v);
    }

    fn add_string(&mut self, val: String) {
        let key = self.interner.intern_string(val);
        self.stack.push(Value::Object(ObjectValue {
            ty: crate::value::ObjectType::String,
            object_id: key.idx,
        }));
    }

    fn push(&mut self, v: Value) {
        debug!("on stack = {}", self.as_display(v.clone()));
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

    fn get_or_create_string(&mut self, val: String) -> Value {
        // if let Some(object_id) = self.all_strings.get(str_val.as_ref()) {
        //     Value::Object(self.all_objects[*object_id].clone())
        // } else {
        //     let object_id = self.all_objects.len();
        //     let str_val = Rc::new(str_val.into_owned());
        //     self.all_objects
        //         .push(StringObject::new_ref(object_id, str_val.clone()));

        //     Value::Object(self.all_objects[object_id].clone())
        // }
        todo!()
    }

    fn run(&mut self) -> Result<(), VirtualMachineError> {
        while let Ok(op_code) = self.read_byte() {
            match TryInto::<OpCode>::try_into(op_code)? {
                OpCode::CONSTANT => {
                    let idx = self.read_u16()?;
                    self.push(self.constants[idx as usize].clone());
                }
                OpCode::ADD => {
                    let right = self.pop()?;
                    let left = self.pop()?;

                    match (left, right) {
                        (Value::Number(left), Value::Number(right)) => {
                            self.push((left + right).into());
                            debug!("ADD {} {}", left, right);
                        }
                        (
                            Value::Object(ObjectValue {
                                ty: ObjectType::String,
                                object_id: ob_id_1,
                            }),
                            Value::Object(ObjectValue {
                                ty: ObjectType::String,
                                object_id: ob_id_2,
                            }),
                        ) => {
                            let mut contatenate =
                                String::from(self.interner.get_str(&Key { idx: ob_id_1 }));
                            contatenate.push_str(self.interner.get_str(&Key { idx: ob_id_2 }));
                            let result = self.interner.intern_string(contatenate);
                            self.push(Value::Object(ObjectValue {
                                ty: ObjectType::String,
                                object_id: result.idx,
                            }));
                        }
                        (_, _) => todo!(),
                    }
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
                    println!("{}", self.as_display(param));
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
                OpCode::DEFINE_GLOBAL => {
                    todo!()
                }
                OpCode::GET_GLOBAL => {
                    todo!()
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

    pub fn compile_statement(
        &mut self,
        stmt: &AstStmt,
        bytes: &mut ByteCode,
    ) -> Result<(), VirtualMachineError> {
        match stmt.node() {
            crate::parser::Stmt::Print(expr) => {
                self.compile_expr(expr, bytes)?;
                bytes.emit(OpCode::PRINT, self.lookup_source_line(expr.start()));
                Ok(())
            }
            crate::parser::Stmt::Expression(expr) => {
                self.compile_expr(expr, bytes)?;
                Ok(())
            }
            crate::parser::Stmt::Declarations(StmtDeclaration::Variable { ident, expr }) => {
                if let Some(e) = expr {
                    self.compile_expr(e, bytes)?;
                } else {
                    bytes.emit(OpCode::NIL, self.lookup_source_line(ident.start()));
                }
                let (value, idx) = self.make_string_value(ident.node());
                self.add_constant(value);
                self.globals.insert(idx, self.constants.len() - 1);
                self.byte_code.emit_one_u16(
                    OpCode::DEFINE_GLOBAL,
                    (self.constants.len() - 1) as u16,
                    self.lookup_source_line(ident.start()),
                );
                Ok(())
            }
        }
    }

    pub fn compile_expr(
        &mut self,
        expr: &AstExpression,
        bytes: &mut ByteCode,
    ) -> Result<(), VirtualMachineError> {
        match expr.node() {
            crate::parser::Expression::Identier(name) => {
                let name_idx = self.interner.intern_str(name.as_str());
                let name_idx = self.globals.get(&name_idx.idx).map_or_else(
                    || Err(VirtualMachineError::UndeclaredVariable { name: name.into() }),
                    Ok,
                )?;

                self.byte_code.emit_one_u16(
                    OpCode::GET_GLOBAL,
                    *name_idx as u16,
                    self.lookup_source_line(expr.start()),
                )
            }
            crate::parser::Expression::Literal(crate::parser::AstLiteral::NumberLiteral(num)) => {
                bytes.emit_const(self.constants.len(), expr.start());
                self.add_constant((*num).into());
            }
            crate::parser::Expression::Literal(crate::parser::AstLiteral::StringLiteral(s)) => {
                self.add_string(s.clone());
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
                self.compile_expr(&left.as_ref(), bytes)?;
                self.compile_expr(&right.as_ref(), bytes)?;
                bytes.emit(OpCode::MULTIPLY, left.start());
            }
            crate::parser::Expression::Divide { left: _, right: _ } => todo!(),
            crate::parser::Expression::Add { left, right } => {
                self.compile_expr(&left.as_ref(), bytes)?;
                self.compile_expr(&right.as_ref(), bytes)?;
                bytes.emit(OpCode::ADD, left.start());
            }
            crate::parser::Expression::Subtract { left: _, right: _ } => {}
            crate::parser::Expression::UnaryNegation { expr } => {
                self.compile_expr(expr.as_ref(), bytes)?;
                bytes.emit(OpCode::NEGATE, expr.start());
            }
            crate::parser::Expression::Grouping { expr } => {
                self.compile_expr(&expr, bytes)?;
            }
            crate::parser::Expression::UnaryNot { expr } => {
                self.compile_expr(expr, bytes)?;
                bytes.emit(OpCode::NOT, expr.start());
            }
            crate::parser::Expression::Logical(LogicalExpression::Greater { left, right }) => {
                self.compile_expr(&right, bytes)?;
                self.compile_expr(&left, bytes)?;
                bytes.emit(OpCode::GREATER, left.start());
            }
            crate::parser::Expression::Logical(LogicalExpression::Less { left, right }) => {
                self.compile_expr(&right, bytes)?;
                self.compile_expr(&left, bytes)?;
                bytes.emit(OpCode::LESS, left.start());
            }

            _ => todo!(),
        }

        Ok(())
    }

    fn make_string_value(&mut self, val: &str) -> (Value, usize) {
        let idx = self.interner.intern_str(val).idx;
        (
            Value::Object(ObjectValue {
                ty: ObjectType::String,
                object_id: idx,
            }),
            idx,
        )
    }

    fn as_display(&self, value: Value) -> DispayValue {
        DispayValue { value, vm: self }
    }

    fn lookup_source_line(&self, start: usize) -> usize {
        return start; //todo lookup source table
    }
}

pub struct DispayValue<'a> {
    vm: &'a VirtualMachine,
    value: Value,
}

impl<'a> Display for DispayValue<'a> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self.value {
            Value::Number(x) => write!(f, "{}", x),
            Value::Bool(x) => write!(f, "{}", x),
            Value::Nil => f.write_str("nil"),
            Value::Object(ObjectValue {
                ty: ObjectType::String,
                object_id,
            }) => f.write_str(self.vm.interner.get_str(&Key { idx: object_id })),
            _ => todo!(),
        }
    }
}
