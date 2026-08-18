use std::borrow::Borrow;
use std::collections::HashMap;
use std::hash::Hash;

struct LruNode<K, V> {
    key: K,
    value: V,
    cost: usize,
    prev: Option<usize>,
    next: Option<usize>,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct LruStats {
    pub hits: u64,
    pub misses: u64,
    pub evictions: u64,
    pub total_cost_bytes: usize,
    pub item_count: usize,
}

pub struct LruCache<K, V> {
    nodes: Vec<Option<LruNode<K, V>>>,
    free_indices: Vec<usize>,
    map: HashMap<K, usize>,
    head: Option<usize>, 
    tail: Option<usize>, 
    max_cost_bytes: usize,
    total_cost_bytes: usize,
    stats: LruStats,
}

impl<K: Clone + Eq + Hash, V> LruCache<K, V> {
    pub fn new(max_cost_bytes: usize) -> Self {
        Self {
            nodes: Vec::new(),
            free_indices: Vec::new(),
            map: HashMap::new(),
            head: None,
            tail: None,
            max_cost_bytes,
            total_cost_bytes: 0,
            stats: LruStats::default(),
        }
    }

    pub fn max_cost_bytes(&self) -> usize {
        self.max_cost_bytes
    }

    pub fn set_max_cost_bytes(&mut self, max_bytes: usize) {
        self.max_cost_bytes = max_bytes;
        self.evict_to_fit(0);
    }

    pub fn total_cost_bytes(&self) -> usize {
        self.total_cost_bytes
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    pub fn stats(&self) -> LruStats {
        let mut s = self.stats;
        s.total_cost_bytes = self.total_cost_bytes;
        s.item_count = self.len();
        s
    }

    pub fn contains_key<Q>(&self, key: &Q) -> bool
    where
        K: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        self.map.contains_key(key)
    }

    pub fn get<Q>(&mut self, key: &Q) -> Option<&V>
    where
        K: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        if let Some(&idx) = self.map.get(key) {
            self.touch(idx);
            self.stats.hits += 1;
            self.nodes[idx].as_ref().map(|n| &n.value)
        } else {
            self.stats.misses += 1;
            None
        }
    }

    pub fn get_mut<Q>(&mut self, key: &Q) -> Option<&mut V>
    where
        K: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        if let Some(&idx) = self.map.get(key) {
            self.touch(idx);
            self.stats.hits += 1;
            self.nodes[idx].as_mut().map(|n| &mut n.value)
        } else {
            self.stats.misses += 1;
            None
        }
    }

    pub fn peek<Q>(&self, key: &Q) -> Option<&V>
    where
        K: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        if let Some(&idx) = self.map.get(key) {
            self.nodes[idx].as_ref().map(|n| &n.value)
        } else {
            None
        }
    }

    pub fn insert(&mut self, key: K, value: V, cost_bytes: usize) -> Option<V> {
        let old = self.remove(&key);

        self.evict_to_fit(cost_bytes);

        let idx = if let Some(free_idx) = self.free_indices.pop() {
            self.nodes[free_idx] = Some(LruNode {
                key: key.clone(),
                value,
                cost: cost_bytes,
                prev: None,
                next: None,
            });
            free_idx
        } else {
            let idx = self.nodes.len();
            self.nodes.push(Some(LruNode {
                key: key.clone(),
                value,
                cost: cost_bytes,
                prev: None,
                next: None,
            }));
            idx
        };

        self.map.insert(key, idx);
        self.total_cost_bytes += cost_bytes;
        self.push_front(idx);

        old
    }

    pub fn remove<Q>(&mut self, key: &Q) -> Option<V>
    where
        K: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        let idx = self.map.remove(key)?;
        self.unlink(idx);
        let node = self.nodes[idx].take()?;
        self.free_indices.push(idx);
        self.total_cost_bytes = self.total_cost_bytes.saturating_sub(node.cost);
        Some(node.value)
    }

    pub fn evict_lru(&mut self) -> Option<(K, V)> {
        let tail_idx = self.tail?;
        self.unlink(tail_idx);
        let node = self.nodes[tail_idx].take()?;
        self.map.remove(&node.key);
        self.free_indices.push(tail_idx);
        self.total_cost_bytes = self.total_cost_bytes.saturating_sub(node.cost);
        self.stats.evictions += 1;
        Some((node.key, node.value))
    }

    pub fn evict_to_fit(&mut self, required_cost: usize) {
        if self.max_cost_bytes == 0 {
            return;
        }
        while self.total_cost_bytes + required_cost > self.max_cost_bytes && self.tail.is_some() {
            self.evict_lru();
        }
    }

    pub fn evict_fraction(&mut self, fraction: f32) -> usize {
        let fraction = fraction.clamp(0.0, 1.0);
        let target_cost = ((self.total_cost_bytes as f32) * (1.0 - fraction)) as usize;
        let mut count = 0;
        while self.total_cost_bytes > target_cost && self.tail.is_some() {
            if self.evict_lru().is_some() {
                count += 1;
            }
        }
        count
    }

    pub fn clear(&mut self) {
        self.nodes.clear();
        self.free_indices.clear();
        self.map.clear();
        self.head = None;
        self.tail = None;
        self.total_cost_bytes = 0;
    }

    fn touch(&mut self, idx: usize) {
        if self.head == Some(idx) {
            return;
        }
        self.unlink(idx);
        self.push_front(idx);
    }

    fn push_front(&mut self, idx: usize) {
        let old_head = self.head;
        if let Some(h) = old_head {
            if let Some(node) = self.nodes[h].as_mut() {
                node.prev = Some(idx);
            }
        }
        if let Some(node) = self.nodes[idx].as_mut() {
            node.prev = None;
            node.next = old_head;
        }
        self.head = Some(idx);
        if self.tail.is_none() {
            self.tail = Some(idx);
        }
    }

    fn unlink(&mut self, idx: usize) {
        let (prev, next) = match &self.nodes[idx] {
            Some(node) => (node.prev, node.next),
            None => return,
        };

        if let Some(p) = prev {
            if let Some(node) = self.nodes[p].as_mut() {
                node.next = next;
            }
        } else {
            self.head = next;
        }

        if let Some(n) = next {
            if let Some(node) = self.nodes[n].as_mut() {
                node.prev = prev;
            }
        } else {
            self.tail = prev;
        }

        if let Some(node) = self.nodes[idx].as_mut() {
            node.prev = None;
            node.next = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lru_basic_put_get() {
        let mut cache = LruCache::<String, i32>::new(100);
        cache.insert("a".to_string(), 10, 20);
        cache.insert("b".to_string(), 20, 30);
        assert_eq!(cache.get("a"), Some(&10));
        assert_eq!(cache.get("b"), Some(&20));
        assert_eq!(cache.total_cost_bytes(), 50);
    }

    #[test]
    fn test_lru_eviction_on_capacity() {
        let mut cache = LruCache::<String, i32>::new(50);
        cache.insert("a".to_string(), 1, 20);
        cache.insert("b".to_string(), 2, 20);
        cache.insert("c".to_string(), 3, 20);
        assert_eq!(cache.get("a"), None);
        assert_eq!(cache.get("b"), Some(&2));
        assert_eq!(cache.get("c"), Some(&3));
        assert_eq!(cache.total_cost_bytes(), 40);
    }

    #[test]
    fn test_lru_evict_fraction() {
        let mut cache = LruCache::<i32, i32>::new(1000);
        for i in 0..10 {
            cache.insert(i, i * 10, 100);
        }
        assert_eq!(cache.total_cost_bytes(), 1000);
        assert_eq!(cache.len(), 10);
        let evicted = cache.evict_fraction(0.5);
        assert_eq!(evicted, 5);
        assert_eq!(cache.len(), 5);
        assert_eq!(cache.total_cost_bytes(), 500);
    }
}
