use std::{
    collections::hash_map::DefaultHasher,
    fmt::{Display, Formatter},
    hash::BuildHasherDefault,
    io::Write,
    mem,
};

use anyhow::anyhow;
use hashbrown::HashMap;

use thiserror::Error;
use tracing::debug;

use crate::{
    byte_code::{ByteCode, JumpOffset, OpCode, OpCodeError, OpCodeTypes},
    codemap::Codemap,
    interner::{DefaultInterner, Interner, StrId},
    parser::{
        AstExpression, AstStmt, Expression, ForStmt, IfStmt, LogicalExpression, ParseError, Parser,
        StmtDeclaration, WhileStmt,
    },
    value::{ObjectType, ObjectValue, Value, ValueTypes},
};

#[derive(Error, Debug)]
pub enum RuntimeErrorKind {
    #[error("Undefined variable '{name}'.")]
    UndefinedVariable { name: String },
    #[error("Can only call functions and classes.")]
    InvalidCallee,
    #[error("Expected {expected} arguments but got {got}.")]
    InvalidFunArity { got: usize, expected: usize },
}

#[derive(Error, Debug)]
pub enum VirtualMachineError {
    #[error("{0:?}")]
    CompileError(Vec<ParseError>),
    #[error("{kind}\n{stacktrace}")]
    RuntimeError {
        kind: RuntimeErrorKind,
        stacktrace: String,
    },
    #[error("Opcode error: '{0}'")]
    OpCodeError(#[from] OpCodeError),
    #[error("Unexpected end of byte code sequence detected")]
    _UnexpectedEndOfByteCode,
    #[error("Expected operand on the stack but none found.")]
    MissingStackOperand,

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
    name: StrId,
    arity: usize,
    code: ByteCode,
    upvalue_count: usize,
}

#[derive(Debug)]
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

    fn ip(&self) -> usize {
        self.ip
    }
}

pub struct VirtualMachine {
    frame_idx: usize,
    frames: Vec<CallFrame>,
    interner: DefaultInterner,
    stack: Vec<Value>,
    constants: Vec<Value>,
    functions: Vec<Function>,
    function_by_name: HashMap<StrId, usize>,
    globals: HashMap<StrId, Value>,
    codemap: Codemap,
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
            codemap: Codemap {
                line_endings: Default::default(),
                lines_start_at_1: true,
                position_starts_at_1: false,
            },
        }
    }

    pub fn decompile<TOut: Write>(&self, stdout: &mut TOut) -> Result<(), VirtualMachineError> {
        for (name_idx, function_idx) in &self.function_by_name {
            write!(
                stdout,
                "{}:\n{}",
                self.interner.get_str(*name_idx),
                self.functions[*function_idx].code.decompile(0)
            )
            .map_err(|e| VirtualMachineError::Unhandled(e.into()))?
        }

        Ok(())
    }

    pub fn compile(&mut self, code: &str) -> Result<(), VirtualMachineError> {
        self.codemap.index(code);
        let mut parser = Parser::new(code);
        let smts = parser.parse();
        if parser.had_errors() {
            return Err(VirtualMachineError::CompileError(parser.into_errors()));
        }

        let mut result = ByteCode::default();
        let mut context = LexicalScope::root();
        self.define_function("script", 0, 0, ByteCode::default())?;
        for smt in smts.iter() {
            self.compile_statement(smt, &mut context, &mut result)?;
        }
        self.functions[0].code = result;
        Ok(())
    }

    fn add_constant(&mut self, v: Value) {
        self.constants.push(v);
    }

    fn push(&mut self, v: Value) {
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
            Value::String(_) => true,
            Value::Function(_) => todo!(),
            Value::Closure(_) => todo!(),
        }
    }

    fn equal(&self, left: &Value, right: &Value) -> bool {
        match (left, right) {
            (Value::Number(l), Value::Number(r)) => l == r,
            (Value::Bool(l), Value::Bool(r)) => l == r,
            (Value::Nil, Value::Nil) => true,
            (Value::Object(l), Value::Object(r)) => match (l.ty, r.ty) {
                (ObjectType::Function, ObjectType::Function) => l.object_id == r.object_id,
                (ObjectType::_Class, ObjectType::_Class) => l.object_id == r.object_id,
                (_, _) => false,
            },
            (Value::String(l), Value::String(r)) => l == r,
            (_, _) => false,
        }
    }

    fn define_function(
        &mut self,
        name: &str,
        arity: usize,
        upvalue_count: usize,
        code: ByteCode,
    ) -> Result<(), VirtualMachineError> {
        let name_idx = self.interner.intern_str(name);
        if self.function_by_name.contains_key(&name_idx) {
            Err(VirtualMachineError::Unhandled(anyhow!(
                "Function with name {} has already been declared",
                name
            )))
        } else {
            self.function_by_name.insert(name_idx, self.functions.len());
            self.constants.push(Value::fun(self.functions.len()));
            self.functions.push(Function {
                name: name_idx,
                arity: arity,
                code,
                upvalue_count,
            });
            Ok(())
        }
    }

    pub fn next_instruction(&self, code: &ByteCode) -> usize {
        code.size()
    }

    pub fn offset_since(&self, ip: usize, code: &ByteCode) -> JumpOffset {
        JumpOffset::new(ip, self.next_instruction(code))
    }

    fn next_op(&mut self) -> Option<Result<(OpCode, usize), OpCodeError>> {
        let frame = &self.frames[self.frame_idx];
        let function = &mut self.functions[frame.function_idx];
        function.code.read_next(frame.ip)
    }

    fn ip(&self) -> usize {
        self.frames[self.frame_idx].ip
    }

    fn current_line(&self) -> usize {
        self.functions[self.frames[self.frame_idx].function_idx]
            .code
            .line(self.ip())
    }

    fn stacktrace_line(&self, frame: &CallFrame) -> String {
        let fun_idx = frame.function_idx;
        let fun = &self.functions[fun_idx];
        format!(
            "[line {}] in {}",
            fun.code.line(frame.ip()),
            self.interner.get_str(fun.name)
        )
    }

    fn stacktrace(&self) -> String {
        let mut result = String::new();
        for (f_idx, frame) in self.frames.iter().enumerate().rev() {
            if f_idx != self.frame_idx {
                result.push_str("\n");
            }
            result.push_str(&self.stacktrace_line(frame));
        }
        result
    }

    fn show_stack(&self, last_n: usize) -> String {
        self.stack
            .iter()
            .enumerate()
            .rev()
            .take(last_n)
            .map(|(i, x)| {
                if i == self.frames[self.frame_idx].locals_idx {
                    format!("{} <--", self.as_display(x.clone()))
                } else {
                    self.as_display(x.clone()).to_string()
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub fn run<TOut: Write>(&mut self, stdout: &mut TOut) -> Result<(), VirtualMachineError> {
        self.frames.push(CallFrame {
            function_idx: 0,
            ip: 0,
            locals_idx: 0,
        });

        while let Some(x) = self.next_op() {
            let (op_code, size) = x?;
            debug!("stack\n{}", self.show_stack(10));
            match op_code {
                OpCode::CONSTANT(idx) => {
                    let value = &self.constants[idx as usize].clone();
                    debug!(
                        "@{} CONSTANT {} [line: {}]",
                        self.ip(),
                        self.as_display(value.clone()),
                        self.current_line()
                    );
                    self.push(value.clone());
                }
                OpCode::POP => {
                    debug!(
                        "@{} POP {} [line: {}]",
                        self.ip(),
                        {
                            let val = self.peek().ok().clone();
                            self.as_display(val.unwrap_or(crate::vm::Value::Nil))
                        },
                        self.current_line()
                    );

                    self.pop()?;
                }
                OpCode::ADD => {
                    let right = self.pop()?;
                    let left = self.pop()?;

                    match (left, right) {
                        (Value::Number(left), Value::Number(right)) => {
                            self.push((left + right).into());
                            debug!(
                                "@{} ADD {} {} [line: {}]",
                                self.ip(),
                                left,
                                right,
                                self.current_line()
                            );
                        }
                        (Value::String(l), Value::String(r)) => {
                            let mut contatenate = String::from(self.interner.get_str(l));
                            contatenate.push_str(self.interner.get_str(r));
                            let result = self.interner.intern_string(contatenate);
                            self.push(Value::String(result));
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
                    debug!(
                        "@{} MULTIPLY {} {} [line: {}]",
                        self.ip(),
                        left,
                        right,
                        self.current_line()
                    );
                    self.push((left * right).into());
                }
                OpCode::SUBTRACT => {
                    let right = self.pop_number(OpCodeTypes::MULTIPLY)?;
                    let left = self.pop_number(OpCodeTypes::MULTIPLY)?;
                    debug!(
                        "@{} SUBTRACT {} {} [line: {}]",
                        self.ip(),
                        left,
                        right,
                        self.current_line()
                    );
                    self.push((left - right).into());
                }
                OpCode::NEGATE => {
                    let value = self.pop_number(OpCodeTypes::NEGATE)?;
                    debug!(
                        "@{} NEGATE {} [line: {}]",
                        self.ip(),
                        value,
                        self.current_line()
                    );
                    self.push((-value).into());
                }
                OpCode::NOT => {
                    let value = self.pop()?;
                    debug!(
                        "@{} NOT {} [line: {}]",
                        self.ip(),
                        self.is_truthy(&value),
                        self.current_line()
                    );
                    self.push((!self.is_truthy(&value)).into());
                }
                OpCode::PRINT => {
                    let value = self.pop()?;
                    debug!(
                        "@{} PRINT {} [line: {}]",
                        self.ip(),
                        self.as_display(value.clone()),
                        self.current_line()
                    );
                    writeln!(stdout, "{}", self.as_display(value)).map_err(anyhow::Error::msg)?;
                }
                OpCode::TRUE => self.push(true.into()),
                OpCode::FALSE => self.push(false.into()),
                OpCode::NIL => self.push(Value::Nil),
                OpCode::GREATER => {
                    let right = self.pop_number(OpCodeTypes::GREATER)?;
                    let left = self.pop_number(OpCodeTypes::GREATER)?;
                    debug!(
                        "@{} GREATER {} {} [line: {}]",
                        self.ip(),
                        left,
                        right,
                        self.current_line()
                    );
                    self.push((left > right).into());
                }
                OpCode::LESS => {
                    let left = self.pop_number(OpCodeTypes::LESS)?;
                    let right = self.pop_number(OpCodeTypes::LESS)?;
                    debug!(
                        "@{} LESS {} {} [line: {}]",
                        self.ip(),
                        left,
                        right,
                        self.current_line()
                    );
                    self.push((left < right).into());
                }
                OpCode::EQUAL => {
                    let right = self.pop()?;
                    let left = self.pop()?;

                    self.push(self.equal(&left, &right).into());

                    debug!(
                        "@{} EQUAL {} {} [line: {}]",
                        self.ip(),
                        self.as_display(left),
                        self.as_display(right),
                        self.current_line()
                    );
                }
                OpCode::DECLAREGLOBAL(name_idx) => {
                    let init_value = self.pop()?;
                    debug!(
                        "@{} DEFINE_GLOBAL {}={} [line: {}]",
                        self.ip(),
                        self.interner.get_str(name_idx),
                        self.as_display(init_value.clone()),
                        self.current_line()
                    );
                    self.globals.insert(name_idx, init_value);
                }
                OpCode::GETGLOBAL(name_idx) => {
                    debug!(
                        "@{} GET_GLOBAL {} [line: {}]",
                        self.ip(),
                        self.interner.get_str(name_idx),
                        self.current_line()
                    );
                    let value = self
                        .globals
                        .get(&name_idx)
                        .ok_or_else(|| VirtualMachineError::RuntimeError {
                            kind: RuntimeErrorKind::UndefinedVariable {
                                name: self.interner.get_str(name_idx).into(),
                            },
                            stacktrace: self.stacktrace(),
                        })?
                        .clone();
                    self.push(value);
                }
                OpCode::SETGLOBAL(name_idx) => {
                    let new_value = self.pop()?;
                    let name = self.interner.get_str(name_idx);
                    if !self.globals.contains_key(&name_idx) {
                        return Err(VirtualMachineError::RuntimeError {
                            kind: RuntimeErrorKind::UndefinedVariable { name: name.into() },
                            stacktrace: self.stacktrace(),
                        });
                    }
                    debug!(
                        "@{} SETGLOBAL {}={} [line: {}]",
                        self.ip(),
                        name,
                        self.as_display(new_value.clone()),
                        self.current_line()
                    );

                    self.globals.insert(name_idx, new_value.clone());
                    self.push(new_value);
                }
                OpCode::GETLOCAL(stack_offset) => {
                    let slot = self.frames[self.frame_idx].locals_idx;
                    let v = self.stack[slot + stack_offset as usize].clone();
                    debug!(
                        "@{} GETLOCAL {} [line: {}] = {}",
                        self.ip(),
                        stack_offset,
                        self.current_line(),
                        self.as_display(v.clone())
                    );
                    self.push(v);
                }
                OpCode::SETLOCAL(slot) => {
                    let new_value = self.pop()?;
                    debug!(
                        "@{} SETLOCAL {}={} [line: {}]",
                        self.ip(),
                        slot,
                        self.as_display(new_value.clone()),
                        self.current_line()
                    );
                    if self.stack.len() == self.frames[self.frame_idx].locals_idx + slot as usize {
                        self.push(new_value.clone());
                    } else {
                        self.stack[self.frames[self.frame_idx].locals_idx + slot as usize] =
                            new_value.clone();
                        self.push(new_value);
                    }
                }
                OpCode::JUMP(offset) => {
                    debug!(
                        "@{} JUMP {} [line: {}]",
                        self.ip(),
                        offset,
                        self.current_line()
                    );
                    self.frames[self.frame_idx].inc_ip(offset as usize);
                    continue;
                }
                OpCode::JUMPIFFALSE(offset) => {
                    let val = self.peek()?;

                    let val_bool = self.is_truthy(&val);

                    debug!(
                        "@{} JUMPIFFALSE {} cond = {} [line: {}]",
                        self.ip(),
                        offset,
                        val_bool,
                        self.current_line()
                    );

                    if !val_bool {
                        self.frames[self.frame_idx].inc_ip(offset as usize);
                        continue;
                    }
                }
                OpCode::LOOP(offset) => {
                    debug!(
                        "@{} LOOP {} [line: {}]",
                        self.ip(),
                        offset,
                        self.current_line()
                    );
                    self.frames[self.frame_idx].dec_ip(offset as usize);
                    continue;
                }
                OpCode::CALL(arg_count) => {
                    let arg_count = arg_count as usize;
                    let function = &self.stack[self.stack.len() - arg_count - 1];
                    debug!(
                        "@{} CALL {}({}) [line: {}]",
                        self.ip(),
                        self.as_display(function.clone()),
                        (0..arg_count)
                            .into_iter()
                            .map(|i| {
                                self.as_display(
                                    (&self.stack[self.stack.len() - arg_count + i]).clone(),
                                )
                            })
                            .map(|x| x.to_string())
                            .collect::<Vec<_>>()
                            .join(","),
                        self.current_line()
                    );
                    //debug!("Last 5 values on stack:\n{}", self.show_stack(5));
                    //debug!("frames:\n{:#?}", &self.frames);

                    if let Value::Object(ObjectValue {
                        ty: ObjectType::Function,
                        object_id: function_idx,
                    }) = function
                    {
                        let fun = &self.functions[*function_idx];
                        if fun.arity != arg_count {
                            return Err(VirtualMachineError::RuntimeError {
                                kind: RuntimeErrorKind::InvalidFunArity {
                                    got: arg_count,
                                    expected: fun.arity,
                                },
                                stacktrace: self.stacktrace(),
                            });
                        }
                        //self.frames[self.frame_idx].inc_ip(size);
                        self.frames.push(CallFrame {
                            ip: 0,
                            function_idx: *function_idx,
                            locals_idx: self.stack.len() - arg_count - 1,
                        });
                        self.frame_idx += 1;
                        //debug!("frames after call:\n{:#?}", &self.frames);
                        continue;
                    } else {
                        return Err(VirtualMachineError::RuntimeError {
                            kind: RuntimeErrorKind::InvalidCallee,
                            stacktrace: self.stacktrace(),
                        });
                    }
                }
                OpCode::RET => {
                    let ret_value = self.pop()?;
                    //debug!("Last 5 values on stack:\n{}", self.show_stack(5));
                    debug!(
                        "@{} RET = {} [line: {}]",
                        self.ip(),
                        self.as_display(ret_value.clone()),
                        self.current_line()
                    );
                    self.stack.truncate(self.frames[self.frame_idx].locals_idx);
                    self.frames.pop();
                    self.frame_idx -= 1;
                    self.stack.push(ret_value);
                    self.frames[self.frame_idx].inc_ip(2); // add sizeof call instruction

                    continue;
                }
            }
            self.frames[self.frame_idx].inc_ip(size);
        }

        Ok(())
    }

    pub fn patch_offset(&mut self, code: &mut ByteCode, addr: usize, new_offset: JumpOffset) {
        code.patch_offset(addr, new_offset);
    }

    pub fn compile_statement<'a>(
        &mut self,
        stmt: &'a AstStmt,
        context: &mut LexicalScope<'a>,
        result: &mut ByteCode,
    ) -> Result<(), VirtualMachineError> {
        match stmt.node() {
            crate::parser::Stmt::Print(expr) => {
                self.compile_expr(expr, context, result)?;
                result.write_op(OpCode::PRINT, self.lookup_source_line(expr.start()));
                Ok(())
            }
            crate::parser::Stmt::Expression(expr) => {
                self.compile_expr(expr, context, result)?;
                result.write_op(OpCode::POP, self.lookup_source_line(expr.end()));
                Ok(())
            }

            crate::parser::Stmt::Declarations(StmtDeclaration::Function { name, params, body }) => {
                let str_id = self.interner.intern_str(&name.node());
                if self.function_by_name.contains_key(&str_id) {
                    return Err(VirtualMachineError::Unhandled(anyhow!(
                        "Duplicate function {} is declared",
                        name.node()
                    )));
                }
                result.write_op(
                    OpCode::CONSTANT(self.constants.len() as u16),
                    self.lookup_source_line(name.start()),
                );

                let name_idx = self.interner.intern_str(&name.node());
                if context.is_toplevel() {
                    self.globals
                        .insert(name_idx, Value::fun(self.functions.len()));
                    result.write_op(
                        OpCode::DECLAREGLOBAL(name_idx),
                        self.lookup_source_line(name.start()),
                    )
                } else {
                    let offset = context.new_local(name.node());
                    result.write_op(
                        OpCode::SETLOCAL(offset as u16),
                        self.lookup_source_line(name.start()),
                    )
                }
                let mut byte_code = ByteCode::default();

                context.function_decl(
                    name.node(),
                    self.functions.len(),
                    params.iter().map(|x| x.node().as_str()),
                );
                self.define_function(&name.node(), params.len(), 0, Default::default())?;
                self.compile_statement(&body, context, &mut byte_code)?;
                byte_code.write_op(OpCode::NIL, self.lookup_source_line(body.end()));
                byte_code.write_op(OpCode::RET, self.lookup_source_line(body.end()));
                self.functions[context.function_idx].code = byte_code;
                context.leave_function();
                Ok(())
            }

            crate::parser::Stmt::Declarations(StmtDeclaration::Variable { ident, expr }) => {
                if let Some(e) = expr {
                    self.compile_expr(e, context, result)?;
                } else {
                    result.write_op(OpCode::NIL, self.lookup_source_line(ident.start()));
                }
                if context.is_toplevel() {
                    let ident_idx = self.interner.intern_str(ident.node());
                    result.write_op(
                        OpCode::DECLAREGLOBAL(ident_idx),
                        self.lookup_source_line(ident.start()),
                    );
                } else {
                    let offset = context.new_local(ident.node());
                    result.write_op(
                        OpCode::SETLOCAL(offset as u16),
                        self.lookup_source_line(ident.start()),
                    )
                }
                Ok(())
            }
            crate::parser::Stmt::Block(block) => {
                context.enter_block();
                for each_stmt in block.0.iter() {
                    self.compile_statement(each_stmt, context, result)?;
                }
                context.leave_block();
                Ok(())
            }
            crate::parser::Stmt::If(IfStmt {
                condition,
                then_block,
                else_block,
            }) => {
                self.compile_expr(condition, context, result)?;
                let jump_to_else = self.next_instruction(result); //

                result.write_op(
                    OpCode::JUMPIFFALSE(std::i16::MIN),
                    self.lookup_source_line(stmt.start()),
                );

                result.write_op(OpCode::POP, self.lookup_source_line(stmt.start()));

                self.compile_statement(then_block, context, result)?;
                let jump_after_then = result.size();
                result.write_op(
                    OpCode::JUMP(std::i16::MIN),
                    self.lookup_source_line(stmt.start()),
                );
                if let Some(else_block) = else_block {
                    self.patch_offset(
                        result,
                        jump_to_else + 1,
                        self.offset_since(jump_to_else, &result),
                    );

                    result.write_op(OpCode::POP, self.lookup_source_line(stmt.start()));

                    self.compile_statement(else_block, context, result)?;

                    self.patch_offset(
                        result,
                        jump_after_then + 1,
                        self.offset_since(jump_after_then, result),
                    );
                } else {
                    self.patch_offset(
                        result,
                        jump_to_else + 1,
                        self.offset_since(jump_to_else, result),
                    );
                    result.write_op(OpCode::POP, self.lookup_source_line(stmt.end()));
                    self.patch_offset(
                        result,
                        jump_after_then + 1,
                        self.offset_since(jump_after_then, result),
                    );
                }

                Ok(())
            }
            crate::parser::Stmt::While(WhileStmt {
                condition,
                loop_block,
            }) => {
                let start_of_loop = self.next_instruction(result);
                self.compile_expr(condition, context, result)?;
                let jump_out = self.next_instruction(result);
                result.write_op(
                    OpCode::JUMPIFFALSE(i16::MAX),
                    self.lookup_source_line(condition.start()),
                );
                result.write_op(OpCode::POP, self.lookup_source_line(condition.start()));
                self.compile_statement(loop_block, context, result)?;
                result.write_op(
                    OpCode::LOOP(self.offset_since(start_of_loop, result).into()),
                    self.lookup_source_line(loop_block.end()),
                );
                result.write_op(OpCode::POP, self.lookup_source_line(loop_block.end()));
                self.patch_offset(result, jump_out + 1, self.offset_since(jump_out, result));
                Ok(())
            }
            crate::parser::Stmt::For(ForStmt {
                condition,
                initializer,
                increment,
                block,
            }) => {
                if let Some(initializer) = initializer {
                    self.compile_statement(initializer, context, result)?;
                }

                let when_loop_starts = self.next_instruction(result);
                let mut when_condition_fails = None;

                if let Some(condition) = condition {
                    self.compile_expr(condition, context, result)?;
                    when_condition_fails = Some(self.next_instruction(result));
                    result.write_op(
                        OpCode::JUMPIFFALSE(i16::MAX),
                        self.lookup_source_line(condition.start()),
                    );
                    result.write_op(OpCode::POP, self.lookup_source_line(condition.start()));
                }

                self.compile_statement(block, context, result)?;

                if let Some(increment) = increment {
                    self.compile_statement(increment, context, result)?;
                }

                result.write_op(
                    OpCode::LOOP(self.offset_since(when_loop_starts, result).into()),
                    self.lookup_source_line(block.end()),
                );

                if let Some(when_condition_fails) = when_condition_fails {
                    self.patch_offset(
                        result,
                        when_condition_fails + 1,
                        self.offset_since(when_condition_fails, result),
                    );
                }
                result.write_op(OpCode::POP, self.lookup_source_line(block.end()));

                Ok(())
            }
            crate::parser::Stmt::Return(value) => {
                self.compile_expr(value, context, result)?;
                result.write_op(OpCode::RET, self.lookup_source_line(stmt.start()));
                Ok(())
            }
        }
    }

    pub fn compile_expr(
        &mut self,
        expr: &AstExpression,
        context: &mut LexicalScope,
        result: &mut ByteCode,
    ) -> Result<(), VirtualMachineError> {
        match expr.node() {
            crate::parser::Expression::Assignment { lvalue, rvalue } => {
                if let Expression::Identier(name) = lvalue.node() {
                    self.compile_expr(rvalue, context, result)?;
                    if let Some(offset) = context.resolve_local(name.as_str()) {
                        result.write_op(
                            OpCode::SETLOCAL(offset as u16),
                            self.lookup_source_line(lvalue.start()),
                        )
                    } else {
                        let idx = self.interner.intern_str(&name);
                        result.write_op(
                            OpCode::SETGLOBAL(idx),
                            self.lookup_source_line(expr.start()),
                        );
                    }
                }
            }
            crate::parser::Expression::Identier(name) => {
                if let Some(offset) = context.resolve_local(name.as_str()) {
                    result.write_op(
                        OpCode::GETLOCAL(offset as u16),
                        self.lookup_source_line(expr.start()),
                    )
                } else {
                    let name_idx = self.interner.intern_str(name.as_str());
                    result.write_op(
                        OpCode::GETGLOBAL(name_idx),
                        self.lookup_source_line(expr.start()),
                    )
                }
            }
            crate::parser::Expression::Literal(crate::parser::AstLiteral::NumberLiteral(num)) => {
                result.write_op(
                    OpCode::CONSTANT(self.constants.len() as u16),
                    self.lookup_source_line(expr.start()),
                );
                self.add_constant((*num).into());
            }
            crate::parser::Expression::Literal(crate::parser::AstLiteral::StringLiteral(s)) => {
                let val = Value::String(self.interner.intern_str(s));
                result.write_op(
                    OpCode::CONSTANT(self.constants.len() as u16),
                    self.lookup_source_line(expr.start()),
                );
                self.add_constant(val);
            }
            crate::parser::Expression::Literal(crate::parser::AstLiteral::BoolLiteral(x)) => {
                if *x {
                    result.write_op(OpCode::TRUE, self.lookup_source_line(expr.start()));
                } else {
                    result.write_op(OpCode::FALSE, self.lookup_source_line(expr.start()));
                }
            }
            crate::parser::Expression::Literal(crate::parser::AstLiteral::NilLiteral) => {
                result.write_op(OpCode::NIL, self.lookup_source_line(expr.start()));
            }
            crate::parser::Expression::Multiply { left, right } => {
                self.compile_expr(&left.as_ref(), context, result)?;
                self.compile_expr(&right.as_ref(), context, result)?;
                result.write_op(OpCode::MULTIPLY, self.lookup_source_line(left.start()));
            }
            crate::parser::Expression::Divide { left: _, right: _ } => todo!(),
            crate::parser::Expression::Add { left, right } => {
                self.compile_expr(&left.as_ref(), context, result)?;
                self.compile_expr(&right.as_ref(), context, result)?;
                result.write_op(OpCode::ADD, self.lookup_source_line(left.start()));
            }
            crate::parser::Expression::Subtract { left, right } => {
                self.compile_expr(&left.as_ref(), context, result)?;
                self.compile_expr(&right.as_ref(), context, result)?;
                result.write_op(OpCode::SUBTRACT, self.lookup_source_line(left.start()));
            }
            crate::parser::Expression::UnaryNegation { expr } => {
                self.compile_expr(expr.as_ref(), context, result)?;
                result.write_op(OpCode::NEGATE, self.lookup_source_line(expr.start()));
            }
            crate::parser::Expression::Grouping { expr } => {
                self.compile_expr(&expr, context, result)?;
            }
            crate::parser::Expression::UnaryNot { expr } => {
                self.compile_expr(expr, context, result)?;
                result.write_op(OpCode::NOT, self.lookup_source_line(expr.start()));
            }
            crate::parser::Expression::Logical(LogicalExpression::Greater { left, right }) => {
                self.compile_expr(&left, context, result)?;
                self.compile_expr(&right, context, result)?;
                result.write_op(OpCode::GREATER, self.lookup_source_line(left.start()));
            }
            crate::parser::Expression::Logical(LogicalExpression::Less { left, right }) => {
                self.compile_expr(&right, context, result)?;
                self.compile_expr(&left, context, result)?;
                result.write_op(OpCode::LESS, self.lookup_source_line(left.start()));
            }
            crate::parser::Expression::Logical(LogicalExpression::Equal { left, right }) => {
                self.compile_expr(&left, context, result)?;
                self.compile_expr(&right, context, result)?;
                result.write_op(OpCode::EQUAL, self.lookup_source_line(left.start()));
            }
            crate::parser::Expression::Logical(LogicalExpression::NotEqual { left, right }) => {
                self.compile_expr(&left, context, result)?;
                self.compile_expr(&right, context, result)?;
                result.write_op(OpCode::EQUAL, self.lookup_source_line(left.start()));
                result.write_op(OpCode::NOT, self.lookup_source_line(left.start()));
            }

            crate::parser::Expression::Logical(LogicalExpression::And { left, right }) => {
                self.compile_expr(&left, context, result)?;
                let pos_after_left = self.next_instruction(result);
                result.write_op(
                    OpCode::JUMPIFFALSE(i16::MAX),
                    self.lookup_source_line(left.start()),
                );
                self.compile_expr(&right, context, result)?;
                self.patch_offset(
                    result,
                    pos_after_left + 1,
                    self.offset_since(pos_after_left, result),
                )
            }
            crate::parser::Expression::Logical(LogicalExpression::Or { left, right }) => {
                self.compile_expr(&left, context, result)?;

                result.write_op(
                    OpCode::JUMPIFFALSE(6), // JUMPIFFALSE + JUMP
                    self.lookup_source_line(left.start()),
                );
                let jump_out = self.next_instruction(result);
                result.write_op(
                    OpCode::JUMP(i16::MAX),
                    self.lookup_source_line(left.start()),
                );
                self.compile_expr(&right, context, result)?;
                self.patch_offset(result, jump_out + 1, self.offset_since(jump_out, result));
            }
            crate::parser::Expression::Call { calee, arguments } => {
                self.compile_expr(&calee, context, result)?;
                for arg in arguments {
                    self.compile_expr(arg, context, result)?;
                }
                result.write_op(
                    OpCode::CALL(arguments.len() as u8),
                    self.lookup_source_line(calee.start()),
                )
            }
        }

        Ok(())
    }

    fn as_display(&self, value: Value) -> DispayValue {
        DispayValue { value, vm: self }
    }

    fn lookup_source_line(&self, start: usize) -> usize {
        self.codemap.line_at(start).unwrap_or(0)
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum UpvalueRef<'a> {
    Local(&'a str, usize),
    Upvalue(&'a str, usize),
}

impl<'a> UpvalueRef<'a> {
    pub fn name(&self) -> &'a str {
        match self {
            UpvalueRef::Local(name, _) => name,
            UpvalueRef::Upvalue(name, _) => name,
        }
    }
}

pub struct LexicalScope<'a> {
    parent: Option<Box<LexicalScope<'a>>>,
    //closure_scope: Option<Box<LexicalScope<'a>>>,
    args: Vec<&'a str>,
    locals: Vec<(&'a str, usize)>,
    upvalues: Vec<UpvalueRef<'a>>,
    block_depth: usize,
    function_idx: usize,
}

impl<'a> LexicalScope<'a> {
    fn is_toplevel(&self) -> bool {
        self.block_depth == 0
    }

    pub fn root() -> Self {
        Self {
            parent: None,
            args: Default::default(),
            locals: Default::default(),
            block_depth: 0,
            function_idx: 0,
            upvalues: Default::default(),
        }
    }

    pub fn function_decl(
        &mut self,
        name: &'a str,
        function_idx: usize,
        args: impl IntoIterator<Item = &'a str>,
    ) {
        let new = Self {
            parent: None,
            args: {
                let mut a = Vec::new();
                a.push(name);
                a.extend(args);
                a
            },
            block_depth: self.block_depth,
            function_idx: function_idx,
            locals: Default::default(),
            upvalues: Vec::with_capacity(self.locals.len() + self.upvalues.len()),
        };

        //for (i, local) in self.new_local(name)

        let prev = mem::replace(self, new);
        self.parent = Some(Box::new(prev));
    }

    pub fn leave_function(&mut self) {
        let parent = self.parent.take().expect("must have parent");
        let Self {
            args,
            block_depth,
            function_idx,
            locals,
            parent,
            upvalues,
        } = *parent;

        self.args = args;
        self.block_depth = block_depth;
        self.function_idx = function_idx;
        self.locals = locals;
        self.parent = parent;
        self.upvalues = upvalues;
    }

    fn enter_block(&mut self) {
        self.block_depth += 1;
    }

    fn leave_block(&mut self) {
        self.block_depth -= 1;

        loop {
            if let Some((_, d)) = self.locals.last() {
                if *d > self.block_depth {
                    self.locals.pop();
                    continue;
                }
            }
            break;
        }
    }

    fn new_local(&mut self, name: &'a str) -> usize {
        self.locals.push((name, self.block_depth));
        self.args.len() + self.locals.len() - 1
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

    fn resolve_upvalue(&mut self, name: &'a str) -> Option<UpvalueRef<'a>> {
        if self.is_toplevel() {
            return None;
        }

        if let Some(x) = self.upvalues.iter().find(|x| x.name() == name) {
            return Some(x.to_owned());
        }

        if let Some(local_idx) = self.parent.as_ref().and_then(|x| x.resolve_local(name)) {
            let value = UpvalueRef::Local(name, local_idx);
            self.upvalues.push(value.clone());
            return Some(value);
        }

        if let Some(_) = self.parent.as_mut().and_then(|x| x.resolve_upvalue(name)) {
            let value = UpvalueRef::Upvalue(name, self.upvalues.len());
            self.upvalues.push(value.clone());
            return Some(value);
        }

        return None;
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
                ty: ObjectType::Function,
                object_id: fun_idx,
            }) => write!(
                f,
                "<fn {}>",
                self.vm.interner.get_str(self.vm.functions[fun_idx].name,),
            ),
            Value::String(str_id) => f.write_str(self.vm.interner.get_str(str_id)),
            _ => todo!(),
        }
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use crate::vm::LexicalScope;

    #[test]
    fn test_root_scope_locals() {
        let mut scope = LexicalScope::root();
        assert_eq!(scope.new_local("x"), 0);
        scope.enter_block();
        assert_eq!(scope.new_local("y"), 1);
        assert_eq!(scope.new_local("x"), 2);
        assert_eq!(Some(2), scope.resolve_local("x"));

        scope.function_decl("some_fun", 1, ["a", "b"]);
        assert_eq!(scope.new_local("x"), 3);
        assert_eq!(Some(0), scope.resolve_local("some_fun"));
        assert_eq!(Some(3), scope.resolve_local("x"));
        assert_eq!(None, scope.resolve_local("y"));
        assert_eq!(Some(1), scope.resolve_local("a"));
        scope.leave_function();
        scope.leave_block();
        assert_eq!(Some(0), scope.resolve_local("x"));
    }

    #[test]
    fn test_with_upvalues() {
        let mut scope = LexicalScope::root();
        scope.enter_block();
        scope.new_local("a");
        scope.new_local("b");

        scope.function_decl("outer", 1, ["p1", "p2"]);

        assert_eq!(None, scope.resolve_local("a"));
        assert_eq!(
            Some(crate::vm::UpvalueRef::Local("a", 0)),
            scope.resolve_upvalue("a")
        );

        scope.function_decl("inner", 2, ["inner_param1"]);
        assert_eq!(
            Some(crate::vm::UpvalueRef::Upvalue("a", 0)),
            scope.resolve_upvalue("a")
        );
        assert_eq!(
            Some(crate::vm::UpvalueRef::Upvalue("a", 0)),
            scope.resolve_upvalue("a")
        );
    }
    // "a" ->

    /*
    OP_CLOSURE -> creates function and it's upvalues

    up


     */
}
