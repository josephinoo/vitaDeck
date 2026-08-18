use std::sync::{Arc, Mutex};

pub const TIER_SMALL: usize = 32 * 1024;    
pub const TIER_MEDIUM: usize = 256 * 1024;  
pub const TIER_LARGE: usize = 1024 * 1024;  

#[derive(Debug, Clone, Copy, Default)]
pub struct PoolStats {
    pub acquires: u64,
    pub hits: u64,
    pub releases: u64,
    pub pooled_bytes: usize,
}

pub struct BufferPool {
    small: Vec<Vec<u8>>,
    medium: Vec<Vec<u8>>,
    large: Vec<Vec<u8>>,
    max_small_count: usize,
    max_medium_count: usize,
    max_large_count: usize,
    stats: PoolStats,
}

impl Default for BufferPool {
    fn default() -> Self {
        Self::new(8, 4, 2)
    }
}

impl BufferPool {
    pub fn new(max_small: usize, max_medium: usize, max_large: usize) -> Self {
        Self {
            small: Vec::with_capacity(max_small),
            medium: Vec::with_capacity(max_medium),
            large: Vec::with_capacity(max_large),
            max_small_count: max_small,
            max_medium_count: max_medium,
            max_large_count: max_large,
            stats: PoolStats::default(),
        }
    }

    pub fn stats(&self) -> PoolStats {
        let mut s = self.stats;
        s.pooled_bytes = self.total_pooled_bytes();
        s
    }

    pub fn total_pooled_bytes(&self) -> usize {
        let small_b: usize = self.small.iter().map(|b| b.capacity()).sum();
        let med_b: usize = self.medium.iter().map(|b| b.capacity()).sum();
        let lrg_b: usize = self.large.iter().map(|b| b.capacity()).sum();
        small_b + med_b + lrg_b
    }

    pub fn acquire(&mut self, min_capacity: usize) -> Vec<u8> {
        self.stats.acquires += 1;

        if min_capacity <= TIER_SMALL {
            if let Some(mut buf) = self.small.pop() {
                buf.clear();
                self.stats.hits += 1;
                return buf;
            }
            return Vec::with_capacity(TIER_SMALL);
        }

        if min_capacity <= TIER_MEDIUM {
            if let Some(mut buf) = self.medium.pop() {
                buf.clear();
                self.stats.hits += 1;
                return buf;
            }
            return Vec::with_capacity(TIER_MEDIUM);
        }

        if min_capacity <= TIER_LARGE {
            if let Some(mut buf) = self.large.pop() {
                buf.clear();
                self.stats.hits += 1;
                return buf;
            }
            return Vec::with_capacity(TIER_LARGE);
        }

        Vec::with_capacity(min_capacity)
    }

    pub fn release(&mut self, mut buf: Vec<u8>) {
        self.stats.releases += 1;
        let cap = buf.capacity();
        buf.clear();

        if cap >= TIER_LARGE && self.large.len() < self.max_large_count {
            self.large.push(buf);
        } else if cap >= TIER_MEDIUM && cap < TIER_LARGE && self.medium.len() < self.max_medium_count {
            self.medium.push(buf);
        } else if cap >= TIER_SMALL && cap < TIER_MEDIUM && self.small.len() < self.max_small_count {
            self.small.push(buf);
        }
    }

    pub fn clear(&mut self) {
        self.small.clear();
        self.medium.clear();
        self.large.clear();
    }
}

pub struct SharedBufferPool {
    inner: Arc<Mutex<BufferPool>>,
}

impl Default for SharedBufferPool {
    fn default() -> Self {
        Self::new(8, 4, 2)
    }
}

impl SharedBufferPool {
    pub fn new(max_small: usize, max_medium: usize, max_large: usize) -> Self {
        Self {
            inner: Arc::new(Mutex::new(BufferPool::new(max_small, max_medium, max_large))),
        }
    }

    pub fn acquire(&self, min_capacity: usize) -> PooledBuffer {
        let buf = if let Ok(mut pool) = self.inner.lock() {
            pool.acquire(min_capacity)
        } else {
            Vec::with_capacity(min_capacity)
        };
        PooledBuffer {
            pool: Some(self.inner.clone()),
            buf: Some(buf),
        }
    }
}

pub struct PooledBuffer {
    pool: Option<Arc<Mutex<BufferPool>>>,
    buf: Option<Vec<u8>>,
}

impl PooledBuffer {
    pub fn get(&self) -> &[u8] {
        self.buf.as_deref().unwrap_or(&[])
    }

    pub fn get_mut(&mut self) -> &mut Vec<u8> {
        self.buf.as_mut().expect("PooledBuffer is empty")
    }

    pub fn take(mut self) -> Vec<u8> {
        self.buf.take().unwrap_or_default()
    }
}

impl std::ops::Deref for PooledBuffer {
    type Target = Vec<u8>;
    fn deref(&self) -> &Self::Target {
        self.buf.as_ref().expect("PooledBuffer is empty")
    }
}

impl std::ops::DerefMut for PooledBuffer {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.buf.as_mut().expect("PooledBuffer is empty")
    }
}

impl Drop for PooledBuffer {
    fn drop(&mut self) {
        if let (Some(pool_arc), Some(buf)) = (self.pool.take(), self.buf.take()) {
            if let Ok(mut pool) = pool_arc.lock() {
                pool.release(buf);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_buffer_pool_recycling() {
        let mut pool = BufferPool::new(4, 2, 1);
        let buf1 = pool.acquire(20 * 1024);
        assert!(buf1.capacity() >= TIER_SMALL);
        pool.release(buf1);

        assert_eq!(pool.stats().releases, 1);
        let buf2 = pool.acquire(10 * 1024);
        assert_eq!(pool.stats().hits, 1);
        assert!(buf2.capacity() >= TIER_SMALL);
    }

    #[test]
    fn test_shared_pooled_buffer_raii() {
        let shared = SharedBufferPool::new(2, 2, 1);
        {
            let mut pbuf = shared.acquire(100 * 1024);
            pbuf.extend_from_slice(b"hello vitadeck");
            assert_eq!(&pbuf[..], b"hello vitadeck");
        } 

        let pbuf2 = shared.acquire(50 * 1024);
        assert!(pbuf2.is_empty());
        assert!(pbuf2.capacity() >= TIER_MEDIUM);
    }
}
