use crate::vm::OpCode;

#[derive(Default)]
pub struct ByteCode {
    code: Vec<u8>,
    lines: Vec<usize>,
}

impl ByteCode {
    pub fn emit_const(&mut self, idx: usize, line: usize) {
        self.code
            .push(unsafe { std::mem::transmute(OpCode::CONSTANT) });
        self.code.extend((idx as u16).to_ne_bytes());
        self.lines.extend([line, line, line]);
    }

    pub fn emit(&mut self, op: OpCode, line: usize) {
        self.code.push(unsafe { std::mem::transmute(op) });
        self.lines.push(line);
    }

    pub fn get_byte(&self, ip: usize) -> u8 {
        self.code[ip]
    }

    pub fn size(&self) -> usize {
        self.code.len()
    }

    pub fn get_u16(&self, ip: usize) -> u16 {
        u16::from_ne_bytes([self.get_byte(ip), self.get_byte(ip + 1)])
    }

    pub fn get_op(&self, ip: usize) -> OpCode {
        unsafe { std::mem::transmute(self.get_byte(ip)) }
    }
}
