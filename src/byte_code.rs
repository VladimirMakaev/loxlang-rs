use std::{iter::repeat, mem::size_of};

use byteorder::{ByteOrder, LittleEndian};

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

impl ByteCode {
    pub fn get_byte(&self, ip: usize) -> u8 {
        self.code[ip]
    }

    pub fn read_u16(&self, ip: usize) -> u16 {
        LittleEndian::read_u16(&self.code[ip..])
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
            Some(OpCode::DEFINEGLOBAL(idx)) => {
                *idx = self.read_u16(ip);
                3
            }
            Some(OpCode::SETGLOBAL(idx)) => {
                *idx = self.read_u16(ip);
                3
            }
            Some(OpCode::GETGLOBAL(idx)) => {
                *idx = self.read_u16(ip);
                3
            }
            _ => 1,
        };
        result.map(|x| Ok((x, size)))
    }

    pub fn write_op(&mut self, op: OpCode, line: usize) {
        fn write_one_u16(code: &mut Vec<u8>, param: u16) -> usize {
            let bytes = param.to_le_bytes();
            code.extend(bytes);
            size_of::<u16>()
        }

        self.code
            .push(unsafe { std::mem::transmute(std::mem::discriminant(&op)) });

        let bytes_count = match op {
            OpCode::CONSTANT(idx) => write_one_u16(&mut self.code, idx),
            OpCode::DEFINEGLOBAL(idx) => write_one_u16(&mut self.code, idx),
            OpCode::GETGLOBAL(idx) => write_one_u16(&mut self.code, idx),
            OpCode::SETGLOBAL(idx) => write_one_u16(&mut self.code, idx),
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
            writeln!(f, "{:?}", op)?;
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

#[derive(
    Debug,
    strum::FromRepr,
    strum::EnumProperty,
    PartialEq,
    strum::AsRefStr,
    strum::EnumDiscriminants,
)]
#[strum_discriminants(name(OpCodeTypes), derive(strum::Display))]
#[repr(u8)]
pub enum OpCode {
    CONSTANT(u16),
    GETGLOBAL(u16),
    SETGLOBAL(u16),
    DEFINEGLOBAL(u16),
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
        bytes.write_op(OpCode::GETGLOBAL(30), 3);
        let (x1, s1) = bytes.read_next(0).unwrap().unwrap();
        let (x2, s2) = bytes.read_next(s1).unwrap().unwrap();
        let (x3, _) = bytes.read_next(s1 + s2).unwrap().unwrap();
        assert_eq!(
            vec![x1, x2, x3],
            vec![OpCode::ADD, OpCode::CONSTANT(20), OpCode::GETGLOBAL(30)]
        );
        assert_eq!(bytes.lines, vec![1, 2, 2, 2, 3, 3, 3])
    }
}
