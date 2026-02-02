use std::{iter::repeat, mem::size_of};

use byteorder::{ByteOrder, LittleEndian};

use crate::interner::StrId;

#[derive(thiserror::Error, Debug)]
pub enum OpCodeError {
    #[error("Received incorrect opcode {opcode} that doesn't match any known opcode")]
    _InvalidOpCode { opcode: u8 },
}

#[derive(Default, Clone)]
pub struct ByteCode {
    code: Vec<u8>,
    lines: Vec<usize>,
}

pub struct JumpOffset(i16);

impl JumpOffset {
    pub fn new(start: usize, end: usize) -> Self {
        Self((end - start) as i16)
    }
}

impl From<JumpOffset> for i16 {
    fn from(value: JumpOffset) -> Self {
        return value.0;
    }
}

impl ByteCode {
    pub fn line(&self, ip: usize) -> usize {
        self.lines[ip]
    }

    pub fn size(&self) -> usize {
        self.code.len()
    }

    pub fn get_byte(&self, ip: usize) -> u8 {
        self.code[ip]
    }

    pub fn read_u16(&self, ip: usize) -> u16 {
        LittleEndian::read_u16(&self.code[ip..])
    }

    pub fn read_i16(&self, ip: usize) -> i16 {
        LittleEndian::read_i16(&self.code[ip..])
    }

    pub fn read_u8(&self, ip: usize) -> u8 {
        self.code[ip]
    }

    pub fn read_next(self: &ByteCode, ip: usize) -> Option<Result<(OpCode, usize), OpCodeError>> {
        if self.code.len() <= ip {
            return None;
        }
        let discriminant = self.get_byte(ip);
        let mut result = OpCode::from_repr(discriminant);
        let ip = ip + 1;
        let size = match &mut result {
            Some(OpCode::Constant(idx)) => {
                *idx = self.read_u16(ip);
                3
            }
            Some(OpCode::DeclareGlobal(idx)) => {
                *idx = self.read_u16(ip).into();
                3
            }
            Some(OpCode::SetGlobal(idx)) | Some(OpCode::GetGlobal(idx)) => {
                *idx = self.read_u16(ip).into();
                3
            }
            Some(OpCode::GetLocal(idx)) | Some(OpCode::SetLocal(idx)) => {
                *idx = self.read_u16(ip);
                3
            }
            Some(OpCode::GetUpvalue(idx))
            | Some(OpCode::SetUpvalue(idx))
            | Some(OpCode::Closure(idx))
            | Some(OpCode::Class(idx))
            | Some(OpCode::GetProperty(idx))
            | Some(OpCode::SetProperty(idx))
            | Some(OpCode::Method(idx)) => {
                *idx = self.read_u16(ip);
                3
            }
            Some(OpCode::JumpIfFalse(offset))
            | Some(OpCode::Jump(offset))
            | Some(OpCode::Loop(offset)) => {
                *offset = self.read_i16(ip);
                3
            }
            Some(OpCode::Call(arg_count)) => {
                *arg_count = self.code[ip];
                2
            }
            Some(OpCode::Invoke(name_idx, arg_count)) => {
                *name_idx = self.read_u16(ip);
                *arg_count = self.read_u8(ip + 2);
                4
            }
            Some(OpCode::GetSuper(idx)) => {
                *idx = self.read_u16(ip);
                3
            }
            Some(OpCode::SuperInvoke(name_idx, arg_count)) => {
                *name_idx = self.read_u16(ip);
                *arg_count = self.read_u8(ip + 2);
                4
            }
            _ => 1,
        };
        result.map(|x| Ok((x, size)))
    }

    pub fn patch_bytes<const N: usize>(&mut self, pos: usize, bytes: [u8; N]) {
        self.code.splice(pos..pos + N, bytes);
    }

    pub fn patch_offset(&mut self, pos: usize, offset: JumpOffset) {
        self.patch_bytes(pos, offset.0.to_le_bytes());
    }

    pub fn write_byte(&mut self, byte: u8, line: usize) {
        self.code.push(byte);
        self.lines.push(line);
    }

    pub fn write_u16(&mut self, byte: u16, line: usize) {
        self.code.extend(byte.to_le_bytes());
        self.lines.extend([line, line]);
    }

    pub fn write_op(&mut self, op: OpCode, line: usize) {
        fn write_one_u16(code: &mut Vec<u8>, param: u16) -> usize {
            let bytes = param.to_le_bytes();
            code.extend(bytes);
            size_of::<u16>()
        }

        fn write_one_i16(code: &mut Vec<u8>, param: i16) -> usize {
            let bytes = param.to_le_bytes();
            code.extend(bytes);
            size_of::<i16>()
        }

        self.code
            .push(unsafe { std::mem::transmute(std::mem::discriminant(&op)) });

        let bytes_count = match op {
            OpCode::Constant(idx) => write_one_u16(&mut self.code, idx),
            OpCode::DeclareGlobal(idx) => write_one_u16(&mut self.code, idx.as_u16()),
            OpCode::GetGlobal(idx) | OpCode::SetGlobal(idx) => {
                write_one_u16(&mut self.code, idx.as_u16())
            }
            OpCode::SetLocal(idx) | OpCode::GetLocal(idx) => write_one_u16(&mut self.code, idx),
            OpCode::GetUpvalue(idx) | OpCode::SetUpvalue(idx) => write_one_u16(&mut self.code, idx),
            OpCode::Closure(idx) => write_one_u16(&mut self.code, idx),
            OpCode::Class(idx) => write_one_u16(&mut self.code, idx),
            OpCode::GetProperty(idx) => write_one_u16(&mut self.code, idx),
            OpCode::SetProperty(idx) => write_one_u16(&mut self.code, idx),
            OpCode::Method(idx) => write_one_u16(&mut self.code, idx),
            OpCode::JumpIfFalse(offset) | OpCode::Jump(offset) | OpCode::Loop(offset) => {
                write_one_i16(&mut self.code, offset)
            }
            OpCode::Call(arg_count) => {
                self.code.push(arg_count as u8);
                1
            }
            OpCode::Invoke(name_idx, arg_count) => {
                write_one_u16(&mut self.code, name_idx);
                self.code.push(arg_count);
                3
            }
            OpCode::GetSuper(idx) => write_one_u16(&mut self.code, idx),
            OpCode::SuperInvoke(name_idx, arg_count) => {
                write_one_u16(&mut self.code, name_idx);
                self.code.push(arg_count);
                3
            }
            _ => 0,
        };
        self.lines.extend(repeat(line).take(bytes_count + 1));
    }
}

#[derive(Debug, strum::FromRepr, PartialEq, strum::AsRefStr, strum::EnumDiscriminants)]
#[strum_discriminants(name(OpCodeTypes), derive(strum::Display), derive(strum::FromRepr))]
#[repr(u8)]
pub enum OpCode {
    Constant(u16) = 1,
    GetGlobal(StrId),
    SetGlobal(StrId),
    SetLocal(u16),
    GetLocal(u16),
    GetUpvalue(u16),
    SetUpvalue(u16),
    Closure(u16),
    CloseUpvalue,
    DeclareGlobal(StrId),
    Jump(i16),
    JumpIfFalse(i16),
    Loop(i16),
    Call(u8),
    Ret,
    Pop,
    Add,
    Multiply,
    Subtract,
    Divide,
    Negate,
    Not,
    Print,
    True,
    False,
    Nil,
    Greater,
    Less,
    Equal,
    Class(u16),
    GetProperty(u16),
    SetProperty(u16),
    Method(u16),
    Invoke(u16, u8),      // (method_name_const_idx, arg_count)
    Inherit,              // No operands - stack: [superclass, subclass] -> [superclass]
    GetSuper(u16),        // u16 = method name constant index
    SuperInvoke(u16, u8), // u16 = method name, u8 = arg count
}

#[cfg(test)]
mod tests {
    use crate::byte_code::OpCode;

    use super::ByteCode;

    #[test]
    fn test_write_read_op() {
        let mut bytes = ByteCode::default();
        bytes.write_op(OpCode::Add, 1);
        bytes.write_op(OpCode::Constant(20), 2);
        bytes.write_op(OpCode::GetGlobal(30.into()), 3);
        let (x1, s1) = bytes.read_next(0).unwrap().unwrap();
        let (x2, s2) = bytes.read_next(s1).unwrap().unwrap();
        let (x3, _) = bytes.read_next(s1 + s2).unwrap().unwrap();
        assert_eq!(
            vec![x1, x2, x3],
            vec![
                OpCode::Add,
                OpCode::Constant(20),
                OpCode::GetGlobal(30.into())
            ]
        );
        assert_eq!(bytes.lines, vec![1, 2, 2, 2, 3, 3, 3])
    }
}
