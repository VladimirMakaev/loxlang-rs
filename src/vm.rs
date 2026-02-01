use std::{
    collections::{hash_map::DefaultHasher, BTreeMap},
    fmt::{Display, Formatter},
    hash::BuildHasherDefault,
    io::Write,
    mem,
};

use hashbrown::HashMap;

use thiserror::Error;
use tracing::debug;

use crate::{
    byte_code::{ByteCode, JumpOffset, OpCode, OpCodeError, OpCodeTypes},
    codemap::Codemap,
    gc::{GcRef, Heap},
    interner::{DefaultInterner, DefaultStringTable, Interner, StringTable, StrId},
    object::{Obj, ObjBoundMethod, ObjClosure, ObjFunction, ObjKind, ObjUpvalue, UpvalueLocation},
    parser::{
        AstExpression, AstIdent, AstStmt, ClassDeclaration, Expression, ForStmt, FunDeclaration, IfStmt,
        LogicalExpression, ParseError, Parser, Span, StmtDeclaration, WhileStmt,
    },
    value::{Value, ValueTypes},
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
    #[error("Stack overflow.")]
    StackOverflow,
}

/// Maximum call frame depth before stack overflow
const FRAMES_MAX: usize = 256;

/// Returns the clox-formatted error message for operand type errors
fn operand_type_error_message(instruction: OpCodeTypes) -> &'static str {
    match instruction {
        OpCodeTypes::NEGATE => "Operand must be a number.",
        _ => "Operands must be numbers.",
    }
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

    #[error("{}", operand_type_error_message(*.instruction))]
    UnexpectedStackOperandType {
        instruction: OpCodeTypes,
        expected: ValueTypes,
        actual: ValueTypes,
    },

    #[error("Unhandled error: {0:?}")]
    Unhandled(#[from] anyhow::Error),
}

// OpCodeSlice struct - REMOVED: Decompilation not yet updated for heap-based functions

// UpValuePtr struct - REMOVED: Upvalues are now heap-allocated as ObjUpvalue via GcRef

// Closure struct - REMOVED: Closures are now heap-allocated as ObjClosure via GcRef

#[derive(Debug)]
struct CallFrame {
    closure: GcRef, // GcRef to ObjClosure on heap
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

#[derive(Default)]
struct OpenUpValues {
    slots_to_values: BTreeMap<usize, GcRef>,
}

impl OpenUpValues {
    pub fn take(&mut self, stack_slot: usize) -> Option<GcRef> {
        self.slots_to_values.remove(&stack_slot)
    }

    pub fn get(&self, stack_slot: usize) -> Option<GcRef> {
        self.slots_to_values.get(&stack_slot).copied()
    }

    pub fn insert(&mut self, stack_slot: usize, upvalue: GcRef) {
        self.slots_to_values.insert(stack_slot, upvalue);
    }

    /// Get all open upvalues at or above the given stack slot
    pub fn close_from(&mut self, slot: usize) -> Vec<(usize, GcRef)> {
        let keys: Vec<_> = self.slots_to_values.range(slot..).map(|(&k, &v)| (k, v)).collect();
        for (k, _) in &keys {
            self.slots_to_values.remove(k);
        }
        keys
    }
}

pub struct VirtualMachine {
    pub(crate) frame_idx: usize,
    frames: Vec<CallFrame>,
    pub(crate) interner: DefaultInterner,
    pub(crate) string_table: DefaultStringTable, // NEW: GcRef-based string lookup
    pub(crate) stack: Vec<Value>,
    pub(crate) constants: Vec<Value>,
    // closures: Vec<Closure> - REMOVED: Closures are now heap-allocated
    // functions: Vec<Function> - REMOVED: Functions are now heap-allocated
    // function_by_name: HashMap<StrId, usize> - REMOVED: No longer needed with heap storage
    // closed_upvalues: Vec<Value> - REMOVED: Upvalues now store closed value directly in ObjUpvalue
    pub(crate) globals: HashMap<StrId, Value>,
    pub(crate) codemap: Codemap,
    open_upvalues: OpenUpValues,
    pub(crate) heap: Heap,
    /// The top-level script closure (set after compilation)
    script_closure: Option<GcRef>,
}

impl VirtualMachine {
    pub fn new() -> Self {
        Self {
            constants: Default::default(),
            stack: Default::default(),
            interner: Interner::new(BuildHasherDefault::<DefaultHasher>::default()),
            string_table: StringTable::new(BuildHasherDefault::<DefaultHasher>::default()),
            globals: Default::default(),
            frame_idx: 0,
            frames: Default::default(),
            codemap: Codemap {
                line_endings: Default::default(),
                lines_start_at_1: true,
                position_starts_at_1: false,
            },
            open_upvalues: Default::default(),
            heap: Heap::new(),
            script_closure: None,
        }
    }

    // =========================================================================
    // Heap allocation helpers for GC-managed objects
    // =========================================================================

    /// Allocate a string on the heap, deduplicating via string_table.
    /// Returns a GcRef to the string object.
    pub fn alloc_string(&mut self, s: String) -> GcRef {
        self.maybe_collect();
        let hash = self.string_table.hash_string(&s);

        // Check intern table for existing string
        if let Some(existing) = self.string_table.get(hash) {
            if let ObjKind::String(existing_str) = &self.heap.get(existing).kind {
                if existing_str.value == s {
                    return existing;
                }
            }
        }

        // Allocate new string
        let obj = Obj::string(s, hash);
        let r = self.heap.alloc(obj);
        self.string_table.insert(hash, r);
        r
    }

    /// Allocate a function on the heap.
    pub fn alloc_function(&mut self, name: GcRef, arity: usize, code: ByteCode, upvalue_count: usize) -> GcRef {
        self.maybe_collect();
        let obj = Obj::function(name, arity, code, upvalue_count);
        self.heap.alloc(obj)
    }

    /// Allocate a closure on the heap.
    pub fn alloc_closure(&mut self, function: GcRef, upvalues: Vec<GcRef>) -> GcRef {
        self.maybe_collect();
        let obj = Obj::closure(function, upvalues);
        self.heap.alloc(obj)
    }

    /// Allocate an upvalue on the heap.
    pub fn alloc_upvalue(&mut self, location: UpvalueLocation) -> GcRef {
        self.maybe_collect();
        let obj = Obj::upvalue(location);
        self.heap.alloc(obj)
    }

    /// Allocate a class on the heap.
    pub fn alloc_class(&mut self, name: GcRef) -> GcRef {
        self.maybe_collect();
        let obj = Obj::class(name);
        self.heap.alloc(obj)
    }

    /// Allocate an instance on the heap.
    pub fn alloc_instance(&mut self, klass: GcRef) -> GcRef {
        self.maybe_collect();
        let obj = Obj::instance(klass);
        self.heap.alloc(obj)
    }

    /// Allocate a bound method on the heap.
    pub fn alloc_bound_method(&mut self, receiver: Value, method: GcRef) -> GcRef {
        self.maybe_collect();
        let obj = Obj::bound_method(receiver, method);
        self.heap.alloc(obj)
    }

    /// Get a string value from a GcRef (panics if not a string).
    pub fn get_heap_string(&self, r: GcRef) -> &str {
        if let ObjKind::String(s) = &self.heap.get(r).kind {
            &s.value
        } else {
            panic!("Expected string object")
        }
    }

    /// Get a function from a GcRef (panics if not a function).
    pub fn get_heap_function(&self, r: GcRef) -> &ObjFunction {
        if let ObjKind::Function(f) = &self.heap.get(r).kind {
            f
        } else {
            panic!("Expected function object")
        }
    }

    /// Get a closure from a GcRef (panics if not a closure).
    pub fn get_heap_closure(&self, r: GcRef) -> &ObjClosure {
        if let ObjKind::Closure(c) = &self.heap.get(r).kind {
            c
        } else {
            panic!("Expected closure object")
        }
    }

    /// Get an upvalue from a GcRef (panics if not an upvalue).
    pub fn get_heap_upvalue(&self, r: GcRef) -> &ObjUpvalue {
        if let ObjKind::Upvalue(u) = &self.heap.get(r).kind {
            u
        } else {
            panic!("Expected upvalue object")
        }
    }

    /// Get a bound method from a GcRef (panics if not a bound method).
    pub fn get_heap_bound_method(&self, r: GcRef) -> &ObjBoundMethod {
        if let ObjKind::BoundMethod(bm) = &self.heap.get(r).kind {
            bm
        } else {
            panic!("Expected bound method object")
        }
    }

    /// Capture an upvalue for the given stack slot.
    /// Reuses existing open upvalue if one exists, otherwise creates new one.
    fn capture_upvalue(&mut self, stack_slot: usize) -> GcRef {
        // Check if we already have an open upvalue for this slot
        if let Some(existing) = self.open_upvalues.get(stack_slot) {
            return existing;
        }

        // Create new open upvalue on heap
        let upvalue_ref = self.alloc_upvalue(UpvalueLocation::Open(stack_slot));
        self.open_upvalues.insert(stack_slot, upvalue_ref);
        upvalue_ref
    }

    /// Close all upvalues at or above the given stack slot.
    fn close_upvalues(&mut self, from_slot: usize) {
        let to_close = self.open_upvalues.close_from(from_slot);

        for (stack_slot, upvalue_ref) in to_close {
            let value = self.stack[stack_slot].clone();
            let obj = self.heap.get_mut(upvalue_ref);
            if let ObjKind::Upvalue(upvalue) = &mut obj.kind {
                upvalue.location = UpvalueLocation::Closed(value);
            }
        }
    }

    // =========================================================================
    // Garbage Collection
    // =========================================================================

    /// Mark all root objects (reachable without traversal)
    fn mark_roots(&mut self) {
        // 1. Mark stack values
        let stack_values: Vec<Value> = self.stack.clone();
        for value in stack_values {
            self.mark_value(value);
        }

        // 2. Mark global variable values
        let global_values: Vec<Value> = self.globals.values().cloned().collect();
        for value in global_values {
            self.mark_value(value);
        }

        // 3. Mark constants (they may contain object references)
        let constants: Vec<Value> = self.constants.clone();
        for value in constants {
            self.mark_value(value);
        }

        // 4. Mark call frame closures (CRITICAL - these are roots during execution)
        let frame_closures: Vec<GcRef> = self.frames.iter().map(|f| f.closure).collect();
        for closure_ref in frame_closures {
            self.heap.mark_object(closure_ref);
        }

        // 5. Mark open upvalues (they point to ObjUpvalue on heap)
        let open_refs: Vec<GcRef> = self.open_upvalues.slots_to_values.values().copied().collect();
        for r in open_refs {
            self.heap.mark_object(r);
        }
    }

    /// Mark a value if it's an object reference
    fn mark_value(&mut self, value: Value) {
        if let Value::Object(r) = value {
            self.heap.mark_object(r);
        }
    }

    /// Run garbage collection
    pub fn collect_garbage(&mut self) {
        #[cfg(feature = "debug_gc")]
        let before = self.heap.bytes_allocated;

        #[cfg(feature = "debug_gc")]
        eprintln!("-- gc begin ({} bytes)", before);

        // Reset for new collection cycle
        self.heap.reset_gray_stack();

        // Mark phase
        self.mark_roots();
        self.heap.trace_references();

        // Clean string table (weak references)
        self.string_table.remove_unmarked(|r| self.heap.is_marked(r));

        // Sweep phase
        self.heap.sweep();

        // Update threshold
        self.heap.update_threshold();

        #[cfg(feature = "debug_gc")]
        {
            let after = self.heap.bytes_allocated;
            eprintln!("-- gc end ({} bytes, freed {})", after, before.saturating_sub(after));
        }
    }

    /// Check if GC should run and trigger if needed
    fn maybe_collect(&mut self) {
        if self.heap.bytes_allocated > self.heap.next_gc {
            self.collect_garbage();
        }
    }

    pub fn decompile<TOut: Write>(&self, _stdout: &mut TOut) -> Result<(), VirtualMachineError> {
        // TODO: Decompilation not yet updated for heap-based functions
        // This was only used for debugging, not production functionality
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
        let mut compile_errors: Vec<ParseError> = Vec::new();

        for smt in smts.iter() {
            if let Err(VirtualMachineError::CompileError(errors)) =
                self.compile_statement(smt, &mut context, &mut result)
            {
                compile_errors.extend(errors);
            }
        }

        if !compile_errors.is_empty() {
            return Err(VirtualMachineError::CompileError(compile_errors));
        }

        // Create script function and closure on heap
        let name_ref = self.alloc_string("<script>".to_string());
        let upvalue_count = context.upvalues.len();
        let function_ref = self.alloc_function(name_ref, 0, result, upvalue_count);

        // TODO(03-08): Closure upvalues are empty - fix in Plan 03-08
        let closure_ref = self.alloc_closure(function_ref, vec![]);
        self.script_closure = Some(closure_ref);

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
        Ok(value.clone().try_as_number().ok_or_else(|| {
            VirtualMachineError::UnexpectedStackOperandType {
                instruction: op_code,
                expected: ValueTypes::Number,
                actual: value.into(),
            }
        })?)
    }

    fn is_truthy(&self, value: &Value) -> bool {
        match value {
            Value::Number(_x) => true,
            Value::Bool(x) => *x,
            Value::Nil => false,
            Value::Object(_) => true, // All objects (closures, strings) are truthy
        }
    }

    fn equal(&self, left: &Value, right: &Value) -> bool {
        match (left, right) {
            (Value::Number(l), Value::Number(r)) => l == r,
            (Value::Bool(l), Value::Bool(r)) => l == r,
            (Value::Nil, Value::Nil) => true,
            // Object interning ensures equal objects have same GcRef (identity comparison)
            (Value::Object(l), Value::Object(r)) => l == r,
            (_, _) => false,
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
        let closure = self.get_heap_closure(frame.closure);
        let function = self.get_heap_function(closure.function);
        function.code.read_next(frame.ip)
    }

    fn locals_idx(&self) -> usize {
        self.frames[self.frame_idx].locals_idx
    }

    // read_upvalues function - REMOVED: Upvalue capture now handled directly in CLOSURE opcode

    fn ip(&self) -> usize {
        self.frames[self.frame_idx].ip
    }

    fn current_line(&self) -> usize {
        let frame = &self.frames[self.frame_idx];
        let closure = self.get_heap_closure(frame.closure);
        let function = self.get_heap_function(closure.function);
        function.code.line(self.ip())
    }

    fn stacktrace_line(&self, frame: &CallFrame) -> String {
        let closure = self.get_heap_closure(frame.closure);
        let function = self.get_heap_function(closure.function);
        let name = self.get_heap_string(function.name);
        format!(
            "[line {}] in {}",
            function.code.line(frame.ip()),
            name
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
        let script_closure = self.script_closure.expect("compile() must be called before run()");
        self.frames.push(CallFrame {
            closure: script_closure,
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
                        (Value::Object(l), Value::Object(r)) => {
                            // Check if both are strings using ObjKind pattern matching
                            let left_obj = self.heap.get(l);
                            let right_obj = self.heap.get(r);
                            if let (ObjKind::String(ls), ObjKind::String(rs)) =
                                (&left_obj.kind, &right_obj.kind)
                            {
                                let concatenate = format!("{}{}", ls.value, rs.value);
                                let result = self.alloc_string(concatenate);
                                self.push(Value::Object(result));
                            } else {
                                // Not strings - return type error
                                return Err(VirtualMachineError::RuntimeError {
                                    kind: RuntimeErrorKind::Unexpected {
                                        error: anyhow::Error::msg("Operands must be two numbers or two strings."),
                                    },
                                    stacktrace: self.stacktrace(),
                                });
                            }
                        }
                        (_, _) => {
                            return Err(VirtualMachineError::RuntimeError {
                                kind: RuntimeErrorKind::Unexpected {
                                    error: anyhow::Error::msg("Operands must be two numbers or two strings."),
                                },
                                stacktrace: self.stacktrace(),
                            });
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
                    let right = self.pop_number(OpCodeTypes::SUBTRACT)?;
                    let left = self.pop_number(OpCodeTypes::SUBTRACT)?;
                    debug!(
                        "@{} SUBTRACT {} {} [line: {}]",
                        self.ip(),
                        left,
                        right,
                        self.current_line()
                    );
                    self.push((left - right).into());
                }
                OpCode::DIVIDE => {
                    let right = self.pop_number(OpCodeTypes::DIVIDE)?;
                    let left = self.pop_number(OpCodeTypes::DIVIDE)?;
                    debug!(
                        "@{} DIVIDE {} / {} [line: {}]",
                        self.ip(),
                        left,
                        right,
                        self.current_line()
                    );
                    self.push((left / right).into());
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
                    let callee = &self.stack[self.stack.len() - arg_count - 1];
                    debug!(
                        "@{} CALL {}({}) [line: {}]",
                        self.ip(),
                        self.as_display(callee.clone()),
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

                    if let Value::Object(obj_ref) = callee {
                        let obj_ref = *obj_ref;
                        match &self.heap.get(obj_ref).kind {
                            ObjKind::Closure(closure) => {
                                let function = self.get_heap_function(closure.function);
                                if function.arity != arg_count {
                                    return Err(VirtualMachineError::RuntimeError {
                                        kind: RuntimeErrorKind::InvalidFunArity {
                                            got: arg_count,
                                            expected: function.arity,
                                        },
                                        stacktrace: self.stacktrace(),
                                    });
                                }
                                // Check for stack overflow before pushing new frame
                                if self.frames.len() >= FRAMES_MAX {
                                    return Err(VirtualMachineError::RuntimeError {
                                        kind: RuntimeErrorKind::StackOverflow,
                                        stacktrace: self.stacktrace(),
                                    });
                                }
                                // Advance caller's IP before pushing new frame
                                self.frames[self.frame_idx].inc_ip(size);
                                self.frames.push(CallFrame {
                                    ip: 0,
                                    closure: obj_ref,
                                    locals_idx: self.stack.len() - arg_count - 1,
                                });
                                self.frame_idx += 1;
                                continue;
                            }
                            ObjKind::Class(class) => {
                                // Extract methods for init lookup before mutable borrow
                                let methods = class.methods.clone();

                                // Create instance and replace class on stack at callee position
                                let instance_ref = self.alloc_instance(obj_ref);
                                let stack_pos = self.stack.len() - arg_count - 1;
                                self.stack[stack_pos] = Value::Object(instance_ref);

                                // Look up init method
                                let init_name = self.alloc_string("init".to_string());
                                if let Some(&init_ref) = methods.get(&init_name) {
                                    // Get closure and function for arity check
                                    let closure = self.get_heap_closure(init_ref);
                                    let function = self.get_heap_function(closure.function);

                                    if function.arity != arg_count {
                                        return Err(VirtualMachineError::RuntimeError {
                                            kind: RuntimeErrorKind::InvalidFunArity {
                                                got: arg_count,
                                                expected: function.arity,
                                            },
                                            stacktrace: self.stacktrace(),
                                        });
                                    }

                                    // Check stack overflow
                                    if self.frames.len() >= FRAMES_MAX {
                                        return Err(VirtualMachineError::RuntimeError {
                                            kind: RuntimeErrorKind::StackOverflow,
                                            stacktrace: self.stacktrace(),
                                        });
                                    }

                                    // Push call frame for init
                                    // Advance caller's IP before pushing new frame
                                    self.frames[self.frame_idx].inc_ip(size);
                                    self.frames.push(CallFrame {
                                        ip: 0,
                                        closure: init_ref,
                                        locals_idx: stack_pos,
                                    });
                                    self.frame_idx += 1;
                                    continue;
                                } else if arg_count != 0 {
                                    // No init method but arguments provided
                                    return Err(VirtualMachineError::RuntimeError {
                                        kind: RuntimeErrorKind::InvalidFunArity {
                                            got: arg_count,
                                            expected: 0,
                                        },
                                        stacktrace: self.stacktrace(),
                                    });
                                }
                                // No init and no args - instance is ready on stack
                            }
                            ObjKind::BoundMethod(bound) => {
                                // Replace callee slot with receiver (becomes slot 0 / 'this')
                                let receiver = bound.receiver.clone();
                                let method_ref = bound.method;
                                let stack_pos = self.stack.len() - arg_count - 1;
                                self.stack[stack_pos] = receiver;

                                // Get closure and function for arity check
                                let closure = self.get_heap_closure(method_ref);
                                let function = self.get_heap_function(closure.function);

                                if function.arity != arg_count {
                                    return Err(VirtualMachineError::RuntimeError {
                                        kind: RuntimeErrorKind::InvalidFunArity {
                                            got: arg_count,
                                            expected: function.arity,
                                        },
                                        stacktrace: self.stacktrace(),
                                    });
                                }

                                // Check stack overflow
                                if self.frames.len() >= FRAMES_MAX {
                                    return Err(VirtualMachineError::RuntimeError {
                                        kind: RuntimeErrorKind::StackOverflow,
                                        stacktrace: self.stacktrace(),
                                    });
                                }

                                // Push call frame
                                // Advance caller's IP before pushing new frame
                                self.frames[self.frame_idx].inc_ip(size);
                                self.frames.push(CallFrame {
                                    ip: 0,
                                    closure: method_ref,
                                    locals_idx: stack_pos,
                                });
                                self.frame_idx += 1;
                                continue;
                            }
                            _ => {
                                return Err(VirtualMachineError::RuntimeError {
                                    kind: RuntimeErrorKind::InvalidCallee,
                                    stacktrace: self.stacktrace(),
                                });
                            }
                        }
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

                    // Close any open upvalues for locals that are about to be popped
                    // (including slot 0 which is 'this' for methods or function name for functions)
                    self.close_upvalues(self.locals_idx());

                    self.stack.truncate(self.frames[self.frame_idx].locals_idx);
                    self.frames.pop();
                    self.frame_idx -= 1;
                    self.stack.push(ret_value);
                    // Caller's IP was already advanced before pushing the frame

                    continue;
                }
                OpCode::GETUPVALUE(slot) => {
                    debug!(
                        "@{at} GETUPVALUE {slot} [line: {line}]",
                        at = self.ip(),
                        line = self.current_line()
                    );
                    let frame = &self.frames[self.frame_idx];
                    let closure = self.get_heap_closure(frame.closure);
                    let upvalue_ref = closure.upvalues[slot as usize];
                    let upvalue = self.get_heap_upvalue(upvalue_ref);

                    let value = match &upvalue.location {
                        UpvalueLocation::Open(stack_slot) => self.stack[*stack_slot].clone(),
                        UpvalueLocation::Closed(val) => val.clone(),
                    };
                    self.push(value);
                }
                OpCode::SETUPVALUE(slot) => {
                    let value = self.peek()?.clone();
                    debug!(
                        "@{at} SETUPVALUE {slot} = {val} [line: {line}]",
                        at = self.ip(),
                        val = self.as_display(value.clone()),
                        line = self.current_line()
                    );
                    let frame = &self.frames[self.frame_idx];
                    let closure = self.get_heap_closure(frame.closure);
                    let upvalue_ref = closure.upvalues[slot as usize];

                    // Get mutable access to upvalue
                    let obj = self.heap.get_mut(upvalue_ref);
                    if let ObjKind::Upvalue(upvalue) = &mut obj.kind {
                        match &mut upvalue.location {
                            UpvalueLocation::Open(stack_slot) => {
                                self.stack[*stack_slot] = value;
                            }
                            UpvalueLocation::Closed(ref mut val) => {
                                *val = value;
                            }
                        }
                    }
                }
                OpCode::CLOSURE(function_const_idx) => {
                    debug!(
                        "@{at} CLOSURE {function_const_idx} [line: {line}]",
                        at = self.ip(),
                        line = self.current_line()
                    );
                    self.frames[self.frame_idx].inc_ip(size);

                    // Get function ref from constants table
                    let function_value = &self.constants[function_const_idx as usize];
                    let function_ref = match function_value {
                        Value::Object(r) => *r,
                        _ => return Err(self.unhandled_error("Expected function object in constants")),
                    };

                    // Get upvalue count from function
                    let function = self.get_heap_function(function_ref);
                    let upvalue_count = function.upvalue_count;

                    // Read upvalue descriptors and capture upvalues
                    let mut upvalue_refs: Vec<GcRef> = Vec::with_capacity(upvalue_count);
                    for _ in 0..upvalue_count {
                        let frame = &self.frames[self.frame_idx];
                        let frame_closure = frame.closure;
                        let closure = self.get_heap_closure(frame_closure);
                        let parent_function = self.get_heap_function(closure.function);

                        let is_local = parent_function.code.read_u8(frame.ip) == 1;
                        let index = parent_function.code.read_u16(frame.ip + 1) as usize;
                        self.frames[self.frame_idx].inc_ip(3);

                        let upvalue_ref = if is_local {
                            // Capture from stack
                            let stack_slot = self.locals_idx() + index;
                            self.capture_upvalue(stack_slot)
                        } else {
                            // Capture from enclosing closure's upvalues
                            let parent_closure = self.get_heap_closure(frame_closure);
                            parent_closure.upvalues[index]
                        };
                        upvalue_refs.push(upvalue_ref);
                    }

                    let closure_ref = self.alloc_closure(function_ref, upvalue_refs);
                    self.push(Value::Object(closure_ref));

                    continue;
                }
                OpCode::CLOSEUPVALUE => {
                    debug!(
                        "@{at} CLOSEUPVALUE {value} [line: {line}]",
                        at = self.ip(),
                        line = self.current_line(),
                        value = self.as_display(self.stack.last().cloned().unwrap()),
                    );

                    let slot = self.stack.len() - 1;
                    self.close_upvalues(slot);
                    self.pop()?;
                }
                OpCode::CLASS(name_const_idx) => {
                    // Get class name GcRef from constants table
                    let name_value = &self.constants[name_const_idx as usize];
                    let name_ref = match name_value {
                        Value::Object(r) => *r,
                        _ => return Err(self.unhandled_error("Expected string object in constants")),
                    };

                    debug!(
                        "@{at} CLASS {name} [line: {line}]",
                        at = self.ip(),
                        name = self.get_heap_string(name_ref),
                        line = self.current_line()
                    );

                    // Create class object and push to stack
                    let class_ref = self.alloc_class(name_ref);
                    self.push(Value::Object(class_ref));
                }
                OpCode::GET_PROPERTY(name_const_idx) => {
                    // Get property name GcRef from constants table
                    let name_value = &self.constants[name_const_idx as usize];
                    let name_ref = match name_value {
                        Value::Object(r) => *r,
                        _ => return Err(self.unhandled_error("Expected string object in constants")),
                    };

                    debug!(
                        "@{at} GET_PROPERTY {name} [line: {line}]",
                        at = self.ip(),
                        name = self.get_heap_string(name_ref),
                        line = self.current_line()
                    );

                    // Peek instance from stack
                    let instance_value = self.peek()?;

                    // Check if it's an instance
                    if let Value::Object(instance_ref) = instance_value {
                        let obj = self.heap.get(instance_ref);
                        if let ObjKind::Instance(instance) = &obj.kind {
                            // Look up field in instance.fields by name_ref
                            if let Some(value) = instance.fields.get(&name_ref) {
                                let value = value.clone();
                                // Pop instance from stack
                                self.pop()?;
                                // Push field value
                                self.push(value);
                            } else {
                                // Field not found, check class methods
                                let klass_ref = instance.klass;
                                let klass_obj = self.heap.get(klass_ref);
                                if let ObjKind::Class(klass) = &klass_obj.kind {
                                    if let Some(&method_ref) = klass.methods.get(&name_ref) {
                                        // Clone instance_value before passing to alloc to avoid borrow issues
                                        let receiver = instance_value.clone();
                                        let bound_ref = self.alloc_bound_method(receiver, method_ref);
                                        // Pop instance from stack
                                        self.pop()?;
                                        // Push bound method
                                        self.push(Value::Object(bound_ref));
                                    } else {
                                        // Neither field nor method found
                                        let name = self.get_heap_string(name_ref).to_string();
                                        return Err(VirtualMachineError::RuntimeError {
                                            kind: RuntimeErrorKind::Unexpected {
                                                error: anyhow::Error::msg(format!("Undefined property '{}'.", name)),
                                            },
                                            stacktrace: self.stacktrace(),
                                        });
                                    }
                                } else {
                                    // klass is not a class (shouldn't happen)
                                    return Err(self.unhandled_error("Instance's klass is not a class"));
                                }
                            }
                        } else {
                            // Not an instance
                            return Err(VirtualMachineError::RuntimeError {
                                kind: RuntimeErrorKind::Unexpected {
                                    error: anyhow::Error::msg("Only instances have properties."),
                                },
                                stacktrace: self.stacktrace(),
                            });
                        }
                    } else {
                        // Not an object at all
                        return Err(VirtualMachineError::RuntimeError {
                            kind: RuntimeErrorKind::Unexpected {
                                error: anyhow::Error::msg("Only instances have properties."),
                            },
                            stacktrace: self.stacktrace(),
                        });
                    }
                }
                OpCode::SET_PROPERTY(name_const_idx) => {
                    // Get property name GcRef from constants table
                    let name_value = &self.constants[name_const_idx as usize];
                    let name_ref = match name_value {
                        Value::Object(r) => *r,
                        _ => return Err(self.unhandled_error("Expected string object in constants")),
                    };

                    debug!(
                        "@{at} SET_PROPERTY {name} [line: {line}]",
                        at = self.ip(),
                        name = self.get_heap_string(name_ref),
                        line = self.current_line()
                    );

                    // Pop value from stack
                    let value = self.pop()?;
                    // Pop instance from stack
                    let instance_value = self.pop()?;

                    // Check if it's an instance
                    if let Value::Object(instance_ref) = instance_value {
                        // Get mutable access to instance
                        let obj = self.heap.get_mut(instance_ref);
                        if let ObjKind::Instance(instance) = &mut obj.kind {
                            // Insert/update field
                            instance.fields.insert(name_ref, value.clone());
                            // Push value back onto stack (assignment returns the value)
                            self.push(value);
                        } else {
                            // Not an instance
                            return Err(VirtualMachineError::RuntimeError {
                                kind: RuntimeErrorKind::Unexpected {
                                    error: anyhow::Error::msg("Only instances have fields."),
                                },
                                stacktrace: self.stacktrace(),
                            });
                        }
                    } else {
                        // Not an object at all
                        return Err(VirtualMachineError::RuntimeError {
                            kind: RuntimeErrorKind::Unexpected {
                                error: anyhow::Error::msg("Only instances have fields."),
                            },
                            stacktrace: self.stacktrace(),
                        });
                    }
                }
                OpCode::METHOD(name_const_idx) => {
                    // Get method name GcRef from constants table
                    let name_value = &self.constants[name_const_idx as usize];
                    let name_ref = match name_value {
                        Value::Object(r) => *r,
                        _ => return Err(self.unhandled_error("Expected string object in constants")),
                    };

                    debug!(
                        "@{at} METHOD {name} [line: {line}]",
                        at = self.ip(),
                        name = self.get_heap_string(name_ref),
                        line = self.current_line()
                    );

                    // Pop closure from stack
                    let closure_value = self.pop()?;
                    let closure_ref = match closure_value {
                        Value::Object(r) => r,
                        _ => return Err(self.unhandled_error("Expected closure object")),
                    };

                    // Pop class from stack (METHOD pops class after adding method)
                    let class_value = self.pop()?;
                    let class_ref = match class_value {
                        Value::Object(r) => r,
                        _ => return Err(self.unhandled_error("Expected class object")),
                    };

                    // Add method to class
                    let obj = self.heap.get_mut(class_ref);
                    if let ObjKind::Class(class) = &mut obj.kind {
                        class.methods.insert(name_ref, closure_ref);
                    } else {
                        return Err(self.unhandled_error("Expected class object"));
                    }
                }
                OpCode::INVOKE(name_idx, arg_count) => {
                    let arg_count = arg_count as usize;

                    // Get method name from constants
                    let name_ref = match &self.constants[name_idx as usize] {
                        Value::Object(r) => *r,
                        _ => return Err(self.unhandled_error("Expected string constant for INVOKE")),
                    };

                    debug!(
                        "@{at} INVOKE {name}({arg_count}) [line: {line}]",
                        at = self.ip(),
                        name = self.get_heap_string(name_ref),
                        line = self.current_line()
                    );

                    // Get receiver from stack (before args)
                    let receiver_pos = self.stack.len() - arg_count - 1;
                    let receiver = self.stack[receiver_pos].clone();

                    if let Value::Object(obj_ref) = receiver {
                        let obj = self.heap.get(obj_ref);
                        if let ObjKind::Instance(instance) = &obj.kind {
                            // Check field first (field can shadow method and be callable)
                            if let Some(field_value) = instance.fields.get(&name_ref).cloned() {
                                // Field found - replace receiver with field value and call it
                                self.stack[receiver_pos] = field_value.clone();
                                self.frames[self.frame_idx].inc_ip(size);

                                // Call the field value (delegate to existing call logic)
                                match &field_value {
                                    Value::Object(field_obj_ref) => {
                                        match &self.heap.get(*field_obj_ref).kind {
                                            ObjKind::Closure(closure) => {
                                                let function = self.get_heap_function(closure.function);
                                                if function.arity != arg_count {
                                                    return Err(VirtualMachineError::RuntimeError {
                                                        kind: RuntimeErrorKind::InvalidFunArity {
                                                            got: arg_count,
                                                            expected: function.arity,
                                                        },
                                                        stacktrace: self.stacktrace(),
                                                    });
                                                }
                                                if self.frames.len() >= FRAMES_MAX {
                                                    return Err(VirtualMachineError::RuntimeError {
                                                        kind: RuntimeErrorKind::StackOverflow,
                                                        stacktrace: self.stacktrace(),
                                                    });
                                                }
                                                self.frames.push(CallFrame {
                                                    ip: 0,
                                                    closure: *field_obj_ref,
                                                    locals_idx: receiver_pos,
                                                });
                                                self.frame_idx += 1;
                                                continue;
                                            }
                                            ObjKind::BoundMethod(bound) => {
                                                // Field is a bound method
                                                let bound_receiver = bound.receiver.clone();
                                                let method_ref = bound.method;
                                                self.stack[receiver_pos] = bound_receiver;

                                                let closure = self.get_heap_closure(method_ref);
                                                let function = self.get_heap_function(closure.function);

                                                if function.arity != arg_count {
                                                    return Err(VirtualMachineError::RuntimeError {
                                                        kind: RuntimeErrorKind::InvalidFunArity {
                                                            got: arg_count,
                                                            expected: function.arity,
                                                        },
                                                        stacktrace: self.stacktrace(),
                                                    });
                                                }
                                                if self.frames.len() >= FRAMES_MAX {
                                                    return Err(VirtualMachineError::RuntimeError {
                                                        kind: RuntimeErrorKind::StackOverflow,
                                                        stacktrace: self.stacktrace(),
                                                    });
                                                }
                                                self.frames.push(CallFrame {
                                                    ip: 0,
                                                    closure: method_ref,
                                                    locals_idx: receiver_pos,
                                                });
                                                self.frame_idx += 1;
                                                continue;
                                            }
                                            _ => {
                                                return Err(VirtualMachineError::RuntimeError {
                                                    kind: RuntimeErrorKind::InvalidCallee,
                                                    stacktrace: self.stacktrace(),
                                                });
                                            }
                                        }
                                    }
                                    _ => {
                                        return Err(VirtualMachineError::RuntimeError {
                                            kind: RuntimeErrorKind::InvalidCallee,
                                            stacktrace: self.stacktrace(),
                                        });
                                    }
                                }
                            } else {
                                // Field not found, check class methods
                                let klass_ref = instance.klass;
                                let klass_obj = self.heap.get(klass_ref);
                                if let ObjKind::Class(class) = &klass_obj.kind {
                                    if let Some(&method_ref) = class.methods.get(&name_ref) {
                                        // Direct method call - instance stays at receiver_pos (becomes this)
                                        let closure = self.get_heap_closure(method_ref);
                                        let function = self.get_heap_function(closure.function);

                                        if function.arity != arg_count {
                                            return Err(VirtualMachineError::RuntimeError {
                                                kind: RuntimeErrorKind::InvalidFunArity {
                                                    got: arg_count,
                                                    expected: function.arity,
                                                },
                                                stacktrace: self.stacktrace(),
                                            });
                                        }
                                        if self.frames.len() >= FRAMES_MAX {
                                            return Err(VirtualMachineError::RuntimeError {
                                                kind: RuntimeErrorKind::StackOverflow,
                                                stacktrace: self.stacktrace(),
                                            });
                                        }
                                        // Advance caller's IP before pushing new frame
                                        self.frames[self.frame_idx].inc_ip(size);
                                        self.frames.push(CallFrame {
                                            ip: 0,
                                            closure: method_ref,
                                            locals_idx: receiver_pos,
                                        });
                                        self.frame_idx += 1;
                                        continue;
                                    } else {
                                        // Neither field nor method found
                                        let name = self.get_heap_string(name_ref).to_string();
                                        return Err(VirtualMachineError::RuntimeError {
                                            kind: RuntimeErrorKind::Unexpected {
                                                error: anyhow::Error::msg(format!("Undefined property '{}'.", name)),
                                            },
                                            stacktrace: self.stacktrace(),
                                        });
                                    }
                                } else {
                                    return Err(self.unhandled_error("Instance's klass is not a class"));
                                }
                            }
                        } else {
                            // Not an instance
                            return Err(VirtualMachineError::RuntimeError {
                                kind: RuntimeErrorKind::Unexpected {
                                    error: anyhow::Error::msg("Only instances have properties."),
                                },
                                stacktrace: self.stacktrace(),
                            });
                        }
                    } else {
                        // Not an object
                        return Err(VirtualMachineError::RuntimeError {
                            kind: RuntimeErrorKind::Unexpected {
                                error: anyhow::Error::msg("Only instances have properties."),
                            },
                            stacktrace: self.stacktrace(),
                        });
                    }
                }
                OpCode::INHERIT => {
                    // Stack: [..., superclass, subclass]
                    let subclass_value = self.pop()?;
                    let superclass_value = self.peek()?;  // Leave on stack for "super" local

                    // Validate superclass is a class
                    let superclass_ref = match &superclass_value {
                        Value::Object(r) => *r,
                        _ => {
                            return Err(VirtualMachineError::RuntimeError {
                                kind: RuntimeErrorKind::Unexpected {
                                    error: anyhow::anyhow!("Superclass must be a class."),
                                },
                                stacktrace: self.stacktrace(),
                            });
                        }
                    };

                    if !matches!(self.heap.get(superclass_ref).kind, ObjKind::Class(_)) {
                        return Err(VirtualMachineError::RuntimeError {
                            kind: RuntimeErrorKind::Unexpected {
                                error: anyhow::anyhow!("Superclass must be a class."),
                            },
                            stacktrace: self.stacktrace(),
                        });
                    }

                    // Copy methods from superclass to subclass
                    let methods_to_copy: Vec<(GcRef, GcRef)> = {
                        if let ObjKind::Class(superclass) = &self.heap.get(superclass_ref).kind {
                            superclass.methods.iter().map(|(k, v)| (*k, *v)).collect()
                        } else {
                            vec![]
                        }
                    };

                    let subclass_ref = match subclass_value {
                        Value::Object(r) => r,
                        _ => {
                            return Err(self.unhandled_error("Expected class object"));
                        }
                    };

                    let subclass = self.heap.get_mut(subclass_ref);
                    if let ObjKind::Class(class) = &mut subclass.kind {
                        for (name, method) in methods_to_copy {
                            // Only insert if subclass doesn't already have this method
                            class.methods.entry(name).or_insert(method);
                        }
                    }
                }
                OpCode::GET_SUPER(name_const_idx) => {
                    // Stack: [..., receiver (this), superclass]
                    let name_value = self.constants[name_const_idx as usize].clone();
                    let name_ref = match name_value {
                        Value::Object(r) => r,
                        _ => return Err(self.unhandled_error("Expected string constant for method name")),
                    };

                    let superclass_value = self.pop()?;
                    let receiver = self.pop()?;

                    let superclass_ref = match superclass_value {
                        Value::Object(r) => r,
                        _ => return Err(self.unhandled_error("Expected superclass")),
                    };

                    // Look up method in superclass (NOT in instance's class)
                    let method_ref = {
                        let obj = self.heap.get(superclass_ref);
                        if let ObjKind::Class(class) = &obj.kind {
                            class.methods.get(&name_ref).copied()
                        } else {
                            None
                        }
                    };

                    match method_ref {
                        Some(closure_ref) => {
                            // Create bound method with receiver
                            let bound_ref = self.alloc_bound_method(receiver, closure_ref);
                            self.push(Value::Object(bound_ref));
                        }
                        None => {
                            let name = self.get_heap_string(name_ref).to_string();
                            let frame = &self.frames[self.frame_idx];
                            let closure = self.get_heap_closure(frame.closure);
                            let function = self.get_heap_function(closure.function);
                            let line = function.code.line(frame.ip());
                            return Err(VirtualMachineError::RuntimeError {
                                kind: RuntimeErrorKind::Unexpected {
                                    error: anyhow::anyhow!("Undefined property '{}'.", name),
                                },
                                stacktrace: format!("[line {}]", line),
                            });
                        }
                    }
                }
                OpCode::SUPER_INVOKE(name_const_idx, arg_count) => {
                    // Stack: [receiver, arg1, ..., argN, superclass]
                    let arg_count = arg_count as usize;
                    let name_value = self.constants[name_const_idx as usize].clone();
                    let name_ref = match name_value {
                        Value::Object(r) => r,
                        _ => return Err(self.unhandled_error("Expected string constant")),
                    };

                    let superclass_value = self.pop()?;
                    let superclass_ref = match superclass_value {
                        Value::Object(r) => r,
                        _ => return Err(self.unhandled_error("Expected superclass")),
                    };

                    // Look up method in superclass
                    let method_ref = {
                        let obj = self.heap.get(superclass_ref);
                        if let ObjKind::Class(class) = &obj.kind {
                            class.methods.get(&name_ref).copied()
                        } else {
                            None
                        }
                    };

                    match method_ref {
                        Some(closure_ref) => {
                            // Get the function from closure for arity check
                            let func_ref = self.get_heap_closure(closure_ref).function;
                            let arity = self.get_heap_function(func_ref).arity;

                            if arg_count != arity {
                                let frame = &self.frames[self.frame_idx];
                                let closure = self.get_heap_closure(frame.closure);
                                let function = self.get_heap_function(closure.function);
                                let line = function.code.line(frame.ip());
                                return Err(VirtualMachineError::RuntimeError {
                                    kind: RuntimeErrorKind::InvalidFunArity {
                                        got: arg_count,
                                        expected: arity,
                                    },
                                    stacktrace: format!("[line {}]", line),
                                });
                            }

                            // Check frame limit
                            if self.frames.len() >= FRAMES_MAX {
                                return Err(VirtualMachineError::RuntimeError {
                                    kind: RuntimeErrorKind::StackOverflow,
                                    stacktrace: self.stacktrace(),
                                });
                            }

                            // Advance IP before pushing frame (matches INVOKE pattern)
                            self.frames[self.frame_idx].inc_ip(size);

                            // Push new call frame
                            // Receiver is at stack position: len - arg_count - 1
                            let locals_idx = self.stack.len() - arg_count - 1;
                            self.frames.push(CallFrame {
                                closure: closure_ref,
                                ip: 0,
                                locals_idx,
                            });
                            self.frame_idx += 1;

                            continue;
                        }
                        None => {
                            let name = self.get_heap_string(name_ref).to_string();
                            let frame = &self.frames[self.frame_idx];
                            let closure = self.get_heap_closure(frame.closure);
                            let function = self.get_heap_function(closure.function);
                            let line = function.code.line(frame.ip());
                            return Err(VirtualMachineError::RuntimeError {
                                kind: RuntimeErrorKind::Unexpected {
                                    error: anyhow::anyhow!("Undefined property '{}'.", name),
                                },
                                stacktrace: format!("[line {}]", line),
                            });
                        }
                    }
                }
            }
            self.frames[self.frame_idx].inc_ip(size);
        }

        Ok(())
    }

    pub fn patch_offset(&mut self, code: &mut ByteCode, addr: usize, new_offset: JumpOffset) {
        code.patch_offset(addr, new_offset);
    }

    fn compile_declaration_from_stack<'a>(
        &mut self,
        ident: &'a AstIdent,
        context: &mut LexicalScope<'a>,
        result: &mut ByteCode,
    ) -> Result<(), VirtualMachineError> {
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

            crate::parser::Stmt::Declarations(StmtDeclaration::Function(FunDeclaration {
                name,
                params,
                body,
            })) => {
                if context.is_already_declared(&name.node()) {
                    return Err(VirtualMachineError::CompileError(vec![
                        ParseError::VariableAlreadyDeclared { span: name.span },
                    ]));
                }

                let mut byte_code = ByteCode::default();
                context.function_decl(name.node(), params.iter().map(|x| x.node().as_str()));

                self.compile_statement(body, context, &mut byte_code)?;
                byte_code.write_op(OpCode::NIL, self.lookup_source_line(body.end()));
                byte_code.write_op(OpCode::RET, self.lookup_source_line(body.end()));

                let upvalue_count = context.upvalues.len();

                // Create function on heap
                let name_ref = self.alloc_string(name.node.clone());
                let function_ref = self.alloc_function(name_ref, params.len(), byte_code, upvalue_count);

                // Store function ref in constants table and use index in CLOSURE opcode
                let function_const_idx = self.constants.len() as u16;
                self.constants.push(Value::Object(function_ref));

                result.write_op(
                    OpCode::CLOSURE(function_const_idx),
                    self.lookup_source_line(name.start()),
                );
                for (_, up) in context.upvalues.iter() {
                    up.write_to(result, self.lookup_source_line(name.start()));
                }
                context.leave_function();

                self.compile_declaration_from_stack(name, context, result)?;
                Ok(())
            }

            crate::parser::Stmt::Declarations(StmtDeclaration::Variable { ident, expr }) => {
                if let Some(e) = expr {
                    self.compile_expr(e, context, result)?;
                } else {
                    result.write_op(OpCode::NIL, self.lookup_source_line(ident.start()));
                }
                self.compile_declaration_from_stack(ident, context, result)
            }
            crate::parser::Stmt::Declarations(StmtDeclaration::Class(ClassDeclaration { name, methods, superclass })) => {
                let has_superclass = superclass.is_some();
                // Emit CLASS opcode with name constant index
                let name_ref = self.alloc_string(name.node.clone());
                let name_const_idx = self.constants.len() as u16;
                self.constants.push(Value::Object(name_ref));
                result.write_op(
                    OpCode::CLASS(name_const_idx),
                    self.lookup_source_line(name.start()),
                );
                // Define class as variable (same as functions)
                self.compile_declaration_from_stack(name, context, result)?;

                // Handle inheritance if superclass exists
                if let Some(ref super_ident) = superclass {
                    // Load superclass onto stack
                    if context.is_toplevel() {
                        let super_name_idx = self.interner.intern_str(super_ident.node());
                        result.write_op(
                            OpCode::GETGLOBAL(super_name_idx),
                            self.lookup_source_line(super_ident.start()),
                        );
                    } else if let Some(offset) = context.resolve_local(super_ident.node()) {
                        result.write_op(
                            OpCode::GETLOCAL(offset as u16),
                            self.lookup_source_line(super_ident.start()),
                        );
                    } else if let Some((idx, _)) = context.resolve_upvalue(super_ident.node()) {
                        result.write_op(
                            OpCode::GETUPVALUE(idx as u16),
                            self.lookup_source_line(super_ident.start()),
                        );
                    } else {
                        let super_name_idx = self.interner.intern_str(super_ident.node());
                        result.write_op(
                            OpCode::GETGLOBAL(super_name_idx),
                            self.lookup_source_line(super_ident.start()),
                        );
                    }

                    // Load subclass onto stack
                    if context.is_toplevel() {
                        let class_name_idx = self.interner.intern_str(name.node());
                        result.write_op(
                            OpCode::GETGLOBAL(class_name_idx),
                            self.lookup_source_line(name.start()),
                        );
                    } else if let Some(offset) = context.resolve_local(name.node()) {
                        result.write_op(
                            OpCode::GETLOCAL(offset as u16),
                            self.lookup_source_line(name.start()),
                        );
                    }

                    // Emit INHERIT opcode
                    result.write_op(
                        OpCode::INHERIT,
                        self.lookup_source_line(super_ident.start()),
                    );

                    // Enter block for "super" local (superclass is now on stack top after INHERIT)
                    context.enter_block();
                    context.new_local("super");
                }

                // Compile each method
                for method in methods {
                    // Load class back onto stack for METHOD opcode
                    // Check if class is a local or global (need to handle the case where
                    // we're inside a synthetic block for "super" but the class is global)
                    if let Some(offset) = context.resolve_local(name.node()) {
                        result.write_op(
                            OpCode::GETLOCAL(offset as u16),
                            self.lookup_source_line(method.name.start()),
                        );
                    } else {
                        // Class is a global variable
                        let class_name_idx = self.interner.intern_str(name.node());
                        result.write_op(
                            OpCode::GETGLOBAL(class_name_idx),
                            self.lookup_source_line(method.name.start()),
                        );
                    }

                    // Compile the method body
                    let is_init = method.name.node() == "init";
                    let mut byte_code = ByteCode::default();
                    context.method_decl(
                        method.name.node(),
                        method.params.iter().map(|x| x.node().as_str()),
                        is_init,
                        has_superclass,
                    );

                    self.compile_statement(&method.body, context, &mut byte_code)?;

                    // Emit implicit return
                    // For initializers: return 'this' (slot 0)
                    // For regular methods: return nil
                    if is_init {
                        byte_code.write_op(OpCode::GETLOCAL(0), self.lookup_source_line(method.body.end()));
                    } else {
                        byte_code.write_op(OpCode::NIL, self.lookup_source_line(method.body.end()));
                    }
                    byte_code.write_op(OpCode::RET, self.lookup_source_line(method.body.end()));

                    let upvalue_count = context.upvalues.len();

                    // Create function on heap
                    let method_name_ref = self.alloc_string(method.name.node.clone());
                    let function_ref = self.alloc_function(
                        method_name_ref,
                        method.params.len(),
                        byte_code,
                        upvalue_count,
                    );

                    // Store function ref in constants table and use index in CLOSURE opcode
                    let function_const_idx = self.constants.len() as u16;
                    self.constants.push(Value::Object(function_ref));

                    result.write_op(
                        OpCode::CLOSURE(function_const_idx),
                        self.lookup_source_line(method.name.start()),
                    );
                    for (_, up) in context.upvalues.iter() {
                        up.write_to(result, self.lookup_source_line(method.name.start()));
                    }
                    context.leave_function();

                    // Allocate method name string for METHOD opcode
                    let method_name_const_ref = self.alloc_string(method.name.node.clone());
                    let method_name_const_idx = self.constants.len() as u16;
                    self.constants.push(Value::Object(method_name_const_ref));

                    // Emit METHOD opcode
                    result.write_op(
                        OpCode::METHOD(method_name_const_idx),
                        self.lookup_source_line(method.name.start()),
                    );
                }

                // Close "super" local if we had a superclass
                if superclass.is_some() {
                    // Check if any method captured "super"
                    let (_, captured) = context.iter_locals_in_block_rev().next().unwrap_or(("", false));
                    if captured {
                        result.write_op(OpCode::CLOSEUPVALUE, self.lookup_source_line(name.start()));
                    } else {
                        result.write_op(OpCode::POP, self.lookup_source_line(name.start()));
                    }
                    context.leave_block();
                }

                Ok(())
            }
            crate::parser::Stmt::Block(block) => {
                context.enter_block();
                for each_stmt in block.0.iter() {
                    self.compile_statement(each_stmt, context, result)?;
                }
                for (_, captured) in context.iter_locals_in_block_rev() {
                    if captured {
                        result.write_op(OpCode::CLOSEUPVALUE, self.lookup_source_line(stmt.end()));
                    } else {
                        result.write_op(OpCode::POP, self.lookup_source_line(stmt.end()));
                    }
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
            crate::parser::Stmt::Return(ret, value) => {
                if context.is_toplevel() {
                    return Err(VirtualMachineError::CompileError(vec![
                        ParseError::InvalidTopLevelReturn { span: ret.span },
                    ]));
                }
                // Check for return with value inside initializer
                if context.function_type() == FunctionType::Initializer {
                    if value.is_some() {
                        return Err(VirtualMachineError::CompileError(vec![
                            ParseError::InitializerReturnValue { span: ret.span },
                        ]));
                    }
                    // Empty return in init returns 'this' (slot 0)
                    result.write_op(OpCode::GETLOCAL(0), self.lookup_source_line(stmt.start()));
                    result.write_op(OpCode::RET, self.lookup_source_line(stmt.start()));
                } else {
                    // Regular function/method return
                    if let Some(value) = value {
                        self.compile_expr(value, context, result)?;
                    } else {
                        result.write_op(OpCode::NIL, self.lookup_source_line(stmt.start()));
                    }
                    result.write_op(OpCode::RET, self.lookup_source_line(stmt.start()));
                }
                Ok(())
            }
        }
    }

    fn compile_assignment_from_stack<'a>(
        &mut self,
        name: &'a str,
        span: Span,
        context: &mut LexicalScope<'a>,
        result: &mut ByteCode,
    ) -> Result<(), VirtualMachineError> {
        if let Some(offset) = context.resolve_local(name) {
            result.write_op(
                OpCode::SETLOCAL(offset as u16),
                self.lookup_source_line(span.start()),
            )
        } else {
            match context.resolve_upvalue(name) {
                Some((idx, _)) => {
                    result.write_op(
                        OpCode::SETUPVALUE(idx as u16),
                        self.lookup_source_line(span.start()),
                    );
                }
                _ => {
                    let idx = self.interner.intern_str(&name);
                    result.write_op(
                        OpCode::SETGLOBAL(idx),
                        self.lookup_source_line(span.start()),
                    );
                }
            }
        }
        Ok(())
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
                    self.compile_assignment_from_stack(&name, expr.span, context, result)?;
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
                let gc_ref = self.alloc_string(s.to_string());
                let val = Value::Object(gc_ref);
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
            crate::parser::Expression::Divide { left, right } => {
                self.compile_expr(&left.as_ref(), context, result)?;
                self.compile_expr(&right.as_ref(), context, result)?;
                result.write_op(OpCode::DIVIDE, self.lookup_source_line(left.start()));
            }
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
            crate::parser::Expression::Logical(LogicalExpression::GreaterEqual { left, right }) => {
                // a >= b is equivalent to !(a < b)
                self.compile_expr(&right, context, result)?;
                self.compile_expr(&left, context, result)?;
                result.write_op(OpCode::LESS, self.lookup_source_line(left.start()));
                result.write_op(OpCode::NOT, self.lookup_source_line(left.start()));
            }
            crate::parser::Expression::Logical(LogicalExpression::LessEqual { left, right }) => {
                // a <= b is equivalent to !(a > b)
                self.compile_expr(&left, context, result)?;
                self.compile_expr(&right, context, result)?;
                result.write_op(OpCode::GREATER, self.lookup_source_line(left.start()));
                result.write_op(OpCode::NOT, self.lookup_source_line(left.start()));
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
                let line = self.lookup_source_line(calee.start());

                // Check for method invoke pattern: obj.method(args)
                if let Expression::GetProperty { object, name } = calee.node() {
                    // Compile receiver (pushes instance)
                    self.compile_expr(object, context, result)?;

                    // Compile arguments
                    for arg in arguments {
                        self.compile_expr(arg, context, result)?;
                    }

                    // Emit INVOKE with method name and arg count
                    let name_ref = self.alloc_string(name.clone());
                    let name_idx = self.constants.len() as u16;
                    self.constants.push(Value::Object(name_ref));
                    result.write_op(OpCode::INVOKE(name_idx, arguments.len() as u8), line);
                } else {
                    // Regular call - existing logic
                    self.compile_expr(&calee, context, result)?;
                    for arg in arguments {
                        self.compile_expr(arg, context, result)?;
                    }
                    result.write_op(OpCode::CALL(arguments.len() as u8), line);
                }
            }
            crate::parser::Expression::GetProperty { object, name } => {
                // Compile the object expression (pushes instance onto stack)
                self.compile_expr(object, context, result)?;
                // Allocate property name string and add to constants
                let name_ref = self.alloc_string(name.clone());
                let name_const_idx = self.constants.len() as u16;
                self.constants.push(Value::Object(name_ref));
                // Emit GET_PROPERTY opcode with constant index
                result.write_op(
                    OpCode::GET_PROPERTY(name_const_idx),
                    self.lookup_source_line(expr.start()),
                );
            }
            crate::parser::Expression::SetProperty { object, name, value } => {
                // Compile the object expression (pushes instance onto stack)
                self.compile_expr(object, context, result)?;
                // Compile the value expression (pushes value onto stack)
                self.compile_expr(value, context, result)?;
                // Allocate property name string and add to constants
                let name_ref = self.alloc_string(name.clone());
                let name_const_idx = self.constants.len() as u16;
                self.constants.push(Value::Object(name_ref));
                // Emit SET_PROPERTY opcode with constant index
                result.write_op(
                    OpCode::SET_PROPERTY(name_const_idx),
                    self.lookup_source_line(expr.start()),
                );
            }
            Expression::This => {
                let line = self.lookup_source_line(expr.start());

                // Check if inside a class
                if !context.in_class() {
                    return Err(VirtualMachineError::CompileError(vec![
                        ParseError::UnexpectedToken {
                            span: expr.span,
                            expectation: "Can't use 'this' outside of a class.".to_owned(),
                        },
                    ]));
                }

                // 'this' is always at slot 0 in method scope or via upvalue
                if let Some(offset) = context.resolve_local("this") {
                    result.write_op(OpCode::GETLOCAL(offset as u16), line);
                } else if let Some((idx, _)) = context.resolve_upvalue("this") {
                    result.write_op(OpCode::GETUPVALUE(idx as u16), line);
                } else {
                    // Should not happen if enclosing_class is set correctly
                    panic!("'this' not found in method scope");
                }
            }
            Expression::Super { method } => {
                let line = self.lookup_source_line(expr.start());
                // Create span just for "super" keyword (5 characters)
                let super_span = Span::new(expr.start(), expr.start() + 5);

                // Check compile-time errors
                if !context.in_class() {
                    return Err(VirtualMachineError::CompileError(vec![
                        ParseError::SuperOutsideClass { span: super_span },
                    ]));
                }
                if !context.has_superclass() {
                    return Err(VirtualMachineError::CompileError(vec![
                        ParseError::SuperWithoutSuperclass { span: super_span },
                    ]));
                }

                // Load 'this' (receiver) onto stack
                if let Some(offset) = context.resolve_local("this") {
                    result.write_op(OpCode::GETLOCAL(offset as u16), line);
                } else if let Some((idx, _)) = context.resolve_upvalue("this") {
                    result.write_op(OpCode::GETUPVALUE(idx as u16), line);
                } else {
                    panic!("'this' not found in method scope");
                }

                // Load 'super' (superclass) onto stack
                if let Some(offset) = context.resolve_local("super") {
                    result.write_op(OpCode::GETLOCAL(offset as u16), line);
                } else if let Some((idx, _)) = context.resolve_upvalue("super") {
                    result.write_op(OpCode::GETUPVALUE(idx as u16), line);
                } else {
                    panic!("'super' not found in scope");
                }

                // Emit GET_SUPER with method name constant
                let method_name_ref = self.alloc_string(method.clone());
                let method_const_idx = self.constants.len() as u16;
                self.constants.push(Value::Object(method_name_ref));

                result.write_op(OpCode::GET_SUPER(method_const_idx), line);
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

    fn unhandled_error(&self, error: impl Into<String>) -> VirtualMachineError {
        VirtualMachineError::RuntimeError {
            kind: RuntimeErrorKind::Unexpected {
                error: anyhow::Error::msg(error.into()),
            },
            stacktrace: self.stacktrace(),
        }
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

/// Distinguishes different function compilation contexts.
/// Used to handle slot 0 and return semantics correctly.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FunctionType {
    Script,      // Top-level script
    Function,    // Regular function declaration
    Method,      // Class method
    Initializer, // init() method
}

/// Context for tracking class compilation state
#[derive(Clone, Copy, Debug)]
pub struct ClassContext {
    pub has_superclass: bool,
}

pub struct LexicalScope<'a> {
    parent: Option<Box<LexicalScope<'a>>>,
    args: Vec<(&'a str, bool)>,
    locals: Vec<(&'a str, usize, bool)>,
    upvalues: Vec<(&'a str, UpvalueRef)>,
    block_depth: usize,
    name: Option<String>,
    function_type: FunctionType,
    enclosing_class: Option<ClassContext>, // Some if inside a class body
}

impl<'a> LexicalScope<'a> {
    pub fn name(&self) -> &str {
        self.name.as_deref().unwrap_or("<script>")
    }

    pub fn is_toplevel(&self) -> bool {
        self.block_depth == 0
    }

    pub fn function_type(&self) -> FunctionType {
        self.function_type
    }

    pub fn root() -> Self {
        Self {
            parent: None,
            args: Default::default(),
            locals: Default::default(),
            block_depth: 0,
            upvalues: Default::default(),
            name: None,
            function_type: FunctionType::Script,
            enclosing_class: None,
        }
    }

    pub fn function_decl(&mut self, name: &'a str, args: impl IntoIterator<Item = &'a str>) {
        let enclosing_class = self.enclosing_class;
        let new = Self {
            parent: None,
            args: {
                let mut a = Vec::new();
                a.push((name, false));
                a.extend(args.into_iter().map(|x| (x, false)));
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
            function_type: FunctionType::Function,
            enclosing_class,
        };

        let prev = mem::replace(self, new);
        self.parent = Some(Box::new(prev));
    }

    /// Enter a method compilation context.
    /// Unlike function_decl, slot 0 is "this" instead of the function name.
    pub fn method_decl(&mut self, name: &'a str, args: impl IntoIterator<Item = &'a str>, is_init: bool, has_superclass: bool) {
        let function_type = if is_init {
            FunctionType::Initializer
        } else {
            FunctionType::Method
        };

        let new = Self {
            parent: None,
            args: {
                let mut a = Vec::new();
                // Slot 0 is "this" for methods (instead of function name)
                a.push(("this", false));
                a.extend(args.into_iter().map(|x| (x, false)));
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
            function_type,
            enclosing_class: Some(ClassContext { has_superclass }),
        };

        let prev = mem::replace(self, new);
        self.parent = Some(Box::new(prev));
    }

    /// Check if currently inside a class body
    pub fn in_class(&self) -> bool {
        self.enclosing_class.is_some()
    }

    /// Check if the enclosing class has a superclass
    pub fn has_superclass(&self) -> bool {
        self.enclosing_class.map_or(false, |ctx| ctx.has_superclass)
    }

    /// Set the class context for method compilation
    pub fn set_class_context(&mut self, has_superclass: bool) {
        self.enclosing_class = Some(ClassContext { has_superclass });
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
            function_type,
            enclosing_class,
        } = *parent;
        self.args = args;
        self.block_depth = block_depth;
        self.locals = locals;
        self.parent = parent;
        self.upvalues = upvalues;
        self.name = name;
        self.function_type = function_type;
        self.enclosing_class = enclosing_class;
    }

    pub fn enter_block(&mut self) {
        self.block_depth += 1;
    }

    pub fn leave_block(&mut self) {
        self.block_depth -= 1;

        loop {
            if let Some((_, d, _)) = self.locals.last() {
                if *d > self.block_depth {
                    self.locals.pop();
                    continue;
                }
            }
            break;
        }
    }

    pub fn new_local(&mut self, name: &'a str) -> usize {
        self.locals.push((name, self.block_depth, false));
        self.args.len() + self.locals.len() - 1
    }

    pub fn iter_local_upvalues(&self) -> impl Iterator<Item = (&str, usize)> {
        self.upvalues
            .iter()
            .filter_map(|(name, value)| match value {
                UpvalueRef::Local(idx) => Some((*name, *idx)),
                _ => None,
            })
    }

    pub fn iter_locals_in_block_rev(&self) -> impl Iterator<Item = (&str, bool)> {
        self.locals
            .iter()
            .rev()
            .take_while(|(_, depth, _)| *depth == self.block_depth)
            .map(|(name, _, c)| (*name, *c))
    }

    pub fn is_already_declared(&self, name: &'a str) -> bool {
        for (arg_name, _) in self.args.iter() {
            if arg_name.eq(&name) {
                return true;
            }
        }

        for i in (0..self.locals.len()).rev() {
            let (n, d, _) = self.locals[i];
            if d < self.block_depth {
                break;
            }
            if n.eq(name) {
                return true;
            }
        }

        return false;
    }

    fn capture_local(&mut self, name: &str) -> Option<usize> {
        for i in 0..self.args.len() {
            let (n, _) = self.args[i];
            if n.eq(name) {
                self.args[i].1 = true;
                return Some(i);
            }
        }

        for i in (0..self.locals.len()).rev() {
            let (n, _, _) = self.locals[i];
            if n.eq(name) {
                self.locals[i].2 = true;
                //self.upvalues
                //  .push((n, UpvalueRef::Local(self.args.len() + i)));
                return Some(self.args.len() + i);
            }
        }

        None
    }

    pub fn resolve_local(&self, name: &str) -> Option<usize> {
        for (i, (arg_name, _)) in self.args.iter().enumerate() {
            if arg_name.eq(&name) {
                return Some(i);
            }
        }

        for i in (0..self.locals.len()).rev() {
            let (n, _, _) = self.locals[i];
            if n.eq(name) {
                return Some(self.args.len() + i);
            }
        }
        None
    }

    pub fn resolve_upvalue(&mut self, name: &'a str) -> Option<(usize, UpvalueRef)> {
        if let Some((i, (_, value))) = self
            .upvalues
            .iter()
            .enumerate()
            .find(|(_, (n, _))| n == &name)
        {
            return Some((i, value.to_owned()));
        }

        if let Some(local_idx) = self.parent.as_mut().and_then(|x| x.capture_local(name)) {
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

pub struct DisplayError<'a> {
    pub err: &'a VirtualMachineError,
    pub code: &'a str,
    pub codemap: &'a Codemap,
}

impl<'a> DisplayError<'a> {
    fn report_error_with_span(
        code: &str,
        codemap: &Codemap,
        f: &mut Formatter<'_>,
        span: &Span,
        error: impl Display,
    ) -> std::fmt::Result {
        writeln!(
            f,
            "{}Error at '{}': {}",
            {
                if let Some(line) = codemap.line_at(span.start()) {
                    format!("[line {}] ", line)
                } else {
                    String::from("")
                }
            },
            span.slice(code),
            error
        )
    }

    fn report_error_at_end(
        codemap: &Codemap,
        f: &mut Formatter<'_>,
        last_position: usize,
        error: impl Display,
    ) -> std::fmt::Result {
        let line = codemap.line_at(last_position).unwrap_or(1);
        write!(f, "[line {}] Error at end: {}", line, error)
    }
}

impl<'a> Display for DisplayError<'a> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self.err {
            VirtualMachineError::CompileError(parsing_errors) => {
                for error in parsing_errors.iter() {
                    match error {
                        ParseError::VariableAlreadyDeclared { span } => {
                            Self::report_error_with_span(self.code, self.codemap, f, span, error)?
                        }
                        ParseError::UnexpectedToken { span, .. } => {
                            Self::report_error_with_span(self.code, self.codemap, f, span, error)?
                        }
                        ParseError::InvalidVariableName { span } => {
                            Self::report_error_with_span(self.code, self.codemap, f, span, error)?
                        }
                        ParseError::InvalidAssignmentTarget { span } => {
                            Self::report_error_with_span(self.code, self.codemap, f, span, error)?
                        }
                        ParseError::InvalidTopLevelReturn { span } => {
                            Self::report_error_with_span(self.code, self.codemap, f, span, error)?
                        }
                        ParseError::ExpectedExpression { span } => {
                            Self::report_error_with_span(self.code, self.codemap, f, span, error)?
                        }
                        ParseError::MaxFunCallArguments { span } => {
                            Self::report_error_with_span(self.code, self.codemap, f, span, error)?
                        }
                        ParseError::MaxFunDeclarationParameters { span } => {
                            Self::report_error_with_span(self.code, self.codemap, f, span, error)?
                        }
                        ParseError::InitializerReturnValue { span } => {
                            Self::report_error_with_span(self.code, self.codemap, f, span, error)?
                        }
                        ParseError::SuperOutsideClass { span } => {
                            Self::report_error_with_span(self.code, self.codemap, f, span, error)?
                        }
                        ParseError::SuperWithoutSuperclass { span } => {
                            Self::report_error_with_span(self.code, self.codemap, f, span, error)?
                        }
                        ParseError::UnexpectedEof { last_position } => {
                            Self::report_error_at_end(self.codemap, f, *last_position, "Expect expression.")?
                        }
                        _ => writeln!(f, "{}", error)?,
                    }
                }
            }
            _ => writeln!(f, "{}", self.err)?,
        }
        Ok(())
    }
}

impl<'a> Display for DispayValue<'a> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self.value {
            Value::Number(x) => {
                // Match clox %g behavior: integers print without decimal point
                if x.fract() == 0.0 && x.abs() < 1e15 {
                    write!(f, "{}", x as i64)
                } else {
                    write!(f, "{}", x)
                }
            }
            Value::Bool(x) => write!(f, "{}", x),
            Value::Nil => f.write_str("nil"),
            // Value::Closure - REMOVED: Closures are now Value::Object(GcRef)
            Value::Object(gc_ref) => {
                // Display based on object type in the heap
                let obj = self.vm.heap.get(gc_ref);
                match &obj.kind {
                    ObjKind::String(s) => f.write_str(&s.value),
                    ObjKind::Closure(c) => {
                        // Get the function name from the closure's function
                        let func = self.vm.heap.get(c.function);
                        if let ObjKind::Function(func_obj) = &func.kind {
                            let name_obj = self.vm.heap.get(func_obj.name);
                            if let ObjKind::String(name_str) = &name_obj.kind {
                                write!(f, "<fn {}>", name_str.value)
                            } else {
                                write!(f, "<fn>")
                            }
                        } else {
                            write!(f, "<fn>")
                        }
                    }
                    ObjKind::Function(func) => {
                        let name_obj = self.vm.heap.get(func.name);
                        if let ObjKind::String(name_str) = &name_obj.kind {
                            write!(f, "<fn {}>", name_str.value)
                        } else {
                            write!(f, "<fn>")
                        }
                    }
                    ObjKind::Upvalue(_) => write!(f, "<upvalue>"),
                    ObjKind::Class(c) => {
                        // Print class name only
                        let name_obj = self.vm.heap.get(c.name);
                        if let ObjKind::String(name_str) = &name_obj.kind {
                            f.write_str(&name_str.value)
                        } else {
                            write!(f, "<class>")
                        }
                    }
                    ObjKind::Instance(i) => {
                        // Print "{ClassName} instance"
                        let klass_obj = self.vm.heap.get(i.klass);
                        if let ObjKind::Class(klass) = &klass_obj.kind {
                            let name_obj = self.vm.heap.get(klass.name);
                            if let ObjKind::String(name_str) = &name_obj.kind {
                                write!(f, "{} instance", name_str.value)
                            } else {
                                write!(f, "<instance>")
                            }
                        } else {
                            write!(f, "<instance>")
                        }
                    }
                    ObjKind::BoundMethod(bm) => {
                        // Print bound method same as closure: "<fn methodName>"
                        let method_obj = self.vm.heap.get(bm.method);
                        if let ObjKind::Closure(c) = &method_obj.kind {
                            let func = self.vm.heap.get(c.function);
                            if let ObjKind::Function(func_obj) = &func.kind {
                                let name_obj = self.vm.heap.get(func_obj.name);
                                if let ObjKind::String(name_str) = &name_obj.kind {
                                    write!(f, "<fn {}>", name_str.value)
                                } else {
                                    write!(f, "<fn>")
                                }
                            } else {
                                write!(f, "<fn>")
                            }
                        } else {
                            write!(f, "<fn>")
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use crate::vm::{LexicalScope, UpvalueRef};

    #[test]
    fn test_blocks() {
        let mut scope = LexicalScope::root();
        scope.enter_block();
        scope.new_local("y");
        scope.enter_block();
        scope.new_local("z");
        scope.new_local("y");
        assert_eq!(
            scope.iter_locals_in_block_rev().collect::<Vec<_>>(),
            vec![("y", false), ("z", false)]
        );
    }

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
        scope.function_decl("inner", ["y"]);
        assert_eq!(
            scope.resolve_upvalue("x"),
            Some((0, crate::vm::UpvalueRef::Local(1)))
        );
        scope.leave_function();
        assert_eq!(scope.upvalues.iter().cloned().collect::<Vec<_>>(), vec![]);
        scope.leave_function();
        assert_eq!(scope.upvalues.iter().cloned().collect::<Vec<_>>(), vec![]);
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

    #[test]
    fn test_closing_upvalues() {
        let mut scope = LexicalScope::root();
        scope.enter_block();
        scope.new_local("a");
        scope.new_local("b");

        scope.function_decl("add", ["x"]);
        scope.enter_block();

        scope.function_decl("p", []);
        scope.enter_block();
        scope.resolve_upvalue("x");
        scope.resolve_upvalue("a");
        scope.resolve_upvalue("b");
        assert_eq!(
            scope.upvalues.iter().cloned().collect::<Vec<_>>(),
            vec![
                ("x", UpvalueRef::Local(1)),
                ("a", UpvalueRef::Upvalue(0)),
                ("b", UpvalueRef::Upvalue(1))
            ]
        );
        scope.leave_block();
        scope.leave_function();
        scope.new_local("p");
        scope.resolve_local("p");
        assert_eq!(
            scope.iter_local_upvalues().collect::<Vec<_>>(),
            vec![("a", 0), ("b", 1)]
        );
        assert_eq!(
            scope.upvalues.iter().cloned().collect::<Vec<_>>(),
            vec![("a", UpvalueRef::Local(0)), ("b", UpvalueRef::Local(1))]
        );
        scope.leave_block();
        scope.leave_function();
        assert_eq!(
            scope.iter_locals_in_block_rev().collect::<Vec<_>>(),
            vec![("b", true), ("a", true)]
        );
    }

    #[test]
    fn test_fun_captures_in_block() {
        let mut scope = LexicalScope::root();
        scope.enter_block();
        scope.new_local("isEven");
        scope.function_decl("isOdd", ["n"]);
        scope.enter_block();
        assert_eq!(
            scope.resolve_upvalue("isEven"),
            Some((0, UpvalueRef::Local(0)))
        )
    }

    #[test]
    fn test_gc_triggers() {
        use super::VirtualMachine;

        let mut vm = VirtualMachine::new();
        // Set low threshold to force GC
        let initial_threshold = 1024;
        vm.heap.next_gc = initial_threshold;

        // Allocate many strings to trigger GC
        for i in 0..100 {
            let s = format!("string_{}", i);
            vm.alloc_string(s);
        }

        // Verify GC ran by checking threshold was updated
        // After GC, next_gc should be > initial_threshold (doubled)
        assert!(
            vm.heap.next_gc > initial_threshold,
            "GC should have run and updated threshold. Expected > {}, got {}",
            initial_threshold,
            vm.heap.next_gc
        );

        // Also verify bytes_allocated is reasonable
        assert!(vm.heap.bytes_allocated > 0, "Should have allocated bytes");
    }
}
