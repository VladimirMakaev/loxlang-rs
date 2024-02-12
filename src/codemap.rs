use std::collections::BTreeMap;

pub struct Codemap {
    lines_start_at_1: bool,
    position_starts_at_1: bool,
    line_endings: BTreeMap<usize, usize>,
}

impl Codemap {
    pub fn new(code: &str) -> Self {
        Self {
            lines_start_at_1: true,
            position_starts_at_1: false,
            line_endings: Self::index_lines(code),
        }
    }

    pub fn line_at(&self, position: usize) -> Option<usize> {
        let x = self
            .line_endings
            .range(..position + if self.position_starts_at_1 { 2 } else { 1 });
        if let Some((_, line)) = x.max() {
            Some(*line + if self.lines_start_at_1 { 1 } else { 0 })
        } else {
            None
        }
    }

    fn index_lines(code: &str) -> BTreeMap<usize, usize> {
        let mut line_starts = BTreeMap::new();
        let mut line = 0;
        let mut char_index = 0;
        let mut line_stared = true;

        for c in code.chars() {
            if line_stared {
                line_starts.insert(char_index, line);
                line_stared = false;
            }
            if c == '\n' {
                line_stared = true;
                line += 1;
            }
            char_index += 1;
        }
        line_starts
    }
}

#[cfg(test)]
mod tests {
    use super::Codemap;
    use pretty_assertions::assert_eq;

    #[test]
    pub fn test_code_map() {
        let code = r###"zero
one
two
three
four"###;
        let codemap = Codemap::new(code);

        assert_eq!(
            (0..code.len())
                .map(|i| codemap.line_at(i))
                .collect::<Vec<_>>(),
            vec![1, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 4, 4, 5, 5, 5, 5]
                .into_iter()
                .map(Some)
                .collect::<Vec<_>>()
        )
    }
}
