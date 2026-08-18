use std::collections::HashMap;

const CHUNK_SIZE: usize = 64 * 1024; 

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct StringRef {
    pub chunk_idx: u16,
    pub offset: u32,
    pub len: u16,
}

impl StringRef {
    pub const EMPTY: StringRef = StringRef {
        chunk_idx: 0,
        offset: 0,
        len: 0,
    };

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

pub struct StringArena {
    chunks: Vec<Vec<u8>>,
    map: HashMap<String, StringRef>,
    total_bytes_stored: usize,
}

impl Default for StringArena {
    fn default() -> Self {
        Self::new()
    }
}

impl StringArena {
    pub fn new() -> Self {
        Self {
            chunks: Vec::new(),
            map: HashMap::new(),
            total_bytes_stored: 0,
        }
    }

    pub fn with_capacity(expected_bytes: usize) -> Self {
        let chunk_count = (expected_bytes + CHUNK_SIZE - 1) / CHUNK_SIZE;
        let mut chunks = Vec::with_capacity(chunk_count.max(1));
        chunks.push(Vec::with_capacity(CHUNK_SIZE));
        Self {
            chunks,
            map: HashMap::new(),
            total_bytes_stored: 0,
        }
    }

    pub fn total_bytes_stored(&self) -> usize {
        self.total_bytes_stored
    }

    pub fn allocated_bytes(&self) -> usize {
        self.chunks.iter().map(|c| c.capacity()).sum()
    }

    pub fn chunk_count(&self) -> usize {
        self.chunks.len()
    }

    pub fn intern(&mut self, text: &str) -> StringRef {
        if text.is_empty() {
            return StringRef::EMPTY;
        }

        if let Some(&sref) = self.map.get(text) {
            return sref;
        }

        let sref = self.push(text);
        self.map.insert(text.to_string(), sref);
        sref
    }

    pub fn push(&mut self, text: &str) -> StringRef {
        if text.is_empty() {
            return StringRef::EMPTY;
        }

        let mut text = text;
        if text.len() > u16::MAX as usize {
            let mut end = u16::MAX as usize;
            while end > 0 && !text.is_char_boundary(end) {
                end -= 1;
            }
            text = &text[..end];
        }
        let bytes = text.as_bytes();
        let len = bytes.len();

        if self.chunks.is_empty() {
            self.chunks.push(Vec::with_capacity(CHUNK_SIZE.max(len)));
        }

        let last_idx = self.chunks.len() - 1;
        let can_fit = {
            let last_chunk = &self.chunks[last_idx];
            last_chunk.len() + len <= last_chunk.capacity()
        };

        let chunk_idx = if can_fit {
            last_idx
        } else {
            self.chunks.push(Vec::with_capacity(CHUNK_SIZE.max(len)));
            self.chunks.len() - 1
        };

        let chunk = &mut self.chunks[chunk_idx];
        let offset = chunk.len() as u32;
        chunk.extend_from_slice(bytes);
        self.total_bytes_stored += len;

        StringRef {
            chunk_idx: chunk_idx as u16,
            offset,
            len: len as u16,
        }
    }

    pub fn get(&self, sref: StringRef) -> &str {
        if sref.is_empty() {
            return "";
        }
        let chunk = match self.chunks.get(sref.chunk_idx as usize) {
            Some(c) => c,
            None => return "",
        };
        let start = sref.offset as usize;
        let end = start + sref.len as usize;
        if end > chunk.len() {
            return "";
        }
        std::str::from_utf8(&chunk[start..end]).unwrap_or("")
    }

    pub fn clear(&mut self) {
        for chunk in &mut self.chunks {
            chunk.clear();
        }
        self.map.clear();
        self.total_bytes_stored = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_string_arena_push_and_get() {
        let mut arena = StringArena::new();
        let ref1 = arena.push("PCSF00001");
        let ref2 = arena.push("Killzone: Mercenary");
        let ref3 = arena.push("");

        assert_eq!(arena.get(ref1), "PCSF00001");
        assert_eq!(arena.get(ref2), "Killzone: Mercenary");
        assert_eq!(arena.get(ref3), "");
        assert_eq!(arena.get(StringRef::EMPTY), "");
    }

    #[test]
    fn test_string_arena_intern_dedup() {
        let mut arena = StringArena::new();
        let ref1 = arena.intern("Sony Interactive Entertainment");
        let ref2 = arena.intern("Sony Interactive Entertainment");
        let ref3 = arena.intern("Ubisoft");

        assert_eq!(ref1, ref2);
        assert_ne!(ref1, ref3);
        assert_eq!(arena.get(ref1), "Sony Interactive Entertainment");
        assert_eq!(arena.get(ref3), "Ubisoft");
    }

    #[test]
    fn test_string_arena_overflow_chunks() {
        let mut arena = StringArena::new();
        let large_string = "X".repeat(40 * 1024);
        let ref1 = arena.push(&large_string);
        let ref2 = arena.push(&large_string);

        assert_eq!(arena.chunk_count(), 2);
        assert_eq!(arena.get(ref1), &large_string);
        assert_eq!(arena.get(ref2), &large_string);
    }
}
