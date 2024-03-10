use std::{
    collections::hash_map::DefaultHasher,
    hash::{BuildHasher, BuildHasherDefault},
};

#[derive(PartialEq, Debug, Hash, Eq, Clone, Copy, Default)]
pub struct StrId {
    idx: u16,
}

impl StrId {
    pub fn as_u16(&self) -> u16 {
        self.idx
    }
}

pub type DefaultInterner = Interner<BuildHasherDefault<DefaultHasher>>;

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

pub struct Interner<H: BuildHasher> {
    all_strings: Vec<String>,
    raw_table: hashbrown::HashTable<usize>,
    hasher: H,
}

impl<H: BuildHasher> Interner<H> {
    pub fn new(hasher: H) -> Self {
        Interner {
            raw_table: Default::default(),
            hasher: hasher,
            all_strings: Default::default(),
        }
    }

    pub fn intern_str<'a>(&'a mut self, val: &str) -> StrId {
        let h = |x: &_| self.hasher.hash_one(x);
        if let Some(idx) = self.raw_table.find(h(val), |str_idx| {
            val.eq(self.all_strings[*str_idx].as_str())
        }) {
            idx.to_owned().into()
        } else {
            self.insert_unique_value(val.into())
        }
    }

    pub fn intern_string<'a>(&'a mut self, val: String) -> StrId {
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

    pub fn get_str<'a>(&'a self, key: StrId) -> &'a str {
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
