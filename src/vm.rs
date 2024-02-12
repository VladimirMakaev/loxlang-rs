use std::{
    collections::hash_map::DefaultHasher,
    fmt::{Display, Formatter},
    hash::BuildHasherDefault,
    io::Write,
};

use hashbrown::HashMap;

use thiserror::Error;
use tracing::debug;

use crate::{
    byte_code::{ByteCode, OpCode, OpCodeError, OpCodeTypes},
    interner::{DefaultInterner, Interner, Key},
    parser::{
        AstExpression, AstStmt, Expression, LogicalExpression, ParseError, Parser, StmtDeclaration,
    },
    value::{ObjectType, ObjectValue, Value, ValueTypes},
};

#[derive(Error, Debug)]
pub enum VirtualMachineError {
    #[error("{0:?}")]
    CompileError(Vec<ParseError>),
    #[error("Opcode error: '{0}'")]
    OpCodeError(#[from] OpCodeError),
    #[error("Unexpected end of byte code sequence detected")]
    _UnexpectedEndOfByteCode,
    #[error("Expected operand on the stack but none found.")]
    MissingStackOperand,
    #[error("Undefined variable '{name}'.")]
    UndefinedVariable { name: String },
    #[error(
        "Instruction {instruction} expected stack operand of type '{expected}'. Got : '{actual}"
    )]
    UnexpectedStackOperandType {
        instruction: OpCodeTypes,
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
    globals: HashMap<usize, Value>,
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

    pub fn compile(&mut self, code: &str) -> Result<(), VirtualMachineError> {
        let mut parser = Parser::new(code);
        let smts = parser.parse();
        if parser.had_errors() {
            return Err(VirtualMachineError::CompileError(parser.into_errors()));
        }
        for smt in smts.iter() {
            self.compile_statement(smt)?;
        }
        Ok(())
    }

    fn add_constant(&mut self, v: Value) {
        self.constants.push(v);
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

    fn pop_number(&mut self, op_code: OpCodeTypes) -> Result<f64, VirtualMachineError> {
        let value = self.pop()?;
        Ok(value
            .as_number()
            .ok_or_else(|| VirtualMachineError::UnexpectedStackOperandType {
                instruction: op_code,
                expected: ValueTypes::Number,
                actual: value.into(),
            })?)
    }

    fn pop_bool(&mut self, op_code: OpCodeTypes) -> Result<bool, VirtualMachineError> {
        let value = self.pop()?;
        Ok(value
            .as_bool()
            .ok_or_else(|| VirtualMachineError::UnexpectedStackOperandType {
                instruction: op_code,
                expected: ValueTypes::Bool,
                actual: value.into(),
            })?)
    }

    pub fn run<TOut: Write>(&mut self, stdout: &mut TOut) -> Result<(), VirtualMachineError> {
        debug!("byte code = \n{}", self.byte_code.decompile(0));

        while let Some(x) = self.byte_code.read_next(self.ip) {
            let (op_code, size) = x?;
            self.ip += size;
            match op_code {
                OpCode::CONSTANT(idx) => {
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
                    let left = self.pop_number(OpCodeTypes::MULTIPLY)?;
                    let right = self.pop_number(OpCodeTypes::MULTIPLY)?;
                    debug!("MULTIPLY {} {}", left, right);
                    self.push((left * right).into());
                }
                OpCode::SUBTRACT => {
                    let left = self.pop_number(OpCodeTypes::MULTIPLY)?;
                    let right = self.pop_number(OpCodeTypes::MULTIPLY)?;
                    debug!("SUBTRACT {} {}", left, right);
                    self.push((left - right).into());
                }
                OpCode::NEGATE => {
                    let value = self.pop_number(OpCodeTypes::NEGATE)?;
                    debug!("NEGATE {}", value);
                    self.push((-value).into());
                }
                OpCode::NOT => {
                    let value = self.pop_bool(OpCodeTypes::NOT)?;
                    debug!("NOT {}", value);
                    self.push((!value).into());
                }
                OpCode::PRINT => {
                    let param = self.pop()?;
                    writeln!(stdout, "{}", self.as_display(param)).map_err(anyhow::Error::msg)?;
                }
                OpCode::TRUE => self.push(true.into()),
                OpCode::FALSE => self.push(false.into()),
                OpCode::NIL => self.push(Value::Nil),
                OpCode::GREATER => {
                    let left = self.pop_number(OpCodeTypes::GREATER)?;
                    let right = self.pop_number(OpCodeTypes::GREATER)?;
                    debug!("GREATER {} {}", left, right);
                    self.push((left > right).into());
                }
                OpCode::LESS => {
                    let left = self.pop_number(OpCodeTypes::LESS)?;
                    let right = self.pop_number(OpCodeTypes::LESS)?;
                    debug!("LESS {} {}", left, right);
                    self.push((left < right).into());
                }
                OpCode::DEFINEGLOBAL(name_idx) => {
                    let init_value = self.pop()?;
                    debug!(
                        "DEFINE_GLOBAL {}={}",
                        self.interner.get_str(&Key {
                            idx: name_idx as usize
                        }),
                        self.as_display(init_value.clone()),
                    );
                    self.globals.insert(name_idx as usize, init_value);
                }
                OpCode::GETGLOBAL(name_idx) => {
                    debug!(
                        "GET_GLOBAL {}",
                        self.interner.get_str(&Key {
                            idx: name_idx as usize
                        })
                    );
                    let value = self
                        .globals
                        .get(&(name_idx as usize))
                        .ok_or_else(|| VirtualMachineError::UndefinedVariable {
                            name: self
                                .interner
                                .get_str(&Key {
                                    idx: name_idx as usize,
                                })
                                .into(),
                        })?
                        .clone();
                    self.push(value);
                }
                OpCode::SETGLOBAL(name_idx) => {
                    let new_value = self.pop()?;
                    let name = self.interner.get_str(&Key {
                        idx: name_idx as usize,
                    });
                    if !self.globals.contains_key(&(name_idx as usize)) {
                        return Err(VirtualMachineError::UndefinedVariable { name: name.into() });
                    }
                    debug!("SET_GLOBAL {}={}", name, self.as_display(new_value.clone()));

                    self.globals.insert(name_idx as usize, new_value.clone());
                    self.push(new_value);
                }
            }
        }

        Ok(())
    }

    pub fn compile_statement(&mut self, stmt: &AstStmt) -> Result<(), VirtualMachineError> {
        match stmt.node() {
            crate::parser::Stmt::Print(expr) => {
                self.compile_expr(expr)?;
                self.byte_code
                    .write_op(OpCode::PRINT, self.lookup_source_line(expr.start()));
                Ok(())
            }
            crate::parser::Stmt::Expression(expr) => {
                self.compile_expr(expr)?;
                Ok(())
            }
            crate::parser::Stmt::Declarations(StmtDeclaration::Variable { ident, expr }) => {
                if let Some(e) = expr {
                    self.compile_expr(e)?;
                } else {
                    self.byte_code
                        .write_op(OpCode::NIL, self.lookup_source_line(ident.start()));
                }
                let ident_idx = self.interner.intern_str(ident.node()).idx;
                self.byte_code.write_op(
                    OpCode::DEFINEGLOBAL((ident_idx) as u16),
                    self.lookup_source_line(ident.start()),
                );
                Ok(())
            }
        }
    }

    pub fn compile_expr(&mut self, expr: &AstExpression) -> Result<(), VirtualMachineError> {
        match expr.node() {
            crate::parser::Expression::Assignment { lvalue, rvalue } => {
                if let Expression::Identier(name) = lvalue.node() {
                    let idx = self.interner.intern_str(&name).idx;
                    self.compile_expr(rvalue)?;
                    self.byte_code.write_op(
                        OpCode::SETGLOBAL(idx as u16),
                        self.lookup_source_line(expr.start()),
                    );
                }
            }
            crate::parser::Expression::Identier(name) => {
                let name_idx = self.interner.intern_str(name.as_str());
                self.byte_code.write_op(
                    OpCode::GETGLOBAL(name_idx.idx as u16),
                    self.lookup_source_line(expr.start()),
                )
            }
            crate::parser::Expression::Literal(crate::parser::AstLiteral::NumberLiteral(num)) => {
                self.byte_code
                    .write_op(OpCode::CONSTANT(self.constants.len() as u16), expr.start());
                self.add_constant((*num).into());
            }
            crate::parser::Expression::Literal(crate::parser::AstLiteral::StringLiteral(s)) => {
                let (val, _) = self.interned_str_value(s);
                self.byte_code.write_op(
                    OpCode::CONSTANT(self.constants.len() as u16),
                    self.lookup_source_line(expr.start()),
                );
                self.add_constant(val);
            }
            crate::parser::Expression::Literal(crate::parser::AstLiteral::BoolLiteral(x)) => {
                if *x {
                    self.byte_code.write_op(OpCode::TRUE, expr.start());
                } else {
                    self.byte_code.write_op(OpCode::FALSE, expr.start());
                }
            }
            crate::parser::Expression::Literal(crate::parser::AstLiteral::NilLiteral) => {
                self.byte_code.write_op(OpCode::NIL, expr.start());
            }
            crate::parser::Expression::Multiply { left, right } => {
                self.compile_expr(&left.as_ref())?;
                self.compile_expr(&right.as_ref())?;
                self.byte_code.write_op(OpCode::MULTIPLY, left.start());
            }
            crate::parser::Expression::Divide { left: _, right: _ } => todo!(),
            crate::parser::Expression::Add { left, right } => {
                self.compile_expr(&left.as_ref())?;
                self.compile_expr(&right.as_ref())?;
                self.byte_code.write_op(OpCode::ADD, left.start());
            }
            crate::parser::Expression::Subtract { left: _, right: _ } => {}
            crate::parser::Expression::UnaryNegation { expr } => {
                self.compile_expr(expr.as_ref())?;
                self.byte_code.write_op(OpCode::NEGATE, expr.start());
            }
            crate::parser::Expression::Grouping { expr } => {
                self.compile_expr(&expr)?;
            }
            crate::parser::Expression::UnaryNot { expr } => {
                self.compile_expr(expr)?;
                self.byte_code.write_op(OpCode::NOT, expr.start());
            }
            crate::parser::Expression::Logical(LogicalExpression::Greater { left, right }) => {
                self.compile_expr(&right)?;
                self.compile_expr(&left)?;
                self.byte_code.write_op(OpCode::GREATER, left.start());
            }
            crate::parser::Expression::Logical(LogicalExpression::Less { left, right }) => {
                self.compile_expr(&right)?;
                self.compile_expr(&left)?;
                self.byte_code.write_op(OpCode::LESS, left.start());
            }

            _ => todo!(),
        }

        Ok(())
    }

    fn interned_str_value(&mut self, val: &str) -> (Value, usize) {
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

#[cfg(test)]
mod tests {
    use test_case::test_case;

    use crate::byte_code::OpCode;

    use super::VirtualMachine;
    use pretty_assertions::assert_eq;

    const CODE_1: &str = r###"
    print 1 + 3;
    "###;
    const CODE_2: &str = r###"
    var x = 1;
    print(2 + x);
    "###;

    #[test_case(CODE_1,&[
        OpCode::CONSTANT(0),
        OpCode::CONSTANT(1),
        OpCode::ADD,
        OpCode::PRINT
    ])]
    #[test_case(CODE_2,&[
        OpCode::CONSTANT(0),
        OpCode::DEFINEGLOBAL(0),
        OpCode::CONSTANT(1),
        OpCode::GETGLOBAL(0),
        OpCode::ADD,
        OpCode::PRINT
    ])]
    fn compile_test1(code: &str, expected: &[OpCode]) -> anyhow::Result<()> {
        let mut vm = VirtualMachine::new();
        vm.compile(code)?;
        let mut decompiler = vm.byte_code.decompile(0);
        let mut all_instructions = Vec::new();

        while let Some(x) = decompiler.next() {
            all_instructions.push(x.unwrap());
        }

        assert_eq!(all_instructions.as_slice(), expected);

        Ok(())
    }
}
