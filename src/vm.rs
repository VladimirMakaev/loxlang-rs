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
    byte_code::{ByteCode, JumpOffset, OpCode, OpCodeError, OpCodeTypes},
    interner::{DefaultInterner, Interner, Key},
    parser::{
        AstExpression, AstStmt, Expression, IfStmt, LogicalExpression, ParseError, Parser,
        StmtDeclaration,
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
        let mut context = LexicalContext {
            depth: 0,
            locals: Vec::new(),
        };
        for smt in smts.iter() {
            self.compile_statement(smt, &mut context)?;
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

    fn peek(&mut self) -> Result<Value, VirtualMachineError> {
        self.stack
            .last()
            .cloned()
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

    fn is_truthy(&self, value: &Value) -> bool {
        match value {
            Value::Number(_x) => true,
            Value::Bool(x) => *x,
            Value::Nil => false,
            Value::Object(_) => true,
        }
    }

    pub fn offset_since(&self, ip: usize) -> JumpOffset {
        JumpOffset::new(ip, self.byte_code.size())
    }

    pub fn run<TOut: Write>(&mut self, stdout: &mut TOut) -> Result<(), VirtualMachineError> {
        debug!("byte code = \n{}", self.byte_code.decompile(0));

        while let Some(x) = self.byte_code.read_next(self.ip) {
            let (op_code, size) = x?;
            match op_code {
                OpCode::CONSTANT(idx) => {
                    let value = &self.constants[idx as usize].clone();
                    self.push(value.clone());
                    debug!("@{} CONSTANT {}", self.ip, self.as_display(value.clone()));
                }
                OpCode::POP => {
                    let val = self.pop()?;
                    debug!("@{} POP {}", self.ip, self.as_display(val));
                }
                OpCode::ADD => {
                    let right = self.pop()?;
                    let left = self.pop()?;

                    match (left, right) {
                        (Value::Number(left), Value::Number(right)) => {
                            self.push((left + right).into());
                            debug!("@{} ADD {} {}", self.ip, left, right);
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
                    debug!("@{} MULTIPLY {} {}", self.ip, left, right);
                    self.push((left * right).into());
                }
                OpCode::SUBTRACT => {
                    let left = self.pop_number(OpCodeTypes::MULTIPLY)?;
                    let right = self.pop_number(OpCodeTypes::MULTIPLY)?;
                    debug!("@{} SUBTRACT {} {}", self.ip, left, right);
                    self.push((left - right).into());
                }
                OpCode::NEGATE => {
                    let value = self.pop_number(OpCodeTypes::NEGATE)?;
                    debug!("@{} NEGATE {}", self.ip, value);
                    self.push((-value).into());
                }
                OpCode::NOT => {
                    let value = self.pop()?;
                    debug!("@{} NOT {}", self.ip, self.is_truthy(&value));
                    self.push(self.is_truthy(&value).into());
                }
                OpCode::PRINT => {
                    let value = self.pop()?;
                    debug!("@{} PRINT {}", self.ip, self.as_display(value.clone()));
                    writeln!(stdout, "{}", self.as_display(value)).map_err(anyhow::Error::msg)?;
                }
                OpCode::TRUE => self.push(true.into()),
                OpCode::FALSE => self.push(false.into()),
                OpCode::NIL => self.push(Value::Nil),
                OpCode::GREATER => {
                    let left = self.pop_number(OpCodeTypes::GREATER)?;
                    let right = self.pop_number(OpCodeTypes::GREATER)?;
                    debug!("@{} GREATER {} {}", self.ip, left, right);
                    self.push((left > right).into());
                }
                OpCode::LESS => {
                    let left = self.pop_number(OpCodeTypes::LESS)?;
                    let right = self.pop_number(OpCodeTypes::LESS)?;
                    debug!("@{} LESS {} {}", self.ip, left, right);
                    self.push((left < right).into());
                }
                OpCode::EQUAL => {
                    let left = self.pop_number(OpCodeTypes::EQUAL)?;
                    let right = self.pop_number(OpCodeTypes::EQUAL)?;
                    debug!("@{} EQUAL {} {}", self.ip, left, right);
                    self.push((left == right).into());
                }
                OpCode::DECLAREGLOBAL(name_idx) => {
                    let init_value = self.pop()?;
                    debug!(
                        "@{} DEFINE_GLOBAL {}={}",
                        self.ip,
                        self.interner.get_str(&Key {
                            idx: name_idx as usize
                        }),
                        self.as_display(init_value.clone()),
                    );
                    self.globals.insert(name_idx as usize, init_value);
                }
                OpCode::GETGLOBAL(name_idx) => {
                    debug!(
                        "@{} GET_GLOBAL {}",
                        self.ip,
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
                    debug!(
                        "@{} SETGLOBAL {}={}",
                        self.ip,
                        name,
                        self.as_display(new_value.clone())
                    );

                    self.globals.insert(name_idx as usize, new_value.clone());
                    self.push(new_value);
                }
                OpCode::GETLOCAL(stack_offset) => {
                    debug!("@{} GETLOCAL {}", self.ip, stack_offset);
                    self.push(self.stack[stack_offset as usize].clone());
                }
                OpCode::SETLOCAL(stack_offset) => {
                    let new_value = self.pop()?;
                    debug!(
                        "@{} SETLOCAL {}={}",
                        self.ip,
                        stack_offset,
                        self.as_display(new_value.clone())
                    );
                    if self.stack.len() == stack_offset as usize {
                        self.push(new_value.clone());
                    } else {
                        self.stack[stack_offset as usize] = new_value.clone();
                        self.push(new_value);
                    }
                }
                OpCode::JUMP(offset) => {
                    debug!("@{} JUMP {}", self.ip, offset);
                    self.ip = ((self.ip as isize) + offset as isize) as usize;
                    continue;
                }
                OpCode::JUMPIFFALSE(offset) => {
                    let val = self.peek()?;

                    let val_bool = self.is_truthy(&val);

                    debug!("@{} JUMPIFFALSE {} cond = {}", self.ip, offset, val_bool);

                    if !val_bool {
                        self.ip = ((self.ip as isize) + offset as isize) as usize;
                        continue;
                    }
                }
            }
            self.ip += size;
        }

        Ok(())
    }

    pub fn compile_statement<'a>(
        &mut self,
        stmt: &'a AstStmt,
        context: &mut LexicalContext<'a>,
    ) -> Result<(), VirtualMachineError> {
        match stmt.node() {
            crate::parser::Stmt::Print(expr) => {
                self.compile_expr(expr, context)?;
                self.byte_code
                    .write_op(OpCode::PRINT, self.lookup_source_line(expr.start()));
                Ok(())
            }
            crate::parser::Stmt::Expression(expr) => {
                self.compile_expr(expr, context)?;
                Ok(())
            }
            crate::parser::Stmt::Declarations(StmtDeclaration::Variable { ident, expr }) => {
                if let Some(e) = expr {
                    self.compile_expr(e, context)?;
                } else {
                    self.byte_code
                        .write_op(OpCode::NIL, self.lookup_source_line(ident.start()));
                }
                if context.is_toplevel() {
                    let ident_idx = self.interner.intern_str(ident.node()).idx;
                    self.byte_code.write_op(
                        OpCode::DECLAREGLOBAL((ident_idx) as u16),
                        self.lookup_source_line(ident.start()),
                    );
                } else {
                    let offset = context.new_local(ident.node());
                    self.byte_code.write_op(
                        OpCode::SETLOCAL(offset as u16),
                        self.lookup_source_line(ident.start()),
                    )
                }
                Ok(())
            }
            crate::parser::Stmt::Block(block) => {
                context.enter_block();
                for each_stmt in block.iter() {
                    self.compile_statement(each_stmt, context)?;
                }
                context.leave_block();
                Ok(())
            }
            crate::parser::Stmt::If(IfStmt {
                condition,
                then_block,
                else_block,
            }) => {
                self.compile_expr(condition, context)?;
                let jump_to_else = self.byte_code.size(); //

                self.byte_code.write_op(
                    OpCode::JUMPIFFALSE(std::i16::MIN),
                    self.lookup_source_line(stmt.start()),
                );

                self.byte_code
                    .write_op(OpCode::POP, self.lookup_source_line(stmt.start()));

                self.compile_statement(then_block, context)?;

                if let Some(else_block) = else_block {
                    let jump_after_then = self.byte_code.size();

                    self.byte_code.write_op(
                        OpCode::JUMP(std::i16::MIN),
                        self.lookup_source_line(stmt.start()),
                    );

                    self.byte_code
                        .patch_offset(jump_to_else + 1, self.offset_since(jump_to_else));

                    self.byte_code
                        .write_op(OpCode::POP, self.lookup_source_line(stmt.start()));

                    self.compile_statement(else_block, context)?;

                    self.byte_code
                        .patch_offset(jump_after_then + 1, self.offset_since(jump_after_then));
                } else {
                    self.byte_code
                        .patch_offset(jump_to_else + 1, self.offset_since(jump_to_else));
                }

                Ok(())
            }
        }
    }

    pub fn compile_expr(
        &mut self,
        expr: &AstExpression,
        context: &mut LexicalContext,
    ) -> Result<(), VirtualMachineError> {
        match expr.node() {
            crate::parser::Expression::Assignment { lvalue, rvalue } => {
                if let Expression::Identier(name) = lvalue.node() {
                    self.compile_expr(rvalue, context)?;
                    if context.is_toplevel() {
                        let idx = self.interner.intern_str(&name).idx;
                        self.byte_code.write_op(
                            OpCode::SETGLOBAL(idx as u16),
                            self.lookup_source_line(expr.start()),
                        );
                    } else {
                        if let Some(offset) = context.resolve_local(name.as_str()) {
                            self.byte_code.write_op(
                                OpCode::SETLOCAL(offset as u16),
                                self.lookup_source_line(lvalue.start()),
                            )
                        }
                    }
                }
            }
            crate::parser::Expression::Identier(name) => {
                if let Some(offset) = context.resolve_local(name.as_str()) {
                    self.byte_code.write_op(
                        OpCode::GETLOCAL(offset as u16),
                        self.lookup_source_line(expr.start()),
                    )
                } else {
                    let name_idx = self.interner.intern_str(name.as_str());
                    self.byte_code.write_op(
                        OpCode::GETGLOBAL(name_idx.idx as u16),
                        self.lookup_source_line(expr.start()),
                    )
                }
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
                self.compile_expr(&left.as_ref(), context)?;
                self.compile_expr(&right.as_ref(), context)?;
                self.byte_code.write_op(OpCode::MULTIPLY, left.start());
            }
            crate::parser::Expression::Divide { left: _, right: _ } => todo!(),
            crate::parser::Expression::Add { left, right } => {
                self.compile_expr(&left.as_ref(), context)?;
                self.compile_expr(&right.as_ref(), context)?;
                self.byte_code.write_op(OpCode::ADD, left.start());
            }
            crate::parser::Expression::Subtract { left: _, right: _ } => {}
            crate::parser::Expression::UnaryNegation { expr } => {
                self.compile_expr(expr.as_ref(), context)?;
                self.byte_code.write_op(OpCode::NEGATE, expr.start());
            }
            crate::parser::Expression::Grouping { expr } => {
                self.compile_expr(&expr, context)?;
            }
            crate::parser::Expression::UnaryNot { expr } => {
                self.compile_expr(expr, context)?;
                self.byte_code.write_op(OpCode::NOT, expr.start());
            }
            crate::parser::Expression::Logical(LogicalExpression::Greater { left, right }) => {
                self.compile_expr(&right, context)?;
                self.compile_expr(&left, context)?;
                self.byte_code.write_op(OpCode::GREATER, left.start());
            }
            crate::parser::Expression::Logical(LogicalExpression::Less { left, right }) => {
                self.compile_expr(&right, context)?;
                self.compile_expr(&left, context)?;
                self.byte_code.write_op(OpCode::LESS, left.start());
            }
            crate::parser::Expression::Logical(LogicalExpression::Equal { left, right }) => {
                self.compile_expr(&right, context)?;
                self.compile_expr(&left, context)?;
                self.byte_code
                    .write_op(OpCode::EQUAL, self.lookup_source_line(left.start()));
            }
            crate::parser::Expression::Logical(LogicalExpression::And { left, right }) => {
                self.compile_expr(&left, context)?;
                let pos_after_left = self.byte_code.size();
                self.byte_code.write_op(
                    OpCode::JUMPIFFALSE(i16::MAX),
                    self.lookup_source_line(left.start()),
                );
                self.compile_expr(&right, context)?;
                self.byte_code
                    .patch_offset(pos_after_left + 1, self.offset_since(pos_after_left))
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

pub struct LexicalContext<'a> {
    locals: Vec<(&'a str, usize)>,
    depth: usize,
}

impl<'a> LexicalContext<'a> {
    fn is_toplevel(&self) -> bool {
        self.depth == 0
    }

    fn enter_block(&mut self) {
        self.depth += 1;
    }

    fn leave_block(&mut self) {
        self.depth -= 1;

        loop {
            if let Some((_, d)) = self.locals.last() {
                if *d > self.depth {
                    self.locals.pop();
                    continue;
                }
            }
            break;
        }
    }

    fn new_local(&mut self, name: &'a str) -> usize {
        self.locals.push((name, self.depth));
        self.locals.len() - 1
    }

    fn resolve_local(&self, name: &str) -> Option<usize> {
        for i in (0..self.locals.len()).rev() {
            let (n, _) = self.locals[i];
            if n.eq(name) {
                return Some(i);
            }
        }
        None
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

    use super::{LexicalContext, VirtualMachine};
    use pretty_assertions::assert_eq;

    const CODE_1: &str = r###"
    print 1 + 3;
    "###;
    const CODE_2: &str = r###"
    var x = 1;
    print(2 + x);
    "###;
    const CODE_3: &str = r###"
    var x = 1;
    {
        var y = 2;
    }"###;
    const CODE_4: &str = r###"
    if (1 == 1)
        print 1;
        "###;

    #[test_case(CODE_1,&[
        OpCode::CONSTANT(0),
        OpCode::CONSTANT(1),
        OpCode::ADD,
        OpCode::PRINT
    ])]
    #[test_case(CODE_2,&[
        OpCode::CONSTANT(0),
        OpCode::DECLAREGLOBAL(0),
        OpCode::CONSTANT(1),
        OpCode::GETGLOBAL(0),
        OpCode::ADD,
        OpCode::PRINT
    ])]
    #[test_case(CODE_3, &[
        OpCode::CONSTANT(0),
        OpCode::DECLAREGLOBAL(0),
        OpCode::CONSTANT(1),
        OpCode::SETLOCAL(0),
    ])]
    #[test_case(CODE_4, &[
        OpCode::CONSTANT(0),
        OpCode::CONSTANT(1),
        OpCode::EQUAL,
        OpCode::JUMPIFFALSE(8),
        OpCode::POP,
        OpCode::CONSTANT(2),
        OpCode::PRINT,
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

    #[test]
    fn test_var_scope() {
        let mut scope = LexicalContext {
            depth: 0,
            locals: Vec::new(),
        };
        scope.new_local("x");
        scope.enter_block();
        scope.new_local("y");
        scope.new_local("x");
        assert_eq!(Some(2), scope.resolve_local("x"));
    }
}
