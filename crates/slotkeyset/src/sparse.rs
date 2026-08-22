use slotmap::sparse_secondary::{Entry, IntoIter, Keys};
use slotmap::{Key, SparseSecondaryMap};
use std::iter::{ExactSizeIterator, FusedIterator};

/// A set backed by a SparseSecondaryMap
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SparseSlotKeySet<K: Key> {
    map: SparseSecondaryMap<K, ()>,
}

impl<K: Key> SparseSlotKeySet<K> {
    pub fn new() -> Self {
        SparseSlotKeySet {
            map: SparseSecondaryMap::new(),
        }
    }

    pub fn with_capacity(capacity: usize) -> Self {
        SparseSlotKeySet {
            map: SparseSecondaryMap::with_capacity(capacity),
        }
    }
}

impl<K: Key> SparseSlotKeySet<K> {
    pub fn capacity(&self) -> usize {
        self.map.capacity()
    }

    pub fn iter(&self) -> impl FusedIterator<Item = K> + ExactSizeIterator + Clone + '_ {
        self.map.keys()
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    pub fn drain(&mut self) -> impl FusedIterator<Item = K> + ExactSizeIterator + '_ {
        self.map.drain().map(|(k, _)| k)
    }

    pub fn retain<F>(&mut self, mut f: impl FnMut(K) -> bool) {
        self.map.retain(|k, _| f(k))
    }

    pub fn clear(&mut self) {
        self.map.clear()
    }

    pub fn difference<'a>(
        &'a self,
        other: &'a SparseSlotKeySet<K>,
    ) -> impl FusedIterator<Item = K> + Clone + 'a {
        self.map.keys().filter(|&k| !other.contains(k))
    }

    pub fn symmetric_difference<'a>(
        &'a self,
        other: &'a SparseSlotKeySet<K>,
    ) -> impl FusedIterator<Item = K> + Clone + 'a {
        self.difference(other).chain(other.difference(self))
    }

    pub fn intersection<'a>(
        &'a self,
        other: &'a SparseSlotKeySet<K>,
    ) -> impl FusedIterator<Item = K> + Clone + 'a {
        let (this, other) = if self.len() <= other.len() {
            (self, other)
        } else {
            (other, self)
        };
        this.map.keys().filter(|&k| other.contains(k))
    }

    pub fn union<'a>(
        &'a self,
        other: &'a SparseSlotKeySet<K>,
    ) -> impl FusedIterator<Item = K> + Clone + 'a {
        self.iter().chain(other.difference(self))
    }

    pub fn contains(&self, key: K) -> bool {
        self.map.contains_key(key)
    }

    pub fn is_disjoint(&self, other: &SparseSlotKeySet<K>) -> bool {
        if self.len() <= other.len() {
            self.iter().all(|v| !other.contains(v))
        } else {
            other.iter().all(|v| !self.contains(v))
        }
    }

    pub fn is_subset(&self, other: &SparseSlotKeySet<K>) -> bool {
        if self.len() <= other.len() {
            self.iter().all(|v| other.contains(v))
        } else {
            false
        }
    }

    pub fn is_superset(&self, other: &SparseSlotKeySet<K>) -> bool {
        other.is_subset(self)
    }

    pub fn insert(&mut self, key: K) -> bool {
        match self.map.entry(key) {
            None | Some(Entry::Occupied(_)) => false,
            Some(Entry::Vacant(entry)) => {
                entry.insert(());
                true
            }
        }
    }

    pub fn remove(&mut self, key: K) -> bool {
        match self.map.entry(key) {
            None | Some(Entry::Vacant(_)) => false,
            Some(Entry::Occupied(entry)) => {
                entry.remove();
                true
            }
        }
    }
}

impl<K: Key> IntoIterator for SparseSlotKeySet<K> {
    type Item = K;
    type IntoIter = std::iter::Map<IntoIter<K, ()>, fn((K, ())) -> K>;

    fn into_iter(self) -> Self::IntoIter {
        self.map.into_iter().map(|(k, _)| k)
    }
}

impl<'a, K: Key> IntoIterator for &'a SparseSlotKeySet<K> {
    type Item = K;
    type IntoIter = Keys<'a, K, ()>;

    fn into_iter(self) -> Self::IntoIter {
        self.map.keys()
    }
}

impl<K: Key> FromIterator<K> for SparseSlotKeySet<K> {
    fn from_iter<T: IntoIterator<Item = K>>(iter: T) -> Self {
        SparseSlotKeySet {
            map: iter.into_iter().map(|k| (k, ())).collect(),
        }
    }
}
