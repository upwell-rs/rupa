//! A small least-recently-used map with O(1) `get`, `insert` and eviction.
//!
//! Used for rupa's bounded caches (rendered SQL, prepared statements). Kept
//! here rather than pulling in a dependency: it is a slab of entries linked
//! in recency order, plus an index from key to slot.

use std::borrow::Borrow;
use std::collections::HashMap;
use std::collections::hash_map::RandomState;
use std::fmt;
use std::hash::{BuildHasher, Hash};

const NIL: usize = usize::MAX;

struct Node<K, V> {
    key: K,
    value: V,
    prev: usize,
    next: usize,
}

/// A map holding at most `capacity` entries; inserting into a full map
/// evicts the least recently used one. A capacity of `0` stores nothing.
///
/// `S` hashes keys, as for `HashMap`; keys that are already hashes can use a
/// cheaper one.
pub struct LruCache<K, V, S = RandomState> {
    index: HashMap<K, usize, S>,
    nodes: Vec<Node<K, V>>,
    /// Most recently used.
    head: usize,
    /// Least recently used.
    tail: usize,
    capacity: usize,
}

impl<K: Hash + Eq + Clone, V> LruCache<K, V> {
    /// An empty cache for at most `capacity` entries.
    pub fn new(capacity: usize) -> Self {
        Self::with_hasher(capacity, RandomState::new())
    }
}

impl<K: Hash + Eq + Clone, V, S: BuildHasher> LruCache<K, V, S> {
    /// An empty cache for at most `capacity` entries, hashing keys with
    /// `hasher`.
    pub fn with_hasher(capacity: usize, hasher: S) -> Self {
        Self {
            index: HashMap::with_capacity_and_hasher(capacity.min(1024), hasher),
            nodes: Vec::with_capacity(capacity.min(1024)),
            head: NIL,
            tail: NIL,
            capacity,
        }
    }

    /// The most entries the cache holds.
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// The number of entries.
    pub fn len(&self) -> usize {
        self.index.len()
    }

    /// Whether the cache is empty.
    pub fn is_empty(&self) -> bool {
        self.index.is_empty()
    }

    /// The value for `key`, marking it most recently used.
    pub fn get<Q>(&mut self, key: &Q) -> Option<&V>
    where
        K: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        let slot = *self.index.get(key)?;
        self.touch(slot);
        Some(&self.nodes[slot].value)
    }

    /// The value for `key`, without changing its recency.
    pub fn peek<Q>(&self, key: &Q) -> Option<&V>
    where
        K: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        self.index.get(key).map(|&slot| &self.nodes[slot].value)
    }

    /// Inserts `value` as the most recently used entry, replacing any value
    /// for `key`. Returns the entry evicted to make room, if any.
    pub fn insert(&mut self, key: K, value: V) -> Option<(K, V)> {
        if self.capacity == 0 {
            return Some((key, value));
        }
        if let Some(&slot) = self.index.get(&key) {
            self.nodes[slot].value = value;
            self.touch(slot);
            return None;
        }
        let mut evicted = None;
        let slot = if self.nodes.len() < self.capacity {
            self.nodes.push(Node {
                key: key.clone(),
                value,
                prev: NIL,
                next: NIL,
            });
            self.nodes.len() - 1
        } else {
            // Reuse the least recently used slot.
            let slot = self.tail;
            self.unlink(slot);
            let node = &mut self.nodes[slot];
            let old_key = std::mem::replace(&mut node.key, key.clone());
            let old_value = std::mem::replace(&mut node.value, value);
            self.index.remove(&old_key);
            evicted = Some((old_key, old_value));
            slot
        };
        self.index.insert(key, slot);
        self.push_front(slot);
        evicted
    }

    /// Removes the entry for `key`, returning its value.
    pub fn remove<Q>(&mut self, key: &Q) -> Option<V>
    where
        K: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        let slot = self.index.remove(key)?;
        self.unlink(slot);
        // Keep the slab dense: move the last node into the freed slot.
        let last = self.nodes.len() - 1;
        if slot != last {
            let moved_prev = self.nodes[last].prev;
            let moved_next = self.nodes[last].next;
            self.nodes.swap(slot, last);
            if moved_prev != NIL {
                self.nodes[moved_prev].next = slot;
            } else {
                self.head = slot;
            }
            if moved_next != NIL {
                self.nodes[moved_next].prev = slot;
            } else {
                self.tail = slot;
            }
            let moved_key = &self.nodes[slot].key;
            *self.index.get_mut::<K>(moved_key).expect("indexed") = slot;
        }
        self.nodes.pop().map(|n| n.value)
    }

    /// Removes every entry.
    pub fn clear(&mut self) {
        self.index.clear();
        self.nodes.clear();
        self.head = NIL;
        self.tail = NIL;
    }

    /// Changes the capacity, evicting the least recently used entries that
    /// no longer fit.
    pub fn set_capacity(&mut self, capacity: usize) {
        self.capacity = capacity;
        while self.len() > capacity {
            let key = self.nodes[self.tail].key.clone();
            self.remove(&key);
        }
    }

    fn touch(&mut self, slot: usize) {
        if self.head != slot {
            self.unlink(slot);
            self.push_front(slot);
        }
    }

    fn unlink(&mut self, slot: usize) {
        let (prev, next) = (self.nodes[slot].prev, self.nodes[slot].next);
        if prev != NIL {
            self.nodes[prev].next = next;
        } else {
            self.head = next;
        }
        if next != NIL {
            self.nodes[next].prev = prev;
        } else {
            self.tail = prev;
        }
        self.nodes[slot].prev = NIL;
        self.nodes[slot].next = NIL;
    }

    fn push_front(&mut self, slot: usize) {
        self.nodes[slot].prev = NIL;
        self.nodes[slot].next = self.head;
        if self.head != NIL {
            self.nodes[self.head].prev = slot;
        }
        self.head = slot;
        if self.tail == NIL {
            self.tail = slot;
        }
    }
}

impl<K, V, S> fmt::Debug for LruCache<K, V, S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LruCache")
            .field("len", &self.index.len())
            .field("capacity", &self.capacity)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(c: &LruCache<u32, u32>) -> Vec<u32> {
        let mut out = Vec::new();
        let mut slot = c.head;
        while slot != NIL {
            out.push(c.nodes[slot].key);
            slot = c.nodes[slot].next;
        }
        out
    }

    #[test]
    fn evicts_the_least_recently_used() {
        let mut c = LruCache::new(3);
        for k in 1..=3 {
            assert_eq!(c.insert(k, k * 10), None);
        }
        assert_eq!(c.get(&1), Some(&10)); // 1 is now the most recent
        assert_eq!(c.insert(4, 40), Some((2, 20)));
        assert_eq!(keys(&c), [4, 1, 3]);
        assert_eq!(c.peek(&3), Some(&30));
        assert_eq!(keys(&c), [4, 1, 3], "peek leaves recency alone");
        assert_eq!(c.insert(1, 11), None, "replacing does not evict");
        assert_eq!(keys(&c), [1, 4, 3]);
    }

    #[test]
    fn remove_keeps_links_and_index_consistent() {
        let mut c = LruCache::new(4);
        for k in 1..=4 {
            c.insert(k, k);
        }
        assert_eq!(c.remove(&3), Some(3));
        assert_eq!(c.remove(&1), Some(1));
        assert_eq!(keys(&c), [4, 2]);
        for k in [2, 4] {
            assert_eq!(c.get(&k), Some(&k));
        }
        c.insert(5, 5);
        c.insert(6, 6);
        assert_eq!(c.insert(7, 7), Some((2, 2)));
        assert_eq!(keys(&c), [7, 6, 5, 4]);
    }

    #[test]
    fn capacity_changes_and_zero() {
        let mut c = LruCache::new(4);
        for k in 1..=4 {
            c.insert(k, k);
        }
        c.set_capacity(2);
        assert_eq!(keys(&c), [4, 3]);
        let mut none = LruCache::new(0);
        assert_eq!(none.insert(1, 1), Some((1, 1)));
        assert!(none.is_empty());
    }

    #[test]
    fn matches_a_reference_model() {
        // A deterministic pseudo-random workload against a naive model.
        let mut c = LruCache::new(8);
        let mut model: Vec<(u32, u32)> = Vec::new(); // most recent first
        let mut x = 12345u32;
        for step in 0..5000u32 {
            x = x.wrapping_mul(1_103_515_245).wrapping_add(12345);
            let key = (x >> 16) % 20;
            match x % 3 {
                0 => {
                    c.insert(key, step);
                    model.retain(|(k, _)| *k != key);
                    model.insert(0, (key, step));
                    model.truncate(8);
                }
                1 => {
                    let got = c.get(&key).copied();
                    let pos = model.iter().position(|(k, _)| *k == key);
                    assert_eq!(got, pos.map(|p| model[p].1));
                    if let Some(p) = pos {
                        let e = model.remove(p);
                        model.insert(0, e);
                    }
                }
                _ => {
                    let got = c.remove(&key);
                    let pos = model.iter().position(|(k, _)| *k == key);
                    assert_eq!(got, pos.map(|p| model.remove(p).1));
                }
            }
            assert_eq!(keys(&c), model.iter().map(|(k, _)| *k).collect::<Vec<_>>());
        }
    }
}
