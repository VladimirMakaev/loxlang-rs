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
    value::{ClosureValue, ObjectType, ObjectValue, Value, ValueTypes},
};

#[derive(Error, Debug)]
pub enum RuntimeErrorKind {
    #[error("Undefined variable '{name}'.")]
    UndefinedVariable { name: String },
    #[error("Can only call functions and classes.")]
    InvalidCallee,
    #[error("Expected {expected} arguments but got {got}.")]
    InvalidFunArity { got: usize, expected: usize },
    #[error("Unexpected runtime {error}")]
    Unexpected { error: anyhow::Error },
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

struct OpCodeSlice<'a> {
    fun: &'a Function,
    vm: &'a VirtualMachine,
    start: usize,
}

impl<'a> std::fmt::Display for OpCodeSlice<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut ip = self.start;
        while let Some(Ok((op, size))) = self.fun.code.read_next(ip) {
            match op {
                OpCode::DECLAREGLOBAL(idx) => {
                    writeln!(
                        f,
                        "@{ip} DECLAREGLOBAL(\"{value}\")",
                        value = self.vm.interner.get_str(idx.into()),
                    )?;
                    ip = ip + size;
                }
                OpCode::GETGLOBAL(idx) => {
                    writeln!(
                        f,
                        "@{ip} GETGLOBAL(\"{value}\")",
                        value = self.vm.interner.get_str(idx.into()),
                    )?;
                    ip = ip + size;
                }
                OpCode::SETGLOBAL(idx) => {
                    writeln!(
                        f,
                        "@{ip} SETGLOBAL(\"{value}\")",
                        value = self.vm.interner.get_str(idx.into()),
                    )?;
                    ip = ip + size;
                }
                OpCode::CONSTANT(idx) => {
                    writeln!(
                        f,
                        "@{ip} CONSTANT({value})",
                        value = self.vm.as_display(self.vm.constants[idx as usize].clone())
                    )?;
                    ip = ip + size;
                }
                OpCode::CLOSURE(fun_idx) => {
                    let at = ip;
                    ip = ip + size;
                    let name = self
                        .vm
                        .interner
                        .get_str(self.vm.functions[fun_idx as usize].name);
                    let upvalues = (0..self.vm.functions[fun_idx as usize].upvalue_count)
                        .map(|i| {
                            format!(
                                "{is_log}:{idx}",
                                is_log = if self.fun.code.read_u8(ip + 3 * i) == 1 {
                                    "loc"
                                } else {
                                    "up"
                                },
                                idx = self.fun.code.read_u16(ip + 3 * i + 1)
                            )
                        })
                        .collect::<Vec<_>>();
                    writeln!(
                        f,
                        "@{at} CLOSURE({name},{params})",
                        params = upvalues.join(",")
                    )?;
                    ip = ip + &self.vm.functions[fun_idx as usize].upvalue_count * 3;
                }
                _ => {
                    writeln!(f, "@{} {:?}", ip, op)?;
                    ip = ip + size;
                }
            }
        }
        Ok(())
    }
}

struct Closure {
    function_idx: usize,
    upvalues: Vec<Upvalue>,
}

#[repr(u8)]
#[derive(strum::FromRepr)]
enum Upvalue {
    Local(u16, Option<usize>) = 1,
    Parent(u16, Option<usize>),
}

#[derive(Debug)]
struct CallFrame {
    function_idx: usize,
    closure_idx: usize,
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
    closures: Vec<Closure>,
    functions: Vec<Function>,
    function_by_name: HashMap<StrId, usize>,
    globals: HashMap<StrId, Value>,
    codemap: Codemap,
    _closed_upvalues: Vec<Value>,
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
            closures: Default::default(),
            _closed_upvalues: Default::default(),
        }
    }

    pub fn decompile<TOut: Write>(&self, stdout: &mut TOut) -> Result<(), VirtualMachineError> {
        for (name_idx, function_idx) in &self.function_by_name {
            write!(
                stdout,
                "{}:\n{}",
                self.interner.get_str(*name_idx),
                self.opcodes(*function_idx, 0)
            )
            .map_err(|e| VirtualMachineError::Unhandled(e.into()))?
        }

        Ok(())
    }

    fn opcodes(&self, function_idx: usize, start: usize) -> OpCodeSlice {
        OpCodeSlice {
            fun: &self.functions[function_idx],
            start: start,
            vm: self,
        }
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

    fn read_upvalues(&mut self, fun_idx: usize) -> Result<Vec<Upvalue>, VirtualMachineError> {
        let frame = &mut self.frames[self.frame_idx];
        let function = &mut self.functions[fun_idx];
        let mut result = Vec::with_capacity(function.upvalue_count);
        for _ in 0..function.upvalue_count {
            let discriminant = self.functions[frame.function_idx].code.read_u8(frame.ip);
            let mut upvalue = Upvalue::from_repr(discriminant).unwrap();
            match &mut upvalue {
                Upvalue::Local(idx, _) | Upvalue::Parent(idx, _) => {
                    *idx = self.functions[frame.function_idx]
                        .code
                        .read_u16(frame.ip + 1);
                }
            }
            frame.inc_ip(3);
            result.push(upvalue);
        }
        Ok(result)
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
            closure_idx: 0,
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
                            return Err(VirtualMachineError::RuntimeError {
                                kind: RuntimeErrorKind::Unexpected {
                                    error: anyhow!(
                                        "ADD: unsupported types of operands: left: {}, right: {}",
                                        self.as_display(left),
                                        self.as_display(right)
                                    ),
                                },
                                stacktrace: self.stacktrace(),
                            })
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
                    let closure = &self.stack[self.stack.len() - arg_count - 1];
                    debug!(
                        "@{} CALL {}({}) [line: {}]",
                        self.ip(),
                        self.as_display(closure.clone()),
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

                    if let Value::Closure(ClosureValue { closure_id }) = closure {
                        let closure = &self.closures[*closure_id];
                        let fun = &self.functions[closure.function_idx];
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
                            closure_idx: *closure_id,
                            function_idx: closure.function_idx,
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
                OpCode::GETUPVALUE(idx) => {
                    let frame = &self.frames[self.frame_idx];
                    match self.closures[frame.closure_idx].upvalues[idx as usize] {
                        Upvalue::Local(idx, _) => {
                            self.push(
                                self.stack
                                    [self.frames[self.frame_idx - 1].locals_idx + idx as usize]
                                    .clone(),
                            );
                        }
                        Upvalue::Parent(_, _) => todo!(),
                    }
                }
                OpCode::SETUPVALUE(idx) => todo!(),
                OpCode::CLOSURE(function_idx) => {
                    self.frames[self.frame_idx].inc_ip(size);
                    let upvalues = self.read_upvalues(function_idx as usize)?;
                    self.push(Value::closure(self.closures.len()));
                    self.closures.push(Closure {
                        function_idx: function_idx as usize,
                        upvalues,
                    });
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
                if context.is_already_declared(&name.node()) {
                    return Err(VirtualMachineError::CompileError(vec![
                        ParseError::VariableAlreadyDeclared { span: name.span },
                    ]));
                }

                let mut byte_code = ByteCode::default();
                context.function_decl(name.node(), params.iter().map(|x| x.node().as_str()));
                //self.define_function(&name.node(), params.len(), 0, Default::default())?;

                let mut function = Function {
                    arity: params.len(),
                    code: ByteCode::default(),
                    name: self.interner.intern_str(&name.node),
                    upvalue_count: 0,
                };
                self.compile_statement(&body, context, &mut byte_code)?;
                byte_code.write_op(OpCode::NIL, self.lookup_source_line(body.end()));
                byte_code.write_op(OpCode::RET, self.lookup_source_line(body.end()));
                function.code = byte_code;
                function.upvalue_count = context.upvalues.len();

                result.write_op(
                    OpCode::CLOSURE(self.functions.len() as u16),
                    self.lookup_source_line(name.start()),
                );
                for (_, i) in &context.upvalues {
                    i.write_to(result, self.lookup_source_line(name.start()));
                }

                self.function_by_name.insert(
                    self.interner.intern_str(context.name()),
                    self.functions.len(),
                );
                self.functions.push(function);
                context.leave_function();
                let name_idx = self.interner.intern_str(&name.node());
                if context.is_toplevel() {
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
                    if context.is_already_declared(&ident.node) {
                        return Err(VirtualMachineError::CompileError(vec![
                            ParseError::VariableAlreadyDeclared { span: ident.span },
                        ]));
                    }
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
                context.enter_block();
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
                context.leave_block();
                Ok(())
            }
            crate::parser::Stmt::Return(value) => {
                self.compile_expr(value, context, result)?;
                result.write_op(OpCode::RET, self.lookup_source_line(stmt.start()));
                Ok(())
            }
        }
    }

    pub fn compile_expr<'a>(
        &mut self,
        expr: &'a AstExpression,
        context: &mut LexicalScope<'a>,
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
                        match context.resolve_upvalue(name.as_str()) {
                            Some((idx, _)) => {
                                result.write_op(
                                    OpCode::SETUPVALUE(idx as u16),
                                    self.lookup_source_line(expr.start()),
                                );
                            }
                            _ => {
                                let idx = self.interner.intern_str(&name);
                                result.write_op(
                                    OpCode::SETGLOBAL(idx),
                                    self.lookup_source_line(expr.start()),
                                );
                            }
                        }
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
                    match context.resolve_upvalue(name.as_str()) {
                        Some((idx, _)) => {
                            result.write_op(
                                OpCode::GETUPVALUE(idx as u16),
                                self.lookup_source_line(expr.start()),
                            );
                        }
                        None => {
                            let name_idx = self.interner.intern_str(name.as_str());
                            result.write_op(
                                OpCode::GETGLOBAL(name_idx),
                                self.lookup_source_line(expr.start()),
                            )
                        }
                    }
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
pub enum UpvalueRef {
    Local(usize),
    Upvalue(usize),
}

impl UpvalueRef {
    pub fn write_to(&self, code: &mut ByteCode, line: usize) {
        match self {
            UpvalueRef::Local(x) => {
                code.write_byte(1, line);
                code.write_u16(*x as u16, line);
            }
            UpvalueRef::Upvalue(x) => {
                code.write_byte(2, line);
                code.write_u16(*x as u16, line);
            }
        }
    }
}

pub struct LexicalScope<'a> {
    parent: Option<Box<LexicalScope<'a>>>,
    args: Vec<&'a str>,
    locals: Vec<(&'a str, usize)>,
    upvalues: Vec<(&'a str, UpvalueRef)>,
    block_depth: usize,
    name: Option<String>,
}

impl<'a> LexicalScope<'a> {
    pub fn name(&self) -> &str {
        self.name.as_deref().unwrap_or("<script>")
    }

    pub fn is_toplevel(&self) -> bool {
        self.block_depth == 0
    }

    pub fn root() -> Self {
        Self {
            parent: None,
            args: Default::default(),
            locals: Default::default(),
            block_depth: 0,
            upvalues: Default::default(),
            name: None,
        }
    }

    pub fn function_decl(&mut self, name: &'a str, args: impl IntoIterator<Item = &'a str>) {
        let new = Self {
            parent: None,
            args: {
                let mut a = Vec::new();
                a.push(name);
                a.extend(args);
                a
            },
            block_depth: self.block_depth,
            locals: Default::default(),
            upvalues: Vec::with_capacity(self.locals.len() + self.upvalues.len()),
            name: self
                .name
                .as_ref()
                .map(|p_name| Some(format!("{}/{}", p_name, name)))
                .unwrap_or_else(|| Some(name.to_owned())),
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
            locals,
            parent,
            upvalues,
            name,
        } = *parent;

        self.args = args;
        self.block_depth = block_depth;
        self.locals = locals;
        self.parent = parent;
        self.upvalues = upvalues;
        self.name = name;
    }

    pub fn enter_block(&mut self) {
        self.block_depth += 1;
    }

    pub fn leave_block(&mut self) {
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

    pub fn new_local(&mut self, name: &'a str) -> usize {
        self.locals.push((name, self.block_depth));
        self.args.len() + self.locals.len() - 1
    }

    pub fn is_already_declared(&self, name: &'a str) -> bool {
        for (i, arg_name) in self.args.iter().enumerate() {
            if arg_name.eq(&name) {
                return true;
            }
        }

        for i in (0..self.locals.len()).rev() {
            let (n, d) = self.locals[i];
            if d < self.block_depth {
                break;
            }
            if n.eq(name) {
                return true;
            }
        }

        return false;
    }

    pub fn resolve_local(&self, name: &str) -> Option<usize> {
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

    pub fn resolve_upvalue(&mut self, name: &'a str) -> Option<(usize, UpvalueRef)> {
        if self.is_toplevel() {
            return None;
        }

        if let Some((i, (_, value))) = self
            .upvalues
            .iter()
            .enumerate()
            .find(|(_, (n, _))| n == &name)
        {
            return Some((i, value.to_owned()));
        }

        if let Some(local_idx) = self.parent.as_ref().and_then(|x| x.resolve_local(name)) {
            let value = UpvalueRef::Local(local_idx);
            self.upvalues.push((name, value.clone()));
            return Some((self.upvalues.len() - 1, value));
        }

        if let Some((i, _)) = self.parent.as_mut().and_then(|x| x.resolve_upvalue(name)) {
            let value = UpvalueRef::Upvalue(i);
            self.upvalues.push((name, value.clone()));
            return Some((self.upvalues.len() - 1, value));
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
    fn test_is_already_defined() {
        let mut scope = LexicalScope::root();
        scope.enter_block();
        scope.new_local("x");
        assert_eq!(scope.is_already_declared("x"), true);
        scope.enter_block();
        scope.new_local("y");
        assert_eq!(scope.is_already_declared("x"), false);
        assert_eq!(scope.is_already_declared("y"), true);
    }

    #[test]
    fn test_root_scope_locals() {
        let mut scope = LexicalScope::root();
        assert_eq!(scope.new_local("x"), 0);
        scope.enter_block();
        assert_eq!(scope.new_local("y"), 1);
        assert_eq!(scope.new_local("x"), 2);
        assert_eq!(Some(2), scope.resolve_local("x"));

        scope.function_decl("some_fun", ["a", "b"]);
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
    fn test_upvalues_nesting() {
        let mut scope = LexicalScope::root();
        scope.enter_block();
        scope.new_local("b");
        scope.function_decl("f1", ["x"]);
        assert_eq!(
            scope.resolve_upvalue("b"),
            Some((0, crate::vm::UpvalueRef::Local(0)))
        );
        scope.function_decl("inner", ["y"]);
        assert_eq!(
            scope.resolve_upvalue("x"),
            Some((0, crate::vm::UpvalueRef::Local(1)))
        );
        assert_eq!(
            scope.resolve_upvalue("b"),
            Some((1, crate::vm::UpvalueRef::Upvalue(0)))
        );
    }

    #[test]
    fn test_with_upvalues() {
        let mut scope = LexicalScope::root();
        scope.enter_block();
        scope.new_local("a");
        scope.new_local("b");

        scope.function_decl("outer", ["p1", "p2"]);

        assert_eq!(None, scope.resolve_local("a"));
        assert_eq!(
            Some((0, crate::vm::UpvalueRef::Local(0))),
            scope.resolve_upvalue("a")
        );
        assert_eq!(
            Some((1, crate::vm::UpvalueRef::Local(1))),
            scope.resolve_upvalue("b")
        );

        scope.function_decl("inner", ["inner_param1"]);
        assert_eq!(
            Some((0, crate::vm::UpvalueRef::Upvalue(0))),
            scope.resolve_upvalue("a")
        );
        assert_eq!(
            Some((0, crate::vm::UpvalueRef::Upvalue(0))),
            scope.resolve_upvalue("a")
        );

        assert_eq!(scope.upvalues.len(), 1);
        assert_eq!(scope.parent.unwrap().upvalues.len(), 2);
    }
}
