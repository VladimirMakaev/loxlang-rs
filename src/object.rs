//! Unified object types for GC-managed heap objects.
//!
//! This module defines the object types stored on the GC heap,
//! following the Crafting Interpreters design with mark bits for tracing GC.

use crate::byte_code::ByteCode;
use crate::gc::GcRef;
// Note: Value import will be needed when Value::Object variant is added in Plan 02
#[allow(unused_imports)]
use crate::value::Value;

/// A garbage-collected object with mark bit and kind.
pub struct Obj {
    pub is_marked: bool,
    pub kind: ObjKind,
}

/// The different kinds of objects stored on the heap.
pub enum ObjKind {
    String(ObjString),
    Closure(ObjClosure),
    Function(ObjFunction),
    Upvalue(ObjUpvalue),
}

/// A string object with its value and pre-computed hash.
pub struct ObjString {
    pub value: String,
    pub hash: u64,
}

/// A closure object with a reference to its function and captured upvalues.
pub struct ObjClosure {
    pub function: GcRef,
    pub upvalues: Vec<GcRef>,
}

/// A function object containing compiled bytecode.
pub struct ObjFunction {
    pub name: GcRef, // Points to ObjString
    pub arity: usize,
    pub code: ByteCode,
    pub upvalue_count: usize,
}

/// An upvalue object for closure variable capture.
pub struct ObjUpvalue {
    pub location: UpvalueLocation,
}

/// Represents whether an upvalue is still on the stack or has been closed.
pub enum UpvalueLocation {
    Open(usize),   // Stack slot index
    Closed(Value), // Captured value
}

impl Obj {
    /// Returns the size in bytes of this object (for GC accounting).
    pub fn size(&self) -> usize {
        std::mem::size_of::<Obj>()
            + match &self.kind {
                ObjKind::String(s) => s.value.len(),
                ObjKind::Closure(c) => c.upvalues.len() * std::mem::size_of::<GcRef>(),
                ObjKind::Function(f) => f.code.size(),
                ObjKind::Upvalue(_) => 0,
            }
    }

    /// Creates a new string object.
    pub fn string(value: String, hash: u64) -> Self {
        Obj {
            is_marked: false,
            kind: ObjKind::String(ObjString { value, hash }),
        }
    }

    /// Creates a new closure object.
    pub fn closure(function: GcRef, upvalues: Vec<GcRef>) -> Self {
        Obj {
            is_marked: false,
            kind: ObjKind::Closure(ObjClosure { function, upvalues }),
        }
    }

    /// Creates a new function object.
    pub fn function(name: GcRef, arity: usize, code: ByteCode, upvalue_count: usize) -> Self {
        Obj {
            is_marked: false,
            kind: ObjKind::Function(ObjFunction {
                name,
                arity,
                code,
                upvalue_count,
            }),
        }
    }

    /// Creates a new upvalue object.
    pub fn upvalue(location: UpvalueLocation) -> Self {
        Obj {
            is_marked: false,
            kind: ObjKind::Upvalue(ObjUpvalue { location }),
        }
    }
}

impl ObjKind {
    /// Returns all GC references contained in this object (for mark phase traversal).
    pub fn get_references(&self) -> Vec<GcRef> {
        match self {
            ObjKind::String(_) => vec![],
            ObjKind::Closure(c) => {
                let mut refs = vec![c.function];
                refs.extend(&c.upvalues);
                refs
            }
            ObjKind::Function(f) => vec![f.name],
            ObjKind::Upvalue(u) => {
                // Extract GcRef from closed upvalues containing object references
                if let UpvalueLocation::Closed(Value::Object(r)) = &u.location {
                    vec![*r]
                } else {
                    vec![]
                }
            }
        }
    }
}
