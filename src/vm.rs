use std::fmt::Display;

use thiserror::Error;
use tracing::debug;

#[repr(u8)]
pub enum OpCode {
    CONSTANT = 1,
    ADD,
    MULTIPLY,
    SUBTRACT,
    NEGATE,
    PRINT,
    Unsupported,
}

#[derive(Error, Debug)]
pub enum VirtualMachineError {
    #[error("Invalid op code number '{0}'")]
    InvalidOpCode(u8),
    #[error("Unexpected end of byte code sequence detected")]
    UnexpectedEndOfByteCode,
    #[error("Expected operand on the stack but none found.")]
    MissingStackOperand,
}

#[derive(Clone, Copy)]
pub struct Value {
    val: f64,
}

impl Display for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.val)
    }
}

impl From<f64> for Value {
    fn from(value: f64) -> Self {
        Self { val: value }
    }
}

pub struct VirtualMachine {
    chunks: Vec<u8>,
    ip: usize,
    stack: Vec<Value>,
    constants: Vec<Value>,
}

impl TryInto<OpCode> for u8 {
    type Error = VirtualMachineError;

    fn try_into(self) -> Result<OpCode, Self::Error> {
        if self > OpCode::Unsupported as u8 {
            Err(VirtualMachineError::InvalidOpCode(self))
        } else {
            Ok(unsafe { std::mem::transmute(self) })
        }
    }
}

impl VirtualMachine {
    pub fn new(byte_code: Vec<u8>) -> Self {
        Self {
            chunks: byte_code,
            constants: Default::default(),
            ip: 0,
            stack: Default::default(),
        }
    }

    pub fn add_constant(&mut self, v: Value) {
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

    pub fn run(&mut self) -> Result<(), VirtualMachineError> {
        while let Ok(op_code) = self.read_byte() {
            match TryInto::<OpCode>::try_into(op_code)? {
                OpCode::CONSTANT => {
                    let c = self.read_byte()?;
                    self.push(self.constants[c as usize]);
                }
                OpCode::ADD => {
                    let left = self.pop()?;
                    let right = self.pop()?;
                    debug!("ADD {} {}", left, right);
                    self.push((left.val + right.val).into());
                }
                OpCode::MULTIPLY => {
                    let left = self.pop()?;
                    let right = self.pop()?;
                    debug!("MULTIPLY {} {}", left, right);
                    self.push((left.val * right.val).into());
                }
                OpCode::SUBTRACT => {
                    let left = self.pop()?;
                    let right = self.pop()?;
                    debug!("SUBTRACT {} {}", left, right);
                    self.push((left.val - right.val).into());
                }
                OpCode::NEGATE => {
                    let left = self.pop()?;
                    debug!("NEGATE {}", left);
                    self.push((-left.val).into());
                }
                OpCode::PRINT => {
                    let param = self.pop()?;
                    println!("{}", param);
                }
                OpCode::Unsupported => return Err(VirtualMachineError::InvalidOpCode(op_code)),
            }
        }

        Ok(())
    }

    pub fn read_byte(&mut self) -> Result<u8, VirtualMachineError> {
        let result = self.chunks.get(self.ip).map(ToOwned::to_owned);
        self.ip += 1;
        result.ok_or_else(|| VirtualMachineError::UnexpectedEndOfByteCode)
    }
}
