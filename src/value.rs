use crate::interner::StrId;

#[derive(Clone, strum::EnumDiscriminants)]
#[strum_discriminants(name(ValueTypes))]
#[strum_discriminants(derive(strum::Display))]
#[repr(u8)]
pub enum Value {
    Number(f64),
    Bool(bool),
    Nil,
    String(StrId),
    Object(ObjectValue),
}

impl Value {
    pub fn fun(fun_idx: usize) -> Value {
        return Value::Object(ObjectValue {
            object_id: fun_idx,
            ty: ObjectType::Function,
        });
    }
}

#[derive(Clone, Copy)]
pub enum ObjectType {
    Function,
    _Class,
}

#[derive(Clone, Copy)]
pub struct ObjectValue {
    pub ty: ObjectType,
    pub object_id: usize,
}

impl Value {
    pub fn as_number(&self) -> Option<f64> {
        if let Value::Number(x) = self {
            Some(*x)
        } else {
            None
        }
    }

    pub fn as_object(&self) -> Option<ObjectValue> {
        if let Value::Object(x) = self {
            Some(*x)
        } else {
            None
        }
    }
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
