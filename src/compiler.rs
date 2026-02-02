use std::rc::Rc;

use hashbrown::HashMap;

use crate::{
    byte_code::{ByteCode, JumpOffset, OpCode},
    codemap::{self, Codemap},
    interner::StrId,
    parser::{AstExpression, Expression, LogicalExpression, Stmt},
    value::Value,
    vm::{LexicalScope, UpvalueRef, VirtualMachine, VirtualMachineError},
};

pub struct Compiler {
    codemap: Codemap,
    function: Function,
}

pub struct Function {
    code: ByteCode,
    constants: Vec<Value>,
    arity: usize,
    upvalue_count: usize,
}

impl Compiler {
    pub fn new(codemap: Codemap) -> Self {
        Self { codemap: codemap }
    }

    fn lookup_source_line(&self, start: usize) -> usize {
        self.codemap.line_at(start).unwrap_or(0)
    }

    fn patch_offset(&mut self, code: &mut ByteCode, addr: usize, new_offset: JumpOffset) {
        code.patch_offset(addr, new_offset);
    }

    fn add_constant(&mut self, v: Value) {}

    pub fn next_instruction(&self, code: &ByteCode) -> usize {
        code.size()
    }

    pub fn offset_since(&self, ip: usize, code: &ByteCode) -> JumpOffset {
        JumpOffset::new(ip, self.next_instruction(code))
    }

    pub fn compile<'a>(
        &mut self,
        statements: Vec<Stmt>,
        scope: &LexicalScope<'a>,
    ) -> Result<Function, VirtualMachine> {
        todo!()
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
                            Some(UpvalueRef::Local(idx)) | Some(UpvalueRef::Upvalue(idx)) => {
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
                        Some(UpvalueRef::Local(idx) | UpvalueRef::Upvalue(idx)) => {
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
            crate::parser::Expression::Literal(crate::parser::AstLiteral::Number(num)) => {
                result.write_op(
                    OpCode::CONSTANT(self.constants.len() as u16),
                    self.lookup_source_line(expr.start()),
                );
                self.add_constant((*num).into());
            }
            crate::parser::Expression::Literal(crate::parser::AstLiteral::Str(s)) => {
                let val = Value::String(self.interner.intern_str(s));
                result.write_op(
                    OpCode::CONSTANT(self.constants.len() as u16),
                    self.lookup_source_line(expr.start()),
                );
                self.add_constant(val);
            }
            crate::parser::Expression::Literal(crate::parser::AstLiteral::Bool(x)) => {
                if *x {
                    result.write_op(OpCode::TRUE, self.lookup_source_line(expr.start()));
                } else {
                    result.write_op(OpCode::FALSE, self.lookup_source_line(expr.start()));
                }
            }
            crate::parser::Expression::Literal(crate::parser::AstLiteral::Nil) => {
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
}
