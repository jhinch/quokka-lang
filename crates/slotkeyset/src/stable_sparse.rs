use indexmap::IndexSet;
use slotmap::Key;
use std::iter::{Copied, ExactSizeIterator, FusedIterator};
/// A set backed by a IndexSet to provide a repeatable iteration order.
/// Order is not guaranteed to be insertion order as removals will use swap_remove
/// instead of a shift_remove
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct StableSparseSlotKeySet<K: Key> {
    set: IndexSet<K>,
}

impl<K: Key> StableSparseSlotKeySet<K> {
    #[must_use]
    pub fn new() -> Self {
        StableSparseSlotKeySet {
            set: IndexSet::new(),
        }
    }

    #[must_use]
    pub fn with_capacity(capacity: usize) -> Self {
        StableSparseSlotKeySet {
            set: IndexSet::with_capacity(capacity),
        }
    }
}

impl<K: Key> StableSparseSlotKeySet<K> {
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.set.capacity()
    }

    #[must_use]
    pub fn iter(&self) -> impl FusedIterator<Item = K> + ExactSizeIterator + Clone + '_ {
        self.set.iter().copied()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.set.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.set.is_empty()
    }

    pub fn drain(&mut self) -> impl FusedIterator<Item = K> + ExactSizeIterator + '_ {
        self.set.drain(..)
    }

    pub fn retain<F>(&mut self, mut f: impl FnMut(K) -> bool) {
        self.set.retain(|&k| f(k));
    }

    pub fn clear(&mut self) {
        self.set.clear();
    }

    #[must_use]
    pub fn difference<'a>(
        &'a self,
        other: &'a StableSparseSlotKeySet<K>,
    ) -> impl FusedIterator<Item = K> + Clone + 'a {
        self.set.difference(&other.set).copied()
    }

    #[must_use]
    pub fn symmetric_difference<'a>(
        &'a self,
        other: &'a StableSparseSlotKeySet<K>,
    ) -> impl FusedIterator<Item = K> + Clone + 'a {
        self.set.symmetric_difference(&other.set).copied()
    }

    #[must_use]
    pub fn intersection<'a>(
        &'a self,
        other: &'a StableSparseSlotKeySet<K>,
    ) -> impl FusedIterator<Item = K> + Clone + 'a {
        self.set.intersection(&other.set).copied()
    }

    #[must_use]
    pub fn union<'a>(
        &'a self,
        other: &'a StableSparseSlotKeySet<K>,
    ) -> impl FusedIterator<Item = K> + Clone + 'a {
        self.iter().chain(other.difference(self))
    }

    pub fn contains(&self, key: K) -> bool {
        self.set.contains(&key)
    }

    #[must_use]
    pub fn is_disjoint(&self, other: &StableSparseSlotKeySet<K>) -> bool {
        self.set.is_disjoint(&other.set)
    }

    #[must_use]
    pub fn is_subset(&self, other: &StableSparseSlotKeySet<K>) -> bool {
        self.set.is_subset(&other.set)
    }

    #[must_use]
    pub fn is_superset(&self, other: &StableSparseSlotKeySet<K>) -> bool {
        self.set.is_superset(&other.set)
    }

    pub fn insert(&mut self, key: K) -> bool {
        self.set.insert(key)
    }

    pub fn remove(&mut self, key: K) -> bool {
        self.set.swap_remove(&key)
    }
}

impl<K: Key> IntoIterator for StableSparseSlotKeySet<K> {
    type Item = K;
    type IntoIter = indexmap::set::IntoIter<K>;

    fn into_iter(self) -> Self::IntoIter {
        self.set.into_iter()
    }
}

impl<'a, K: Key> IntoIterator for &'a StableSparseSlotKeySet<K> {
    type Item = K;
    type IntoIter = Copied<indexmap::set::Iter<'a, K>>;

    fn into_iter(self) -> Self::IntoIter {
        self.set.iter().copied()
    }
}

impl<K: Key> FromIterator<K> for StableSparseSlotKeySet<K> {
    fn from_iter<T: IntoIterator<Item = K>>(iter: T) -> Self {
        StableSparseSlotKeySet {
            set: iter.into_iter().collect(),
        }
    }
}
