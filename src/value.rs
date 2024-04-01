use crate::interner::StrId;

#[derive(Clone, strum::EnumDiscriminants, strum::EnumTryAs)]
#[strum_discriminants(name(ValueTypes))]
#[strum_discriminants(derive(strum::Display))]
#[repr(u8)]
pub enum Value {
    Number(f64),
    Bool(bool),
    Nil,
    String(StrId),
    Closure(ClosureValue),
}

impl Value {
    pub fn closure(closure_id: usize) -> Value {
        return Value::Closure(ClosureValue { closure_id });
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct ClosureValue {
    pub closure_id: usize,
}

#[derive(Debug)]
pub enum UpValueImpl {
    Open(usize),
    Closed(usize),
}

impl From<f64> for Value {
    fn from(value: f64) -> Self {
        Value::Number(value)
    }
}

impl From<bool> for Value {
    fn from(value: bool) -> Self {
        Value::Bool(value)
    }
}
