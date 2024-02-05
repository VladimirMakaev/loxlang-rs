use std::fmt::Display;

#[derive(Clone)]
pub enum Value {
    Number(f64),
    Bool(bool),
    Nil,
    String(Box<String>),
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
            Value::String(x) => write!(f, "{}", x),
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
