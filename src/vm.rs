use std::{
    collections::hash_map::DefaultHasher,
    fmt::{Display, Formatter},
    hash::BuildHasherDefault,
    io::Write,
};

use anyhow::anyhow;
use hashbrown::HashMap;

use thiserror::Error;
use tracing::debug;

use crate::{
    byte_code::{ByteCode, JumpOffset, OpCode, OpCodeError, OpCodeTypes},
    interner::{DefaultInterner, Interner, Key},
    parser::{
        AstExpression, AstStmt, Expression, ForStmt, IfStmt, LogicalExpression, ParseError, Parser,
        StmtDeclaration, WhileStmt,
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

struct Function {
    name: usize,
    arity: usize,
    code: ByteCode,
}

struct CallFrame {
    function_idx: usize,
    ip: usize,
    locals_idx: usize,
}

impl CallFrame {
    fn inc_ip(&mut self, offset: usize) {
        self.ip += offset;
    }

    fn dec_ip(&mut self, offset: usize) {
        self.ip -= offset;
    }
}

pub struct VirtualMachine {
    frame_idx: usize,
    frames: Vec<CallFrame>,
    interner: DefaultInterner,
    stack: Vec<Value>,
    constants: Vec<Value>,
    functions: Vec<Function>,
    function_by_name: HashMap<usize, usize>,
    globals: HashMap<usize, Value>,
}

impl VirtualMachine {
    pub fn new() -> Self {
        Self {
            constants: Default::default(),
            stack: Default::default(),
            interner: Interner::new(BuildHasherDefault::<DefaultHasher>::default()),
            globals: Default::default(),
            frame_idx: 0,
            frames: Default::default(),
            functions: Default::default(),
            function_by_name: Default::default(),
        }
    }

    pub fn decompile<TOut: Write>(&self, stdout: &mut TOut) -> Result<(), VirtualMachineError> {
        for (name_idx, idx) in &self.function_by_name {
            write!(
                stdout,
                "{}:\n{}",
                self.interner.get_str(&Key { idx: *name_idx }),
                self.functions[self.constants[*idx]
                    .as_object()
                    .ok_or(VirtualMachineError::Unhandled(anyhow!("Expected function")))?
                    .object_id]
                    .code
                    .decompile(0)
            )
            .map_err(|e| VirtualMachineError::Unhandled(e.into()))?
        }

        Ok(())
    }

    pub fn compile(&mut self, code: &str) -> Result<(), VirtualMachineError> {
        let mut parser = Parser::new(code);
        let smts = parser.parse();
        if parser.had_errors() {
            return Err(VirtualMachineError::CompileError(parser.into_errors()));
        }

        self.define_function("<script>", 0)?;

        let mut context = LexicalContext {
            depth: 0,
            locals: Vec::new(),
            function_idx: 0,
            args: Default::default(),
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

    fn define_function(&mut self, name: &str, arity: usize) -> Result<(), VirtualMachineError> {
        let idx = self.interner.intern_str(name).idx;
        if self.function_by_name.contains_key(&idx) {
            Err(VirtualMachineError::Unhandled(anyhow!(
                "Function with name {} has already been declared",
                name
            )))
        } else {
            self.function_by_name.insert(idx, self.constants.len());
            self.constants.push(Value::fun(self.functions.len()));
            self.functions.push(Function {
                name: idx,
                arity: arity,
                code: ByteCode::default(),
            });
            Ok(())
        }
    }

    fn write_op(&mut self, function_idx: usize, op: OpCode, line: usize) {
        self.functions[function_idx].code.write_op(op, line);
    }

    pub fn next_instruction(&self, function_idx: usize) -> usize {
        self.functions[function_idx].code.size()
    }

    pub fn offset_since(&self, ip: usize, function_idx: usize) -> JumpOffset {
        JumpOffset::new(ip, self.next_instruction(function_idx))
    }

    fn next_op(&mut self) -> Option<Result<(OpCode, usize), OpCodeError>> {
        let frame = &self.frames[self.frame_idx];
        let function = &mut self.functions[frame.function_idx];
        function.code.read_next(frame.ip)
    }

    fn ip(&self) -> usize {
        self.frames[self.frame_idx].ip
    }

    pub fn run<TOut: Write>(&mut self, stdout: &mut TOut) -> Result<(), VirtualMachineError> {
        self.frames.push(CallFrame {
            function_idx: 0,
            ip: 0,
            locals_idx: 0,
        });

        while let Some(x) = self.next_op() {
            let (op_code, size) = x?;
            match op_code {
                OpCode::CONSTANT(idx) => {
                    let value = &self.constants[idx as usize].clone();
                    debug!("@{} CONSTANT {}", self.ip(), self.as_display(value.clone()));
                    self.push(value.clone());
                }
                OpCode::POP => {
                    let val = self.pop()?;
                    debug!("@{} POP {}", self.ip(), self.as_display(val));
                }
                OpCode::ADD => {
                    let right = self.pop()?;
                    let left = self.pop()?;

                    match (left, right) {
                        (Value::Number(left), Value::Number(right)) => {
                            self.push((left + right).into());
                            debug!("@{} ADD {} {}", self.ip(), left, right);
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
                        (left, right) => {
                            return Err(VirtualMachineError::Unhandled(anyhow!(
                                "ADD: unsupported types of operands: left: {}, right: {}",
                                self.as_display(left),
                                self.as_display(right)
                            )))
                        }
                    }
                }
                OpCode::MULTIPLY => {
                    let left = self.pop_number(OpCodeTypes::MULTIPLY)?;
                    let right = self.pop_number(OpCodeTypes::MULTIPLY)?;
                    debug!("@{} MULTIPLY {} {}", self.ip(), left, right);
                    self.push((left * right).into());
                }
                OpCode::SUBTRACT => {
                    let right = self.pop_number(OpCodeTypes::MULTIPLY)?;
                    let left = self.pop_number(OpCodeTypes::MULTIPLY)?;
                    debug!("@{} SUBTRACT {} {}", self.ip(), left, right);
                    self.push((left - right).into());
                }
                OpCode::NEGATE => {
                    let value = self.pop_number(OpCodeTypes::NEGATE)?;
                    debug!("@{} NEGATE {}", self.ip(), value);
                    self.push((-value).into());
                }
                OpCode::NOT => {
                    let value = self.pop()?;
                    debug!("@{} NOT {}", self.ip(), self.is_truthy(&value));
                    self.push(self.is_truthy(&value).into());
                }
                OpCode::PRINT => {
                    let value = self.pop()?;
                    debug!("@{} PRINT {}", self.ip(), self.as_display(value.clone()));
                    writeln!(stdout, "{}", self.as_display(value)).map_err(anyhow::Error::msg)?;
                }
                OpCode::TRUE => self.push(true.into()),
                OpCode::FALSE => self.push(false.into()),
                OpCode::NIL => self.push(Value::Nil),
                OpCode::GREATER => {
                    let right = self.pop_number(OpCodeTypes::GREATER)?;
                    let left = self.pop_number(OpCodeTypes::GREATER)?;
                    debug!("@{} GREATER {} {}", self.ip(), left, right);
                    self.push((left > right).into());
                }
                OpCode::LESS => {
                    let left = self.pop_number(OpCodeTypes::LESS)?;
                    let right = self.pop_number(OpCodeTypes::LESS)?;
                    debug!("@{} LESS {} {}", self.ip(), left, right);
                    self.push((left < right).into());
                }
                OpCode::EQUAL => {
                    let left = self.pop_number(OpCodeTypes::EQUAL)?;
                    let right = self.pop_number(OpCodeTypes::EQUAL)?;
                    debug!("@{} EQUAL {} {}", self.ip(), left, right);
                    self.push((left == right).into());
                }
                OpCode::DECLAREGLOBAL(name_idx) => {
                    let init_value = self.pop()?;
                    debug!(
                        "@{} DEFINE_GLOBAL {}={}",
                        self.ip(),
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
                        self.ip(),
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
                        self.ip(),
                        name,
                        self.as_display(new_value.clone())
                    );

                    self.globals.insert(name_idx as usize, new_value.clone());
                    self.push(new_value);
                }
                OpCode::GETLOCAL(stack_offset) => {
                    debug!("@{} GETLOCAL {}", self.ip(), stack_offset);
                    let slot = self.frames[self.frame_idx].locals_idx;
                    self.push(self.stack[slot + stack_offset as usize].clone());
                }
                OpCode::SETLOCAL(stack_offset) => {
                    let new_value = self.pop()?;
                    debug!(
                        "@{} SETLOCAL {}={}",
                        self.ip(),
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
                    debug!("@{} JUMP {}", self.ip(), offset);
                    self.frames[self.frame_idx].inc_ip(offset as usize);
                    continue;
                }
                OpCode::JUMPIFFALSE(offset) => {
                    let val = self.peek()?;

                    let val_bool = self.is_truthy(&val);

                    debug!("@{} JUMPIFFALSE {} cond = {}", self.ip(), offset, val_bool);

                    if !val_bool {
                        self.frames[self.frame_idx].inc_ip(offset as usize);
                        continue;
                    }
                }
                OpCode::LOOP(offset) => {
                    debug!("@{} LOOP {}", self.ip(), offset);
                    self.frames[self.frame_idx].dec_ip(offset as usize);
                    continue;
                }
                OpCode::CALL(arg_count) => {
                    let arg_count = arg_count as usize;
                    let function = &self.stack[self.stack.len() - arg_count - 1];
                    debug!(
                        "@{} CALL {} arity = {}",
                        self.ip(),
                        self.as_display(function.clone()),
                        arg_count
                    );
                    if let Value::Object(ObjectValue {
                        ty: ObjectType::Function,
                        object_id: function_idx,
                    }) = function
                    {
                        self.frames[self.frame_idx].inc_ip(size);
                        self.frames.push(CallFrame {
                            ip: 0,
                            function_idx: *function_idx,
                            locals_idx: self.stack.len() - arg_count - 1,
                        });
                        self.frame_idx += 1;
                        continue;
                    } else {
                        return Err(anyhow!(
                            "Expected function value on the stack, got {}",
                            self.as_display(function.to_owned())
                        )
                        .into());
                    }
                }
                OpCode::RET => {
                    let ret_value = self.pop()?;
                    debug!(
                        "@{} RET = {}",
                        self.ip(),
                        self.as_display(ret_value.clone())
                    );
                    let fun = &self.functions[self.frames[self.frame_idx].function_idx];
                    self.stack.truncate(self.stack.len() - fun.arity - 1);
                    self.frames.pop();
                    self.frame_idx -= 1;
                    self.stack.push(ret_value);
                    continue;
                }
            }
            self.frames[self.frame_idx].inc_ip(size);
        }

        Ok(())
    }

    pub fn patch_offset(&mut self, function_idx: usize, addr: usize, new_offset: JumpOffset) {
        self.functions[function_idx]
            .code
            .patch_offset(addr, new_offset);
    }

    pub fn compile_statement<'a>(
        &mut self,
        stmt: &'a AstStmt,
        context: &mut LexicalContext<'a>,
    ) -> Result<(), VirtualMachineError> {
        match stmt.node() {
            crate::parser::Stmt::Print(expr) => {
                self.compile_expr(expr, context)?;
                self.write_op(
                    context.function_idx,
                    OpCode::PRINT,
                    self.lookup_source_line(expr.start()),
                );
                Ok(())
            }
            crate::parser::Stmt::Expression(expr) => {
                self.compile_expr(expr, context)?;
                Ok(())
            }

            crate::parser::Stmt::Declarations(StmtDeclaration::Function { name, params, body }) => {
                let str_id = self.interner.intern_str(&name.node()).idx;
                if self.function_by_name.contains_key(&str_id) {
                    return Err(VirtualMachineError::Unhandled(anyhow!(
                        "Duplicate function {} is declared",
                        name.node()
                    )));
                }
                let fun_idx = context.function_idx;
                context.function(self.functions.len());
                self.define_function(&name.node(), params.len())?;
                context.push_args(["__current_fun__"]);
                context.push_args(params.iter().map(|x| x.node().as_str()));
                context.enter_block();
                self.compile_statement(&body, context)?;
                self.write_op(
                    context.function_idx,
                    OpCode::NIL,
                    self.lookup_source_line(body.end()),
                );
                self.write_op(
                    context.function_idx,
                    OpCode::RET,
                    self.lookup_source_line(body.end()),
                );
                context.leave_block();
                context.clear_args();
                context.function(fun_idx);
                Ok(())
            }

            crate::parser::Stmt::Declarations(StmtDeclaration::Variable { ident, expr }) => {
                if let Some(e) = expr {
                    self.compile_expr(e, context)?;
                } else {
                    self.write_op(
                        context.function_idx,
                        OpCode::NIL,
                        self.lookup_source_line(ident.start()),
                    );
                }
                if context.is_toplevel() {
                    let ident_idx = self.interner.intern_str(ident.node()).idx;
                    self.write_op(
                        context.function_idx,
                        OpCode::DECLAREGLOBAL((ident_idx) as u16),
                        self.lookup_source_line(ident.start()),
                    );
                } else {
                    let offset = context.new_local(ident.node());
                    self.write_op(
                        context.function_idx,
                        OpCode::SETLOCAL(offset as u16),
                        self.lookup_source_line(ident.start()),
                    )
                }
                Ok(())
            }
            crate::parser::Stmt::Block(block) => {
                context.enter_block();
                for each_stmt in block.0.iter() {
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
                let jump_to_else = self.next_instruction(context.function_idx); //

                self.write_op(
                    context.function_idx,
                    OpCode::JUMPIFFALSE(std::i16::MIN),
                    self.lookup_source_line(stmt.start()),
                );

                self.write_op(
                    context.function_idx,
                    OpCode::POP,
                    self.lookup_source_line(stmt.start()),
                );

                self.compile_statement(then_block, context)?;

                if let Some(else_block) = else_block {
                    let jump_after_then = self.functions[context.function_idx].code.size();

                    self.write_op(
                        context.function_idx,
                        OpCode::JUMP(std::i16::MIN),
                        self.lookup_source_line(stmt.start()),
                    );

                    self.patch_offset(
                        context.function_idx,
                        jump_to_else + 1,
                        self.offset_since(jump_to_else, context.function_idx),
                    );

                    self.write_op(
                        context.function_idx,
                        OpCode::POP,
                        self.lookup_source_line(stmt.start()),
                    );

                    self.compile_statement(else_block, context)?;

                    self.patch_offset(
                        context.function_idx,
                        jump_after_then + 1,
                        self.offset_since(jump_after_then, context.function_idx),
                    );
                } else {
                    self.patch_offset(
                        context.function_idx,
                        jump_to_else + 1,
                        self.offset_since(jump_to_else, context.function_idx),
                    );
                }

                Ok(())
            }
            crate::parser::Stmt::While(WhileStmt {
                condition,
                loop_block,
            }) => {
                let start_of_loop = self.next_instruction(context.function_idx);
                self.compile_expr(condition, context)?;
                let jump_out = self.next_instruction(context.function_idx);
                self.write_op(
                    context.function_idx,
                    OpCode::JUMPIFFALSE(i16::MAX),
                    self.lookup_source_line(condition.start()),
                );
                self.write_op(
                    context.function_idx,
                    OpCode::POP,
                    self.lookup_source_line(condition.start()),
                );
                self.compile_statement(loop_block, context)?;
                self.write_op(
                    context.function_idx,
                    OpCode::LOOP(
                        self.offset_since(start_of_loop, context.function_idx)
                            .into(),
                    ),
                    self.lookup_source_line(loop_block.end()),
                );
                self.write_op(
                    context.function_idx,
                    OpCode::POP,
                    self.lookup_source_line(loop_block.end()),
                );
                self.patch_offset(
                    context.function_idx,
                    jump_out + 1,
                    self.offset_since(jump_out, context.function_idx),
                );
                Ok(())
            }
            crate::parser::Stmt::For(ForStmt {
                condition,
                initializer,
                increment,
                block,
            }) => {
                if let Some(initializer) = initializer {
                    self.compile_statement(initializer, context)?;
                }

                let when_loop_starts = self.next_instruction(context.function_idx);
                let mut when_condition_fails = None;

                if let Some(condition) = condition {
                    self.compile_expr(condition, context)?;
                    when_condition_fails = Some(self.next_instruction(context.function_idx));
                    self.write_op(
                        context.function_idx,
                        OpCode::JUMPIFFALSE(i16::MAX),
                        self.lookup_source_line(condition.start()),
                    );
                    self.write_op(
                        context.function_idx,
                        OpCode::POP,
                        self.lookup_source_line(condition.start()),
                    );
                }

                self.compile_statement(block, context)?;

                if let Some(increment) = increment {
                    self.compile_statement(increment, context)?;
                }

                self.write_op(
                    context.function_idx,
                    OpCode::LOOP(
                        self.offset_since(when_loop_starts, context.function_idx)
                            .into(),
                    ),
                    self.lookup_source_line(block.end()),
                );

                if let Some(when_condition_fails) = when_condition_fails {
                    self.patch_offset(
                        context.function_idx,
                        when_condition_fails + 1,
                        self.offset_since(when_condition_fails, context.function_idx),
                    );
                }
                self.write_op(
                    context.function_idx,
                    OpCode::POP,
                    self.lookup_source_line(block.end()),
                );

                Ok(())
            }
            crate::parser::Stmt::Return(value) => {
                self.compile_expr(value, context)?;
                self.write_op(
                    context.function_idx,
                    OpCode::RET,
                    self.lookup_source_line(stmt.start()),
                );
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
                    if let Some(offset) = context.resolve_local(name.as_str()) {
                        self.write_op(
                            context.function_idx,
                            OpCode::SETLOCAL(offset as u16),
                            self.lookup_source_line(lvalue.start()),
                        )
                    } else {
                        let idx = self.interner.intern_str(&name).idx;
                        self.write_op(
                            context.function_idx,
                            OpCode::SETGLOBAL(idx as u16),
                            self.lookup_source_line(expr.start()),
                        );
                    }
                }
            }
            crate::parser::Expression::Identier(name) => {
                if let Some(offset) = context.resolve_local(name.as_str()) {
                    self.write_op(
                        context.function_idx,
                        OpCode::GETLOCAL(offset as u16),
                        self.lookup_source_line(expr.start()),
                    )
                } else {
                    let name_idx = self.interner.intern_str(name.as_str());

                    if let Some(const_idx) = self.function_by_name.get(&name_idx.idx) {
                        self.write_op(
                            context.function_idx,
                            OpCode::CONSTANT(*const_idx as u16),
                            self.lookup_source_line(expr.start()),
                        )
                    } else {
                        self.write_op(
                            context.function_idx,
                            OpCode::GETGLOBAL(name_idx.idx as u16),
                            self.lookup_source_line(expr.start()),
                        )
                    }
                }
            }
            crate::parser::Expression::Literal(crate::parser::AstLiteral::NumberLiteral(num)) => {
                self.write_op(
                    context.function_idx,
                    OpCode::CONSTANT(self.constants.len() as u16),
                    expr.start(),
                );
                self.add_constant((*num).into());
            }
            crate::parser::Expression::Literal(crate::parser::AstLiteral::StringLiteral(s)) => {
                let (val, _) = self.interned_str_value(s);
                self.write_op(
                    context.function_idx,
                    OpCode::CONSTANT(self.constants.len() as u16),
                    self.lookup_source_line(expr.start()),
                );
                self.add_constant(val);
            }
            crate::parser::Expression::Literal(crate::parser::AstLiteral::BoolLiteral(x)) => {
                if *x {
                    self.write_op(context.function_idx, OpCode::TRUE, expr.start());
                } else {
                    self.write_op(context.function_idx, OpCode::FALSE, expr.start());
                }
            }
            crate::parser::Expression::Literal(crate::parser::AstLiteral::NilLiteral) => {
                self.write_op(context.function_idx, OpCode::NIL, expr.start());
            }
            crate::parser::Expression::Multiply { left, right } => {
                self.compile_expr(&left.as_ref(), context)?;
                self.compile_expr(&right.as_ref(), context)?;
                self.write_op(context.function_idx, OpCode::MULTIPLY, left.start());
            }
            crate::parser::Expression::Divide { left: _, right: _ } => todo!(),
            crate::parser::Expression::Add { left, right } => {
                self.compile_expr(&left.as_ref(), context)?;
                self.compile_expr(&right.as_ref(), context)?;
                self.write_op(context.function_idx, OpCode::ADD, left.start());
            }
            crate::parser::Expression::Subtract { left, right } => {
                self.compile_expr(&left.as_ref(), context)?;
                self.compile_expr(&right.as_ref(), context)?;
                self.write_op(context.function_idx, OpCode::SUBTRACT, left.start());
            }
            crate::parser::Expression::UnaryNegation { expr } => {
                self.compile_expr(expr.as_ref(), context)?;
                self.write_op(context.function_idx, OpCode::NEGATE, expr.start());
            }
            crate::parser::Expression::Grouping { expr } => {
                self.compile_expr(&expr, context)?;
            }
            crate::parser::Expression::UnaryNot { expr } => {
                self.compile_expr(expr, context)?;
                self.write_op(context.function_idx, OpCode::NOT, expr.start());
            }
            crate::parser::Expression::Logical(LogicalExpression::Greater { left, right }) => {
                self.compile_expr(&left, context)?;
                self.compile_expr(&right, context)?;
                self.write_op(context.function_idx, OpCode::GREATER, left.start());
            }
            crate::parser::Expression::Logical(LogicalExpression::Less { left, right }) => {
                self.compile_expr(&right, context)?;
                self.compile_expr(&left, context)?;
                self.write_op(context.function_idx, OpCode::LESS, left.start());
            }
            crate::parser::Expression::Logical(LogicalExpression::Equal { left, right }) => {
                self.compile_expr(&right, context)?;
                self.compile_expr(&left, context)?;
                self.write_op(
                    context.function_idx,
                    OpCode::EQUAL,
                    self.lookup_source_line(left.start()),
                );
            }

            crate::parser::Expression::Logical(LogicalExpression::And { left, right }) => {
                self.compile_expr(&left, context)?;
                let pos_after_left = self.next_instruction(context.function_idx);
                self.write_op(
                    context.function_idx,
                    OpCode::JUMPIFFALSE(i16::MAX),
                    self.lookup_source_line(left.start()),
                );
                self.compile_expr(&right, context)?;
                self.patch_offset(
                    context.function_idx,
                    pos_after_left + 1,
                    self.offset_since(pos_after_left, context.function_idx),
                )
            }
            crate::parser::Expression::Logical(LogicalExpression::Or { left, right }) => {
                self.compile_expr(&left, context)?;

                self.write_op(
                    context.function_idx,
                    OpCode::JUMPIFFALSE(6), // JUMPIFFALSE + JUMP
                    self.lookup_source_line(left.start()),
                );
                let jump_out = self.next_instruction(context.function_idx);
                self.write_op(
                    context.function_idx,
                    OpCode::JUMP(i16::MAX),
                    self.lookup_source_line(left.start()),
                );
                self.compile_expr(&right, context)?;
                self.patch_offset(
                    context.function_idx,
                    jump_out + 1,
                    self.offset_since(jump_out, context.function_idx),
                );
            }
            crate::parser::Expression::Call { calee, arguments } => {
                self.compile_expr(&calee, context)?;
                for arg in arguments {
                    self.compile_expr(arg, context)?;
                }
                self.write_op(
                    context.function_idx,
                    OpCode::CALL(arguments.len() as u8),
                    self.lookup_source_line(calee.start()),
                )
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
    args: Vec<&'a str>,
    locals: Vec<(&'a str, usize)>,
    depth: usize,
    function_idx: usize,
}

impl<'a> LexicalContext<'a> {
    fn is_toplevel(&self) -> bool {
        self.depth == 0
    }

    fn push_args(&mut self, args: impl IntoIterator<Item = &'a str>) {
        self.args.extend(args);
    }

    fn clear_args(&mut self) {
        self.args.clear();
    }

    fn function(&mut self, value: usize) {
        self.function_idx = value;
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
        for (i, arg_name) in self.args.iter().enumerate() {
            if arg_name.eq(&name) {
                return Some(i);
            }
        }

        for i in (0..self.locals.len()).rev() {
            let (n, _) = self.locals[i];
            if n.eq(name) {
                return Some(self.args.len() + i);
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

            Value::Object(ObjectValue {
                ty: ObjectType::Function,
                object_id: fun_idx,
            }) => write!(
                f,
                "<{}>",
                self.vm.interner.get_str(&Key {
                    idx: self.vm.functions[fun_idx].name,
                }),
            ),
            _ => todo!(),
        }
    }
}

#[cfg(test)]
mod tests {

    use super::LexicalContext;
    use pretty_assertions::assert_eq;

    #[test]
    fn test_var_scope() {
        let mut scope = LexicalContext {
            depth: 0,
            locals: Vec::new(),
            function_idx: 0,
            args: Default::default(),
        };
        scope.new_local("x");
        scope.enter_block();
        scope.new_local("y");
        scope.new_local("x");
        assert_eq!(Some(2), scope.resolve_local("x"));
    }
}
