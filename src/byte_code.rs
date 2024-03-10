use std::{iter::repeat, mem::size_of};

use byteorder::{ByteOrder, LittleEndian};

use crate::interner::StrId;

#[derive(thiserror::Error, Debug)]
pub enum OpCodeError {
    #[error("Received incorrect opcode {opcode} that doesn't match any known opcode")]
    _InvalidOpCode { opcode: u8 },
}

#[derive(Default)]
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

    pub fn read_next(self: &ByteCode, ip: usize) -> Option<Result<(OpCode, usize), OpCodeError>> {
        if self.code.len() <= ip {
            return None;
        }
        let discriminant = self.get_byte(ip);
        let mut result = OpCode::from_repr(discriminant);
        let ip = ip + 1;
        let size = match &mut result {
            Some(OpCode::CONSTANT(idx)) => {
                *idx = self.read_u16(ip);
                3
            }
            Some(OpCode::DECLAREGLOBAL(idx)) => {
                *idx = self.read_u16(ip).into();
                3
            }
            Some(OpCode::SETGLOBAL(idx)) | Some(OpCode::GETGLOBAL(idx)) => {
                *idx = self.read_u16(ip).into();
                3
            }
            Some(OpCode::GETLOCAL(idx)) | Some(OpCode::SETLOCAL(idx)) => {
                *idx = self.read_u16(ip);
                3
            }
            Some(OpCode::JUMPIFFALSE(offset))
            | Some(OpCode::JUMP(offset))
            | Some(OpCode::LOOP(offset)) => {
                *offset = self.read_i16(ip);
                3
            }
            Some(OpCode::CALL(arg_count)) => {
                *arg_count = self.code[ip];
                2
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
            OpCode::CONSTANT(idx) => write_one_u16(&mut self.code, idx),
            OpCode::DECLAREGLOBAL(idx) => write_one_u16(&mut self.code, idx.as_u16()),
            OpCode::GETGLOBAL(idx) | OpCode::SETGLOBAL(idx) => write_one_u16(&mut self.code, idx.as_u16()),
            OpCode::SETLOCAL(idx) | OpCode::GETLOCAL(idx) => write_one_u16(&mut self.code, idx),
            OpCode::JUMPIFFALSE(offset) | OpCode::JUMP(offset) | OpCode::LOOP(offset) => {
                write_one_i16(&mut self.code, offset)
            }
            OpCode::CALL(arg_count) => {
                self.code.push(arg_count as u8);
                1
            }
            _ => 0,
        };
        self.lines.extend(repeat(line).take(bytes_count + 1));
    }

    pub fn decompile<'a>(
        &'a self,
        ip: usize,
    ) -> impl Iterator<Item = Result<OpCode, OpCodeError>> + 'a + std::fmt::Display {
        ByteCodeSlice {
            byte_code: self,
            start: ip,
        }
    }
}

struct ByteCodeSlice<'a> {
    byte_code: &'a ByteCode,
    start: usize,
}

impl<'a> std::fmt::Display for ByteCodeSlice<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut ip = self.start;
        while let Some(Ok((op, size))) = self.byte_code.read_next(ip) {
            writeln!(f, "@{} {:?}", ip, op)?;
            ip = ip + size;
        }
        Ok(())
    }
}

impl<'a> Iterator for ByteCodeSlice<'a> {
    type Item = Result<OpCode, OpCodeError>;

    fn next(&mut self) -> Option<Self::Item> {
        match self.byte_code.read_next(self.start) {
            Some(Ok((op, size))) => {
                self.start += size;
                Some(Ok(op))
            }
            Some(Err(err)) => Some(Err(err)),
            None => None,
        }
    }
}

#[derive(Debug, strum::FromRepr, PartialEq, strum::AsRefStr, strum::EnumDiscriminants)]
#[strum_discriminants(name(OpCodeTypes), derive(strum::Display), derive(strum::FromRepr))]
#[repr(u8)]
pub enum OpCode {
    CONSTANT(u16) = 1,
    GETGLOBAL(StrId),
    SETGLOBAL(StrId),
    SETLOCAL(u16),
    GETLOCAL(u16),
    DECLAREGLOBAL(StrId),
    JUMP(i16),
    JUMPIFFALSE(i16),
    LOOP(i16),
    CALL(u8),
    RET,
    POP,
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
    EQUAL,
}

#[cfg(test)]
mod tests {
    use crate::byte_code::OpCode;

    use super::ByteCode;

    #[test]
    fn test_write_read_op() {
        let mut bytes = ByteCode::default();
        bytes.write_op(OpCode::ADD, 1);
        bytes.write_op(OpCode::CONSTANT(20), 2);
        bytes.write_op(OpCode::GETGLOBAL(30.into()), 3);
        let (x1, s1) = bytes.read_next(0).unwrap().unwrap();
        let (x2, s2) = bytes.read_next(s1).unwrap().unwrap();
        let (x3, _) = bytes.read_next(s1 + s2).unwrap().unwrap();
        assert_eq!(
            vec![x1, x2, x3],
            vec![
                OpCode::ADD,
                OpCode::CONSTANT(20),
                OpCode::GETGLOBAL(30.into())
            ]
        );
        assert_eq!(bytes.lines, vec![1, 2, 2, 2, 3, 3, 3])
    }
}
