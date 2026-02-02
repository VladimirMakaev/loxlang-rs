use std::hash::{BuildHasher, BuildHasherDefault};
use std::collections::hash_map::DefaultHasher;
use hashbrown::HashMap;
use crate::gc::GcRef;

pub type DefaultStringTable = StringTable<BuildHasherDefault<DefaultHasher>>;

/// String intern table - maps hash to GcRef for deduplication
/// Strings themselves are stored in Heap as ObjString
pub struct StringTable<H: BuildHasher> {
    table: HashMap<u64, GcRef>,
    hasher: H,
}

impl<H: BuildHasher> StringTable<H> {
    pub fn new(hasher: H) -> Self {
        StringTable {
            table: HashMap::new(),
            hasher,
        }
    }

    pub fn hash_string(&self, s: &str) -> u64 {
        self.hasher.hash_one(s)
    }

    /// Look up existing interned string by hash
    pub fn get(&self, hash: u64) -> Option<GcRef> {
        self.table.get(&hash).copied()
    }

    /// Insert new string reference
    pub fn insert(&mut self, hash: u64, gc_ref: GcRef) {
        self.table.insert(hash, gc_ref);
    }

    /// Remove entries for unmarked strings (called during GC sweep)
    pub fn remove_unmarked<F>(&mut self, is_marked: F)
    where
        F: Fn(GcRef) -> bool,
    {
        self.table.retain(|_, r| is_marked(*r));
    }

    /// Get the underlying hasher for external use
    #[allow(dead_code)]
    pub fn hasher(&self) -> &H {
        &self.hasher
    }
}

// Keep StrId for backward compatibility during migration
// This can be removed after full migration
#[derive(PartialEq, Hash, Eq, Clone, Copy, Default, Debug)]
pub struct StrId {
    pub(crate) idx: u16,
}

impl StrId {
    pub fn as_u16(&self) -> u16 {
        self.idx
    }
}

impl From<usize> for StrId {
    fn from(value: usize) -> Self {
        StrId { idx: value as u16 }
    }
}

impl From<u16> for StrId {
    fn from(value: u16) -> Self {
        StrId { idx: value }
    }
}

impl From<i32> for StrId {
    fn from(value: i32) -> Self {
        (value as u16).into()
    }
}

// Keep the old Interner for backward compatibility during migration
pub type DefaultInterner = Interner<BuildHasherDefault<DefaultHasher>>;

pub struct Interner<H: BuildHasher> {
    all_strings: Vec<String>,
    raw_table: hashbrown::HashTable<usize>,
    hasher: H,
}

impl<H: BuildHasher> Interner<H> {
    pub fn new(hasher: H) -> Self {
        Interner {
            raw_table: Default::default(),
            hasher,
            all_strings: Default::default(),
        }
    }

    pub fn intern_str(&mut self, val: &str) -> StrId {
        let h = |x: &_| self.hasher.hash_one(x);
        if let Some(idx) = self.raw_table.find(h(val), |str_idx| {
            val.eq(self.all_strings[*str_idx].as_str())
        }) {
            idx.to_owned().into()
        } else {
            self.insert_unique_value(val.into())
        }
    }

    #[allow(dead_code)]
    pub fn intern_string(&mut self, val: String) -> StrId {
        let h = |x: &_| self.hasher.hash_one(x);
        if let Some(idx) = self.raw_table.find(h(val.as_str()), |str_idx| {
            val.eq(self.all_strings[*str_idx].as_str())
        }) {
            idx.to_owned().into()
        } else {
            self.insert_unique_value(val)
        }
    }

    fn insert_unique_value(&mut self, value: String) -> StrId {
        let h = |x: &_| self.hasher.hash_one(x);
        let idx = self.all_strings.len();
        self.raw_table
            .insert_unique(h(value.as_str()), idx, |x| h(self.all_strings[*x].as_str()));
        self.all_strings.push(value);
        idx.into()
    }

    pub fn get_str(&self, key: StrId) -> &str {
        self.all_strings[key.idx as usize].as_str()
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::hash_map::DefaultHasher, hash::BuildHasherDefault};

    use super::Interner;

    #[test]
    pub fn test1() {
        let mut interner = Interner::new(BuildHasherDefault::<DefaultHasher>::default());

        assert_eq!(
            interner.intern_str("hello"),
            interner.intern_string("hello".to_owned())
        );
        assert_eq!(interner.intern_str("hello"), interner.intern_str("hello"));
        assert_eq!(
            interner.intern_string("hello".to_owned()),
            interner.intern_string("hello".to_owned())
        );
    }

    #[test]
    pub fn test2() {
        let mut interner = Interner::new(BuildHasherDefault::<DefaultHasher>::default());
        let key = interner.intern_string("hello".to_owned());
        assert_eq!(interner.get_str(key), "hello");
    }
}
