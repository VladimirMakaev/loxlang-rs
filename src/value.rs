use crate::gc::GcRef;
// StrId import kept for globals HashMap keys (identifier interning still uses StrId)
#[allow(unused_imports)]
use crate::interner::StrId;

#[derive(Clone, strum::EnumDiscriminants, strum::EnumTryAs)]
#[strum_discriminants(name(ValueTypes))]
#[strum_discriminants(derive(strum::Display))]
#[repr(u8)]
pub enum Value {
    Number(f64),
    Bool(bool),
    Nil,
    // String(StrId) - REMOVED: Strings are now heap-allocated as Value::Object(GcRef)
    Closure(ClosureValue),
    Object(GcRef),
}

impl Value {
    pub fn closure(closure_id: usize) -> Value {
        Value::Closure(ClosureValue { closure_id })
    }

    pub fn object(gc_ref: GcRef) -> Value {
        Value::Object(gc_ref)
    }

    pub fn is_object(&self) -> bool {
        matches!(self, Value::Object(_))
    }

    pub fn as_object(&self) -> Option<GcRef> {
        if let Value::Object(r) = self {
            Some(*r)
        } else {
            None
        }
    }

    /// Returns the GcRef if this is an Object variant.
    /// Caller must verify the object kind is ObjKind::String using heap.get(r).kind.
    pub fn as_string_ref(&self) -> Option<GcRef> {
        if let Value::Object(r) = self {
            Some(*r)
        } else {
            None
        }
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
