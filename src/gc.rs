//! Garbage collection infrastructure for the Lox VM.
//!
//! This module provides the Heap arena with index-based GcRef references,
//! enabling mark-and-sweep garbage collection.

use crate::object::Obj;

/// A reference to a GC-managed object in the heap.
///
/// Uses an index into the arena rather than a raw pointer for safety.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub struct GcRef(pub(crate) u32);

/// The garbage-collected heap arena.
///
/// Objects are stored in a Vec with index-based references (GcRef).
/// Free slots are tracked for reuse, and a gray stack supports tricolor marking.
pub struct Heap {
    objects: Vec<Option<Obj>>,
    free_list: Vec<u32>,
    gray_stack: Vec<u32>,
    pub bytes_allocated: usize,
    pub next_gc: usize,
}

impl Heap {
    /// Creates a new empty heap with default GC threshold.
    pub fn new() -> Self {
        Self {
            objects: Vec::new(),
            free_list: Vec::new(),
            gray_stack: Vec::new(),
            bytes_allocated: 0,
            next_gc: 1024 * 1024, // 1MB default threshold
        }
    }

    /// Allocates an object on the heap, returning a reference to it.
    ///
    /// Reuses slots from the free list when available.
    pub fn alloc(&mut self, obj: Obj) -> GcRef {
        let size = obj.size();
        self.bytes_allocated += size;

        let index = if let Some(free_idx) = self.free_list.pop() {
            self.objects[free_idx as usize] = Some(obj);
            free_idx
        } else {
            let idx = self.objects.len() as u32;
            self.objects.push(Some(obj));
            idx
        };

        GcRef(index)
    }

    /// Gets an immutable reference to the object at the given reference.
    ///
    /// # Panics
    ///
    /// Panics if the reference is dangling (slot is None).
    pub fn get(&self, r: GcRef) -> &Obj {
        self.objects[r.0 as usize]
            .as_ref()
            .expect("dangling GcRef")
    }

    /// Gets a mutable reference to the object at the given reference.
    ///
    /// # Panics
    ///
    /// Panics if the reference is dangling (slot is None).
    pub fn get_mut(&mut self, r: GcRef) -> &mut Obj {
        self.objects[r.0 as usize]
            .as_mut()
            .expect("dangling GcRef")
    }

    /// Marks an object as reachable and adds it to the gray stack for processing.
    pub fn mark_object(&mut self, r: GcRef) {
        if let Some(obj) = self.objects.get_mut(r.0 as usize).and_then(|o| o.as_mut()) {
            if !obj.is_marked {
                obj.is_marked = true;
                self.gray_stack.push(r.0);
            }
        }
    }

    /// Checks if a reference points to a valid (non-freed) object.
    pub fn is_valid(&self, r: GcRef) -> bool {
        self.objects
            .get(r.0 as usize)
            .map(|o| o.is_some())
            .unwrap_or(false)
    }
}

impl Default for Heap {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::object::{Obj, ObjKind};

    #[test]
    fn test_heap_alloc_and_get() {
        let mut heap = Heap::new();
        let obj = Obj::string("hello".to_string(), 12345);
        let r = heap.alloc(obj);

        assert!(heap.is_valid(r));
        assert!(!heap.get(r).is_marked);
        if let ObjKind::String(s) = &heap.get(r).kind {
            assert_eq!(s.value, "hello");
        } else {
            panic!("Expected string object");
        }
    }
}
