//! Unified object types for GC-managed heap objects.
//!
//! This module defines the object types stored on the GC heap,
//! following the Crafting Interpreters design with mark bits for tracing GC.

use hashbrown::HashMap;

use crate::byte_code::ByteCode;
use crate::gc::GcRef;
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
    Class(ObjClass),
    Instance(ObjInstance),
    BoundMethod(ObjBoundMethod),
    Native(ObjNative),
}

/// A native function object.
pub struct ObjNative {
    pub name: &'static str, // Static string, no heap allocation
    pub arity: usize,
}

/// A string object with its value and pre-computed hash.
pub struct ObjString {
    pub value: String,
    #[allow(dead_code)]
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

/// A class object containing the class name and methods.
pub struct ObjClass {
    pub name: GcRef, // Points to ObjString
    pub methods: HashMap<GcRef, GcRef>, // Method name GcRef -> closure GcRef
}

/// A bound method object binding a closure to an instance receiver.
pub struct ObjBoundMethod {
    pub receiver: Value,  // The instance
    pub method: GcRef,    // Points to ObjClosure
}

/// An instance object with a reference to its class and field storage.
pub struct ObjInstance {
    pub klass: GcRef,                   // Points to ObjClass
    pub fields: HashMap<GcRef, Value>,  // Field name -> value
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
                ObjKind::Class(c) => {
                    // Methods HashMap size (method closures are separate objects)
                    c.methods.len() * (std::mem::size_of::<GcRef>() * 2)
                }
                ObjKind::Instance(i) => {
                    i.fields.len() * (std::mem::size_of::<GcRef>() + std::mem::size_of::<Value>())
                }
                ObjKind::BoundMethod(_) => 0, // Method is separate object
                ObjKind::Native(_) => 0,     // Native uses static str, no heap allocation
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

    /// Creates a new class object.
    pub fn class(name: GcRef) -> Self {
        Obj {
            is_marked: false,
            kind: ObjKind::Class(ObjClass {
                name,
                methods: HashMap::new(),
            }),
        }
    }

    /// Creates a new instance object with empty fields.
    pub fn instance(klass: GcRef) -> Self {
        Obj {
            is_marked: false,
            kind: ObjKind::Instance(ObjInstance {
                klass,
                fields: HashMap::new(),
            }),
        }
    }

    /// Creates a new bound method object.
    pub fn bound_method(receiver: Value, method: GcRef) -> Self {
        Obj {
            is_marked: false,
            kind: ObjKind::BoundMethod(ObjBoundMethod { receiver, method }),
        }
    }

    /// Creates a new native function object.
    pub fn native(name: &'static str, arity: usize) -> Self {
        Obj {
            is_marked: false,
            kind: ObjKind::Native(ObjNative { name, arity }),
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
            ObjKind::Class(c) => {
                // Trace name ref + all method closure refs
                let mut refs = vec![c.name];
                refs.extend(c.methods.values().copied());
                refs
            }
            ObjKind::Instance(i) => {
                // Trace klass ref + all field name refs + all field value object refs
                let mut refs = vec![i.klass];
                for (name_ref, value) in &i.fields {
                    refs.push(*name_ref);
                    if let Value::Object(r) = value {
                        refs.push(*r);
                    }
                }
                refs
            }
            ObjKind::BoundMethod(bm) => {
                // Trace method closure ref + receiver if it's an object
                let mut refs = vec![bm.method];
                if let Value::Object(r) = &bm.receiver {
                    refs.push(*r);
                }
                refs
            }
            ObjKind::Native(_) => vec![], // Native functions have no GC references
        }
    }
}
