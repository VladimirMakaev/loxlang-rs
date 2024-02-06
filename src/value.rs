use std::{borrow::Cow, fmt::Display, rc::Rc};

#[derive(Clone, strum::EnumDiscriminants)]
#[strum_discriminants(name(ValueTypes))]
#[strum_discriminants(derive(strum::Display))]
pub enum Value {
    Number(f64),
    Bool(bool),
    Nil,
    Object(ObjectRef),
}

#[derive(Clone)]
pub struct ObjectRef {
    pub idx: usize,
    pub ty: Rc<dyn ObjectType>,
}

pub trait ObjectType {
    fn type_name(&self) -> &'static str;
    fn base_object(&self) -> Option<ObjectRef>;
    //fn hash(&self) -> usize;
    //fn eq(&self, other: &Self) -> bool;
}

#[derive(Clone)]
pub struct StringObject {
    pub value: Rc<String>,
}

impl StringObject {
    pub fn new_ref(object_id: usize, val: Rc<String>) -> ObjectRef {
        ObjectRef {
            idx: object_id,
            ty: Rc::new(Self { value: val }),
        }
    }
}

impl ObjectType for StringObject {
    fn type_name(&self) -> &'static str {
        "string"
    }

    fn base_object(&self) -> Option<ObjectRef> {
        None
    }
}

impl Value {
    pub fn as_number(&self) -> Option<f64> {
        if let Value::Number(x) = self {
            Some(*x)
        } else {
            None
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        if let Value::Bool(x) = self {
            Some(*x)
        } else {
            None
        }
    }
}

impl Display for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Value::Number(x) => write!(f, "{}", x),
            Value::Bool(x) => write!(f, "{}", x),
            Value::Nil => f.write_str("nil"),
            _ => todo!(), //Value::String(x) => write!(f, "{}", x),
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
