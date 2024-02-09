use std::{
    collections::hash_map::DefaultHasher,
    hash::{BuildHasher, BuildHasherDefault},
};

#[derive(PartialEq, Debug)]
pub struct Key {
    pub idx: usize,
}

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
            hasher: hasher,
            all_strings: Default::default(),
        }
    }

    pub fn intern_str<'a>(&'a mut self, val: &str) -> Key {
        let h = |x: &_| self.hasher.hash_one(x);
        if let Some(idx) = self.raw_table.find(h(val), |str_idx| {
            val.eq(self.all_strings[*str_idx].as_str())
        }) {
            Key { idx: *idx }
        } else {
            self.insert_unique_value(val.into())
        }
    }

    pub fn intern_string<'a>(&'a mut self, val: String) -> Key {
        let h = |x: &_| self.hasher.hash_one(x);
        if let Some(idx) = self.raw_table.find(h(val.as_str()), |str_idx| {
            val.eq(self.all_strings[*str_idx].as_str())
        }) {
            Key { idx: *idx }
        } else {
            self.insert_unique_value(val)
        }
    }

    fn insert_unique_value(&mut self, value: String) -> Key {
        let h = |x: &_| self.hasher.hash_one(x);
        let idx = self.all_strings.len();
        self.raw_table
            .insert_unique(h(value.as_str()), idx, |x| h(self.all_strings[*x].as_str()));
        self.all_strings.push(value);
        Key { idx }
    }

    pub fn get_str<'a>(&'a self, key: &Key) -> &'a str {
        self.all_strings[key.idx].as_str()
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
        assert_eq!(interner.get_str(&key), "hello");
    }
}
