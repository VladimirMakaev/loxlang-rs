use std::io::Write;
use std::path::Path;

use crate::byte_code::OpCodeTypes;

pub struct PerfCounters {
    total_instructions: u64,
    opcode_counts: Vec<u64>,
    function_calls: u64,
    max_stack_depth: usize,
    max_call_depth: usize,
    gc_collections: u64,
    gc_bytes_freed: u64,
    heap_allocations: u64,
}

impl PerfCounters {
    pub fn new() -> Self {
        Self {
            total_instructions: 0,
            opcode_counts: vec![0u64; 256],
            function_calls: 0,
            max_stack_depth: 0,
            max_call_depth: 0,
            gc_collections: 0,
            gc_bytes_freed: 0,
            heap_allocations: 0,
        }
    }

    #[inline]
    pub fn record_instruction(&mut self, discriminant: u8) {
        self.total_instructions += 1;
        self.opcode_counts[discriminant as usize] += 1;
    }

    #[inline]
    pub fn record_call(&mut self, call_depth: usize) {
        self.function_calls += 1;
        if call_depth > self.max_call_depth {
            self.max_call_depth = call_depth;
        }
    }

    #[inline]
    pub fn record_stack_depth(&mut self, depth: usize) {
        if depth > self.max_stack_depth {
            self.max_stack_depth = depth;
        }
    }

    #[inline]
    pub fn record_gc(&mut self, bytes_freed: usize) {
        self.gc_collections += 1;
        self.gc_bytes_freed += bytes_freed as u64;
    }

    #[inline]
    pub fn record_allocation(&mut self) {
        self.heap_allocations += 1;
    }

    fn opcode_name(discriminant: usize) -> Option<String> {
        OpCodeTypes::from_repr(discriminant as u8).map(|t| t.to_string())
    }

    fn sorted_opcode_entries(&self) -> Vec<(String, u64)> {
        let mut entries: Vec<(String, u64)> = self
            .opcode_counts
            .iter()
            .enumerate()
            .filter(|(_, &count)| count > 0)
            .filter_map(|(i, &count)| Self::opcode_name(i).map(|name| (name, count)))
            .collect();
        entries.sort_by(|a, b| b.1.cmp(&a.1));
        entries
    }

    pub fn print_summary(&self, w: &mut impl Write) -> std::io::Result<()> {
        writeln!(w, "=== Performance Counters ===")?;
        writeln!(w, "Total instructions:  {}", self.total_instructions)?;
        writeln!(w, "Function calls:      {}", self.function_calls)?;
        writeln!(w, "Max stack depth:     {}", self.max_stack_depth)?;
        writeln!(w, "Max call depth:      {}", self.max_call_depth)?;
        writeln!(w, "GC collections:      {}", self.gc_collections)?;
        writeln!(w, "GC bytes freed:      {}", self.gc_bytes_freed)?;
        writeln!(w, "Heap allocations:    {}", self.heap_allocations)?;
        writeln!(w)?;
        writeln!(w, "--- Opcode Counts ---")?;
        writeln!(w, "{:<20} {:>12} {:>8}", "Opcode", "Count", "%")?;
        writeln!(w, "{}", "-".repeat(42))?;

        for (name, count) in self.sorted_opcode_entries() {
            let pct = if self.total_instructions > 0 {
                (count as f64 / self.total_instructions as f64) * 100.0
            } else {
                0.0
            };
            writeln!(w, "{:<20} {:>12} {:>7.1}%", name, count, pct)?;
        }

        Ok(())
    }

    pub fn write_json(&self, path: &Path) -> std::io::Result<()> {
        let mut opcode_counts = serde_json::Map::new();
        for (name, count) in self.sorted_opcode_entries() {
            opcode_counts.insert(name, serde_json::Value::Number(count.into()));
        }

        let root = serde_json::json!({
            "total_instructions": self.total_instructions,
            "opcode_counts": opcode_counts,
            "function_calls": self.function_calls,
            "max_stack_depth": self.max_stack_depth,
            "max_call_depth": self.max_call_depth,
            "gc_collections": self.gc_collections,
            "gc_bytes_freed": self.gc_bytes_freed,
            "heap_allocations": self.heap_allocations,
        });

        let file = std::fs::File::create(path)?;
        serde_json::to_writer_pretty(file, &root)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))
    }
}
