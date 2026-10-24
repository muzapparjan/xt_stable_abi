//! A map whose entries are ordered by the hash of their key.
//!
//! [`crate::StableMap`] orders its entries by the key itself. This map keeps the
//! same guarantee - two maps that hold the same entries have the same bytes,
//! whatever order the entries were inserted in - and orders them by a number
//! computed from the key, so a search compares one word before it looks at the
//! key. Both are binary searches over an ordered block; the difference is what
//! the order is made of.
//!
//! The hash is computed by the caller-supplied function, so the crate does not
//! depend on a hasher and does not take a random seed: a map built from the same
//! entries is the same value on every build and in every process, which is what
//! lets it cross the boundary.

use core::fmt;

use crate::alloc::AbiAllocator;
use crate::error::{AbiError, AbiResult, AbiStatus};
use crate::vec::{StableElement, StableVec};

/// The hash of a key, followed by the key and the value.
///
/// The hash comes first so the comparison a search makes is against one word;
/// the key follows it so two different keys with the same hash still sort in one
/// order on both sides.
#[repr(C)]
pub struct StableHashEntry<K, V> {
    /// The hash the key produces, as computed by the caller's function.
    pub hash: u64,
    /// The key.
    pub key: K,
    /// The value stored for it.
    pub value: V,
}

impl<K, V> StableHashEntry<K, V> {
    /// Builds an entry.
    pub const fn new(hash: u64, key: K, value: V) -> Self {
        Self { hash, key, value }
    }
}

impl<K: fmt::Debug, V: fmt::Debug> fmt::Debug for StableHashEntry<K, V> {
    // The hash is computed from the key, so reporting it here would print
    // information the key's own debug form withholds: a field which hides its
    // contents still leaks a fingerprint of them through the number. The fields
    // decide what they reveal; this form only names them.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StableHashEntry")
            .field("key", &self.key)
            .field("value", &self.value)
            .finish()
    }
}

// SAFETY: the entry is a `#[repr(C)]` struct of its three fields, and dropping it
// drops all of them in declaration order.
unsafe impl<K: StableElement, V: StableElement> StableElement for StableHashEntry<K, V> {
    unsafe fn drop_in_place(ptr: *mut Self) {
        // SAFETY: the caller guarantees an initialised value.
        unsafe { core::ptr::drop_in_place(ptr) }
    }
}

/// A map that finds a key by its hash.
///
/// Entries are kept in one vector, ordered by `(hash, key)`. Two maps that were
/// given the same entries and the same function are the same bytes even when the
/// entries arrived in different orders, because the order is a property of the
/// contents.
///
/// An entry is identified by `(hash, key)`: the number is part of the identity,
/// not an index that a search can do without. The map does not store the
/// function that produced the numbers, so it only answers for the numbers the
/// same function produces: a second function that answers differently for a key
/// names a different entry rather than the same one. Callers therefore use one
/// function for the whole life of a map, and [`StableHashMap::validate_with`]
/// checks a map against the function it will be read with.
///
/// A key whose type implements [`StableHash`](crate::stable_hash::StableHash) has
/// a function both sides already agree on, because this crate computes it:
/// [`StableHashMap::insert_stable`] builds such a map and
/// [`StableHashMap::validate_stable`] checks it without the caller passing
/// anything.
///
/// A key needs `Ord` only to decide the order of two keys that produce the same
/// hash; the search itself compares `u64` values.
///
/// There is no `Extend` or `FromIterator` implementation here, unlike the rest of
/// the collections: every insertion needs the hash function the caller chose, and
/// a trait that supplies entries alone would have to pick one. [`StableHashMap::try_extend`]
/// is the form those implementations take.
#[repr(C)]
pub struct StableHashMap<K: StableElement, V: StableElement> {
    entries: StableVec<StableHashEntry<K, V>>,
}

impl<K: StableElement + Ord, V: StableElement> StableHashMap<K, V> {
    /// An empty map whose block can hold `capacity` entries without growing.
    pub fn with_capacity(capacity: u64) -> AbiResult<Self> {
        Self::with_capacity_and_allocator(AbiAllocator::system(), capacity)
    }

    /// An empty map that uses `allocator` and holds room for `capacity` entries.
    pub fn with_capacity_and_allocator(
        allocator: AbiAllocator,
        capacity: u64,
    ) -> AbiResult<Self> {
        Ok(Self {
            entries: StableVec::with_capacity_and_allocator(allocator, capacity)?,
        })
    }

    /// The number of entries the block can hold without growing.
    pub const fn capacity(&self) -> u64 {
        self.entries.capacity()
    }

    /// Makes room for `additional` more entries.
    ///
    /// The entries are moved into the larger block in their stored order, so the
    /// map stays ordered by `(hash, key)`.
    pub fn try_reserve(&mut self, additional: u64) -> AbiResult<()> {
        self.entries.try_reserve(additional)
    }

    /// Removes every entry, releasing their keys and values.
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Removes every entry for which `keep` answers false.
    ///
    /// The order of the remaining entries is unchanged, so the map stays sorted
    /// by `(hash, key)` and needs no second sort.
    pub fn retain<F: FnMut(&K, &mut V) -> bool>(&mut self, mut keep: F) {
        self.entries
            .retain_mut(|entry| keep(&entry.key, &mut entry.value));
    }

    /// The keys and values, in the order the map stores them.
    pub fn iter(&self) -> impl Iterator<Item = (&K, &V)> {
        self.entries.as_slice().iter().map(|entry| (&entry.key, &entry.value))
    }

    /// The keys, in the order the map stores them.
    pub fn keys(&self) -> impl Iterator<Item = &K> {
        self.entries.as_slice().iter().map(|entry| &entry.key)
    }

    /// The values, in the order the map stores them.
    pub fn values(&self) -> impl Iterator<Item = &V> {
        self.entries.as_slice().iter().map(|entry| &entry.value)
    }

    /// Copies the map, using the same allocator.
    ///
    /// The keys and values are copied by [`TryClone`](crate::clone::TryClone),
    /// and the stored hash of each entry is copied with it: the number names
    /// where the entry sits, and asking the key for it again would need the
    /// function the caller built the map with.
    pub fn try_clone(&self) -> AbiResult<Self>
    where
        K: crate::clone::TryClone,
        V: crate::clone::TryClone,
    {
        let mut entries =
            StableVec::with_capacity_and_allocator(self.entries.allocator(), self.entries.len())?;
        for entry in self.entries.as_slice() {
            entries.try_push(StableHashEntry::new(
                entry.hash,
                crate::clone::TryClone::try_clone(&entry.key)?,
                crate::clone::TryClone::try_clone(&entry.value)?,
            ))?;
        }
        Ok(Self { entries })
    }

    /// Adopts storage that arrived from another build.
    ///
    /// # Safety
    ///
    /// `pointer` must be a live block from `allocator` with at least `capacity`
    /// slots, of which the first `length` hold initialised entries ordered by
    /// `(hash, key)`.
    pub unsafe fn from_raw_parts(
        pointer: *mut StableHashEntry<K, V>,
        length: u64,
        capacity: u64,
        allocator: AbiAllocator,
    ) -> AbiResult<Self> {
        // SAFETY: the caller guarantees the block and its contents.
        let entries = unsafe { StableVec::from_raw_parts(pointer, length, capacity, allocator) }?;
        Ok(Self { entries })
    }

    /// Takes the storage apart without releasing it.
    ///
    /// # Safety
    ///
    /// The returned block must be released exactly once, after the entries it
    /// holds are released.
    pub unsafe fn into_raw_parts(self) -> (*mut StableHashEntry<K, V>, u64, u64, AbiAllocator) {
        // SAFETY: the caller takes over the block as this value gives it up.
        unsafe { self.entries.into_raw_parts() }
    }
}

/// One lookup of a key in a [`StableHashMap`].
///
/// Built by [`StableHashMap::entry`]; the lookup and the hash are done once, so
/// filling a vacant entry does not search the map a second time.
pub struct HashEntry<'a, K: StableElement, V: StableElement> {
    map: &'a mut StableHashMap<K, V>,
    /// The hash the key produced, which is the number the entry is stored under.
    hash: u64,
    /// The key to insert, or `None` when the map already holds a value for it.
    key: Option<K>,
    /// Where the entry is, or where it would be inserted.
    index: u64,
}

impl<K: StableElement + Ord, V: StableElement> HashEntry<'_, K, V> {
    /// The key this entry is about.
    pub fn key(&self) -> &K {
        match &self.key {
            Some(key) => key,
            None => &self.map.as_entries()[self.index as usize].key,
        }
    }

    /// Reports whether the map already holds a value for the key.
    pub fn is_occupied(&self) -> bool {
        self.key.is_none()
    }
}

impl<'a, K: StableElement + Ord, V: StableElement> HashEntry<'a, K, V> {
    /// Returns the value, filling the entry with `value` when it was vacant.
    pub fn or_insert(self, value: V) -> AbiResult<&'a mut V> {
        self.or_insert_with(|| value)
    }

    /// Returns the value, filling the entry with `make()` when it was vacant.
    pub fn or_insert_with<F: FnOnce() -> V>(self, make: F) -> AbiResult<&'a mut V> {
        let this = self;
        // An occupied entry keeps the value it already holds.
        if let Some(key) = this.key {
            this.map
                .entries
                .insert(this.index, StableHashEntry::new(this.hash, key, make()))?;
        }
        Ok(&mut this.map.entries.as_mut_slice()[this.index as usize].value)
    }

    /// Applies `update` to the value, filling the entry first from `make` when
    /// it was vacant.
    pub fn and_modify<F: FnOnce(&mut V), M: FnOnce() -> V>(
        self,
        make: M,
        update: F,
    ) -> AbiResult<&'a mut V> {
        let value = self.or_insert_with(make)?;
        update(value);
        Ok(value)
    }

    /// Returns the value, filling the entry with `V::default()` when it was
    /// vacant.
    pub fn or_default(self) -> AbiResult<&'a mut V>
    where
        V: Default,
    {
        self.or_insert_with(V::default)
    }
}

impl<K: StableElement + Ord, V: StableElement> fmt::Debug for HashEntry<'_, K, V> {
    // The key can be content a log must not repeat, so the debug form reports
    // whether the entry is occupied.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("HashEntry")
            .field("occupied", &self.is_occupied())
            .finish_non_exhaustive()
    }
}

impl<K: StableElement + Ord, V: StableElement> Default for StableHashMap<K, V> {
    fn default() -> Self {
        Self::new()
    }
}

impl<K: StableElement + Ord + PartialEq, V: StableElement + PartialEq> PartialEq
    for StableHashMap<K, V>
{
    /// Compares the stored entries, numbers included.
    ///
    /// An entry is identified by `(hash, key)`, and the order follows that pair,
    /// so two maps hold the same entries exactly when they list the same entries
    /// in the same order. Maps filled with different functions therefore differ:
    /// they store different numbers for the same keys.
    fn eq(&self, other: &Self) -> bool {
        let left = self.entries.as_slice();
        let right = other.entries.as_slice();
        if left.len() != right.len() {
            return false;
        }
        let mut index = 0;
        while index < left.len() {
            if left[index].hash != right[index].hash
                || left[index].key != right[index].key
                || left[index].value != right[index].value
            {
                return false;
            }
            index += 1;
        }
        true
    }
}

impl<K: StableElement + Ord + Eq, V: StableElement + Eq> Eq for StableHashMap<K, V> {}

impl<K, V> core::hash::Hash for StableHashMap<K, V>
where
    K: StableElement + Ord + core::hash::Hash,
    V: StableElement + core::hash::Hash,
{
    /// Folds the entries in the order the map stores them.
    ///
    /// The stored number takes part, because it is part of what identifies an
    /// entry: a map filled with another function holds different entries and
    /// must fold differently.
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        state.write_u64(self.len());
        for entry in self.entries.as_slice() {
            entry.hash.hash(state);
            entry.key.hash(state);
            entry.value.hash(state);
        }
    }
}

impl<K, V> Ord for StableHashMap<K, V>
where
    K: StableElement + Ord,
    V: StableElement + Ord,
{
    /// Orders two maps entry by entry - number, key, value - then by length.
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        let left = self.entries.as_slice();
        let right = other.entries.as_slice();
        let shared = left.len().min(right.len());
        let mut index = 0;
        while index < shared {
            let ordering = left[index]
                .hash
                .cmp(&right[index].hash)
                .then_with(|| left[index].key.cmp(&right[index].key))
                .then_with(|| left[index].value.cmp(&right[index].value));
            if ordering != core::cmp::Ordering::Equal {
                return ordering;
            }
            index += 1;
        }
        left.len().cmp(&right.len())
    }
}

impl<K, V> PartialOrd for StableHashMap<K, V>
where
    K: StableElement + Ord,
    V: StableElement + Ord,
{
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl<K: StableElement + Ord + Clone, V: StableElement + Clone> Clone for StableHashMap<K, V> {
    /// Copies the map, or panics when the allocation fails.
    ///
    /// # Panics
    ///
    /// Panics when the copy of a key or a value, or the allocation, fails; use
    /// [`StableHashMap::try_clone`] for a fallible version.
    fn clone(&self) -> Self {
        let mut entries = match StableVec::with_capacity_and_allocator(
            self.entries.allocator(),
            self.entries.len(),
        ) {
            Ok(entries) => entries,
            Err(error) => panic!("failed to clone a stable hash map: {error}"),
        };
        for entry in self.entries.as_slice() {
            if let Err(error) = entries.try_push(StableHashEntry::new(
                entry.hash,
                entry.key.clone(),
                entry.value.clone(),
            )) {
                panic!("failed to clone a stable hash map: {error}");
            }
        }
        Self { entries }
    }
}

impl<K: StableElement + Ord, V: StableElement> StableHashMap<K, V> {
    /// An empty map that uses the process allocator.
    pub const fn new() -> Self {
        Self {
            entries: StableVec::new(),
        }
    }

    /// An empty map that uses `allocator`.
    pub const fn new_with_allocator(allocator: AbiAllocator) -> Self {
        Self {
            entries: StableVec::new_with_allocator(allocator),
        }
    }

    /// The allocator that owns the storage.
    pub const fn allocator(&self) -> AbiAllocator {
        self.entries.allocator()
    }

    /// The number of entries.
    pub const fn len(&self) -> u64 {
        self.entries.len()
    }

    /// Reports whether the map holds no entries.
    pub const fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The entries in the order the map stores them.
    pub fn as_entries(&self) -> &[StableHashEntry<K, V>] {
        self.entries.as_slice()
    }

    /// The index an entry with this hash and key occupies, or where it would go.
    ///
    /// The hash comes from the caller rather than from `hash` here: a caller that
    /// calls the function once and passes the number keeps the number it looked
    /// up with the number it stores. Calling `hash` twice would be wrong for a
    /// function that is not one: a value that changes between the two calls would
    /// be stored under the first number and looked up by the second.
    fn locate(&self, target: u64, key: &K) -> core::result::Result<u64, u64> {
        let entries = self.entries.as_slice();
        let mut low = 0u64;
        let mut high = entries.len() as u64;
        while low < high {
            let middle = low + (high - low) / 2;
            let entry = &entries[middle as usize];
            let ordering = entry
                .hash
                .cmp(&target)
                .then_with(|| entry.key.cmp(key));
            match ordering {
                core::cmp::Ordering::Less => low = middle + 1,
                core::cmp::Ordering::Greater => high = middle,
                core::cmp::Ordering::Equal => return Ok(middle),
            }
        }
        Err(low)
    }

    /// The value stored for `key`, hashed with `hash`.
    pub fn get<F: Fn(&K) -> u64>(&self, hash: F, key: &K) -> Option<&V> {
        let index = self.locate(hash(key), key).ok()?;
        Some(&self.entries.as_slice()[index as usize].value)
    }

    /// The value stored for `key`, for writing.
    pub fn get_mut<F: Fn(&K) -> u64>(&mut self, hash: F, key: &K) -> Option<&mut V> {
        let index = self.locate(hash(key), key).ok()?;
        Some(&mut self.entries.as_mut_slice()[index as usize].value)
    }

    /// Stores `value` for `key`, answering with the value it replaced.
    ///
    /// An entry that has to be inserted shifts the entries after it, so filling a
    /// map one entry at a time costs one move per entry after the position. Use
    /// [`StableHashMap::from_entries`] to build a map from many entries at once.
    ///
    /// `hash` is called once per call, and the number it answers is both the one
    /// stored and the one searched for. It has to be a function of the key alone:
    /// a function that answers differently for equal keys would place an entry
    /// out of order, and the map's search would then miss it.
    ///
    /// An entry is identified by `(hash, key)`, so inserting a key the map holds
    /// under another number adds a second entry rather than replacing the first:
    /// the number is part of what names an entry, and the map cannot tell that
    /// two numbers mean one key without the function that produced them.
    pub fn insert<F: Fn(&K) -> u64>(
        &mut self,
        hash: F,
        key: K,
        value: V,
    ) -> AbiResult<Option<V>> {
        let computed = hash(&key);
        self.store_entry(StableHashEntry::new(computed, key, value))
    }

    /// Writes an entry whose number is already computed.
    ///
    /// The caller is responsible for the order the entry lands in: the number is
    /// the one the search uses, so `entry.hash` has to be the number `entry.key`
    /// produces.
    fn store_entry(&mut self, entry: StableHashEntry<K, V>) -> AbiResult<Option<V>> {
        match self.locate(entry.hash, &entry.key) {
            Ok(index) => {
                let slot = &mut self.entries.as_mut_slice()[index as usize];
                Ok(Some(core::mem::replace(&mut slot.value, entry.value)))
            }
            Err(index) => {
                self.entries.insert(index, entry)?;
                Ok(None)
            }
        }
    }

    /// Removes the entry for `key`, answering with its value.
    pub fn remove<F: Fn(&K) -> u64>(&mut self, hash: F, key: &K) -> Option<V> {
        let index = self.locate(hash(key), key).ok()?;
        self.entries.remove(index).map(|entry| entry.value)
    }

    /// The key and value stored for `key`.
    ///
    /// The key is the one the map holds, which is what a caller that looked the
    /// entry up with a borrowed key needs in order to keep it.
    pub fn get_key_value<F: Fn(&K) -> u64>(&self, hash: F, key: &K) -> Option<(&K, &V)> {
        let index = self.locate(hash(key), key).ok()?;
        let entry = &self.entries.as_slice()[index as usize];
        Some((&entry.key, &entry.value))
    }

    /// Reports whether `key` is stored, hashed with `hash`.
    pub fn contains_key<F: Fn(&K) -> u64>(&self, hash: F, key: &K) -> bool {
        self.locate(hash(key), key).is_ok()
    }

    /// Removes the entry for `key`, answering with the key and the value.
    pub fn remove_entry<F: Fn(&K) -> u64>(&mut self, hash: F, key: &K) -> Option<(K, V)> {
        let index = self.locate(hash(key), key).ok()?;
        self.entries
            .remove(index)
            .map(|entry| (entry.key, entry.value))
    }

    /// Removes every entry and yields them in the order the map stores them.
    ///
    /// The map stops covering the entries at once, so an entry the iterator has
    /// not reached is released when the iterator is dropped and never twice.
    pub fn drain(&mut self) -> impl Iterator<Item = (K, V)> + '_ {
        self.entries.drain().map(|entry| (entry.key, entry.value))
    }

    /// Takes the map apart, yielding its keys in the stored order and releasing
    /// the values.
    pub fn into_keys(self) -> impl Iterator<Item = K> {
        self.entries.into_iter().map(|entry| entry.key)
    }

    /// Takes the map apart, yielding its values in the stored order and
    /// releasing the keys.
    pub fn into_values(self) -> impl Iterator<Item = V> {
        self.entries.into_iter().map(|entry| entry.value)
    }

    /// The entry for `key`, looked up once.
    ///
    /// The hash is taken here rather than in the entry, so filling a vacant
    /// entry does not search the map a second time and does not call the hash
    /// function again.
    pub fn entry<F: Fn(&K) -> u64>(&mut self, hash: F, key: K) -> HashEntry<'_, K, V> {
        let computed = hash(&key);
        match self.locate(computed, &key) {
            Ok(index) => HashEntry {
                map: self,
                hash: computed,
                key: None,
                index,
            },
            Err(index) => HashEntry {
                map: self,
                hash: computed,
                key: Some(key),
                index,
            },
        }
    }

    /// Builds a map from entries, keeping the last value written for a repeated
    /// key, exactly as inserting them one after another would.
    pub fn from_entries<F: Fn(&K) -> u64>(
        hash: F,
        entries: &[(K, V)],
    ) -> AbiResult<Self>
    where
        K: Clone,
        V: Clone,
    {
        Self::from_entries_with_allocator(hash, AbiAllocator::system(), entries)
    }

    /// Builds a map from entries that uses `allocator`.
    pub fn from_entries_with_allocator<F: Fn(&K) -> u64>(
        hash: F,
        allocator: AbiAllocator,
        entries: &[(K, V)],
    ) -> AbiResult<Self>
    where
        K: Clone,
        V: Clone,
    {
        let mut kept =
            StableVec::with_capacity_and_allocator(allocator, entries.len() as u64)?;
        for (key, value) in entries {
            kept.try_push(StableHashEntry::new(
                hash(key),
                key.clone(),
                value.clone(),
            ))?;
        }
        // Reversing before a stable sort puts the last write for a key first in
        // its run; the deduplication then keeps that one, which is the value an
        // insert of these entries one after another would leave behind.
        kept.as_mut_slice().reverse();
        kept.try_sort_by(|left, right| {
            left.hash
                .cmp(&right.hash)
                .then_with(|| left.key.cmp(&right.key))
        })?;
        kept.dedup_by(|kept, candidate| kept.hash == candidate.hash && kept.key == candidate.key);
        Ok(Self { entries: kept })
    }

    /// Checks that the entries are ordered and that every key and value is a
    /// value its type defines.
    ///
    /// The stored number of each entry is not checked: recomputing it needs the
    /// function the map was built with, so a caller that reads a map from another
    /// build uses [`StableHashMap::validate_with`].
    ///
    /// An entry is identified by `(hash, key)`, and the check requires the pairs
    /// to be strictly ascending, which refuses one entry listed twice. Two entries
    /// that hold one key under two numbers are different entries and pass, which
    /// is why a map is read with the function that filled it.
    ///
    /// The check allocates nothing. It walks the entries twice: once to check
    /// every key and value, and once to compare each entry with the one before
    /// it, which is the order the storage promises.
    pub fn validate(&self) -> AbiResult<()>
    where
        K: crate::boundary::AbiType,
        V: crate::boundary::AbiType,
    {
        self.entries.validate()?;
        let entries = self.entries.as_slice();
        let mut index = 1;
        while index < entries.len() {
            let previous = &entries[index - 1];
            let current = &entries[index];
            if previous.hash > current.hash
                || (previous.hash == current.hash && previous.key >= current.key)
            {
                return Err(AbiError::new(AbiStatus::InvalidDescriptor));
            }
            index += 1;
        }
        Ok(())
    }

    /// Checks the entries against `hash` as well as against the layout.
    ///
    /// Every stored number has to be the one the key produces and has to name
    /// the place the entry sits in, which is what a receiver can only know when
    /// it holds the function the other side built the map with.
    pub fn validate_with<F: Fn(&K) -> u64>(&self, hash: F) -> AbiResult<()>
    where
        K: crate::boundary::AbiType,
        V: crate::boundary::AbiType,
    {
        self.validate()?;
        for entry in self.entries.as_slice() {
            if hash(&entry.key) != entry.hash {
                return Err(AbiError::new(AbiStatus::InvalidDescriptor));
            }
        }
        Ok(())
    }
}

impl<K: StableElement + Ord + fmt::Debug, V: StableElement + fmt::Debug> fmt::Debug
    for StableHashMap<K, V>
{
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StableHashMap")
            .field("len", &self.entries.len())
            .finish_non_exhaustive()
    }
}

// SAFETY: the map owns one vector, and dropping it releases exactly the entries
// that vector holds.
unsafe impl<K: StableElement, V: StableElement> StableElement for StableHashMap<K, V> {
    unsafe fn drop_in_place(ptr: *mut Self) {
        // SAFETY: the caller guarantees an initialised value.
        unsafe { core::ptr::drop_in_place(ptr) }
    }
}

impl<K: StableElement + crate::zeroize::Wipe, V: StableElement + crate::zeroize::Wipe>
    crate::zeroize::Wipe for StableHashEntry<K, V>
{
    /// Overwrites the key, the value and the number the entry is stored under.
    ///
    /// The number is a function of the key, so leaving it behind after the key
    /// is gone would keep a fingerprint of the content that was just erased: a
    /// dictionary can be compared against it as easily as against the key.
    fn wipe_in_place(&mut self) {
        // The number goes first and through a volatile store: the entry is about
        // to be released, so a plain assignment here would be a dead store the
        // compiler may drop, and the number is the one thing the key's own wipe
        // cannot reach.
        let _ = unsafe {
            crate::zeroize::volatile_zero_raw(
                core::ptr::addr_of_mut!(self.hash).cast::<u8>(),
                1,
                core::mem::size_of::<u64>() as u64,
            )
        };
        self.key.wipe_in_place();
        self.value.wipe_in_place();
    }
}

impl<K: StableElement + Ord + crate::zeroize::Wipe, V: StableElement + crate::zeroize::Wipe>
    crate::zeroize::Wipe for StableHashMap<K, V>
{
    /// Wipes every entry and the spare slots past the length.
    ///
    /// The vector underneath does both, and the spare slots matter: a removed
    /// entry is released where it lies without being written over, so its key and
    /// value can still be in the block.
    fn wipe_in_place(&mut self) {
        self.entries.wipe_in_place();
    }
}

/// A set that finds a value by its hash.
///
/// The values are kept in the same order the map keeps them, so two sets given
/// the same values and the same function are the same bytes whatever order the
/// values arrived in. A value is identified by `(hash, value)` for the same
/// reason it is in [`StableHashMap`]: the number is part of what names it, and
/// the set only answers for the numbers the function that filled it produces.
#[repr(C)]
pub struct StableHashSet<T: StableElement + Ord> {
    entries: StableHashMap<T, ()>,
}

impl<T: StableElement + Ord> StableHashSet<T> {
    /// An empty set that uses the process allocator.
    pub const fn new() -> Self {
        Self {
            entries: StableHashMap::new(),
        }
    }

    /// An empty set that uses `allocator`.
    pub const fn new_with_allocator(allocator: AbiAllocator) -> Self {
        Self {
            entries: StableHashMap::new_with_allocator(allocator),
        }
    }

    /// The allocator that owns the storage.
    pub const fn allocator(&self) -> AbiAllocator {
        self.entries.allocator()
    }

    /// The number of values.
    pub const fn len(&self) -> u64 {
        self.entries.len()
    }

    /// Reports whether the set holds no values.
    pub const fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The entries the set stores, in the order it keeps them.
    ///
    /// An entry carries the number its value is stored under together with the
    /// value; [`StableHashSet::iter`] answers the values alone.
    pub fn as_entries(&self) -> &[StableHashEntry<T, ()>] {
        self.entries.as_entries()
    }

    /// Reports whether `value` is in the set, hashed with `hash`.
    pub fn contains<F: Fn(&T) -> u64>(&self, hash: F, value: &T) -> bool {
        self.entries.get(hash, value).is_some()
    }

    /// Adds `value`, answering whether it was absent.
    pub fn insert<F: Fn(&T) -> u64>(&mut self, hash: F, value: T) -> AbiResult<bool> {
        Ok(self.entries.insert(hash, value, ())?.is_none())
    }

    /// Removes `value`, answering whether it was there.
    pub fn remove<F: Fn(&T) -> u64>(&mut self, hash: F, value: &T) -> bool {
        self.entries.remove(hash, value).is_some()
    }

    /// Builds a set from values, keeping one of a repeated value.
    pub fn from_values<F: Fn(&T) -> u64>(hash: F, values: &[T]) -> AbiResult<Self>
    where
        T: Clone,
    {
        Self::from_values_with_allocator(hash, AbiAllocator::system(), values)
    }

    /// Builds a set from values that uses `allocator`.
    pub fn from_values_with_allocator<F: Fn(&T) -> u64>(
        hash: F,
        allocator: AbiAllocator,
        values: &[T],
    ) -> AbiResult<Self>
    where
        T: Clone,
    {
        let mut kept = StableVec::with_capacity_and_allocator(allocator, values.len() as u64)?;
        for value in values {
            kept.try_push(StableHashEntry::new(hash(value), value.clone(), ()))?;
        }
        kept.as_mut_slice().reverse();
        kept.try_sort_by(|left, right| {
            left.hash
                .cmp(&right.hash)
                .then_with(|| left.key.cmp(&right.key))
        })?;
        kept.dedup_by(|kept, candidate| kept.hash == candidate.hash && kept.key == candidate.key);
        Ok(Self {
            entries: StableHashMap { entries: kept },
        })
    }

    /// Checks that the values are ordered and that every one of them is a value
    /// its type defines.
    pub fn validate(&self) -> AbiResult<()>
    where
        T: crate::boundary::AbiType,
    {
        self.entries.validate()
    }

    /// Checks the values against `hash` as well as against the layout.
    ///
    /// Every stored number has to be the one the value produces and has to name
    /// the place the entry sits in, which is what a receiver can only know when
    /// it holds the function the other side built the set with.
    pub fn validate_with<F: Fn(&T) -> u64>(&self, hash: F) -> AbiResult<()>
    where
        T: crate::boundary::AbiType,
    {
        self.entries.validate_with(hash)
    }
}

impl<T: StableElement + Ord> Default for StableHashSet<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: StableElement + Ord> StableHashSet<T> {
    /// An empty set whose block can hold `capacity` values without growing.
    pub fn with_capacity(capacity: u64) -> AbiResult<Self> {
        Self::with_capacity_and_allocator(AbiAllocator::system(), capacity)
    }

    /// An empty set that uses `allocator` and holds room for `capacity` values.
    pub fn with_capacity_and_allocator(
        allocator: AbiAllocator,
        capacity: u64,
    ) -> AbiResult<Self> {
        Ok(Self {
            entries: StableHashMap::with_capacity_and_allocator(allocator, capacity)?,
        })
    }

    /// The number of values the block can hold without growing.
    pub const fn capacity(&self) -> u64 {
        self.entries.capacity()
    }

    /// Makes room for `additional` more values.
    pub fn try_reserve(&mut self, additional: u64) -> AbiResult<()> {
        self.entries.try_reserve(additional)
    }

    /// The values in either set, hashed with `hash`.
    ///
    /// A hash set has to be given the function that names where a value sits, so
    /// this is the form the operation takes here; [`crate::StableSet::union`] is
    /// the one that needs no function.
    ///
    /// The values are collected, ordered once and deduplicated, so the cost is a
    /// sort rather than one insertion per value.
    pub fn union_with<F: Fn(&T) -> u64>(&self, hash: F, other: &Self) -> AbiResult<Self>
    where
        T: Clone,
    {
        Self::build(
            &hash,
            self.allocator(),
            self.iter().chain(other.iter()).cloned(),
            self.len() + other.len(),
        )
    }

    /// The values in both sets, hashed with `hash`.
    pub fn intersection_with<F: Fn(&T) -> u64>(&self, hash: F, other: &Self) -> AbiResult<Self>
    where
        T: Clone,
    {
        Self::build(
            &hash,
            self.allocator(),
            self.iter()
                .filter(|value| other.contains(&hash, value))
                .cloned(),
            self.len(),
        )
    }

    /// The values in this set that `other` does not hold, hashed with `hash`.
    pub fn difference_with<F: Fn(&T) -> u64>(&self, hash: F, other: &Self) -> AbiResult<Self>
    where
        T: Clone,
    {
        Self::build(
            &hash,
            self.allocator(),
            self.iter()
                .filter(|value| !other.contains(&hash, value))
                .cloned(),
            self.len(),
        )
    }

    /// The values in exactly one of the two sets, hashed with `hash`.
    pub fn symmetric_difference_with<F: Fn(&T) -> u64>(
        &self,
        hash: F,
        other: &Self,
    ) -> AbiResult<Self>
    where
        T: Clone,
    {
        Self::build(
            &hash,
            self.allocator(),
            self.iter()
                .filter(|value| !other.contains(&hash, value))
                .chain(other.iter().filter(|value| !self.contains(&hash, value)))
                .cloned(),
            self.len() + other.len(),
        )
    }

    /// Reports whether every value of `self` is in `other`, hashed with `hash`.
    pub fn is_subset_with<F: Fn(&T) -> u64>(&self, hash: F, other: &Self) -> bool {
        self.iter().all(|value| other.contains(&hash, value))
    }

    /// Reports whether every value of `other` is in `self`, hashed with `hash`.
    pub fn is_superset_with<F: Fn(&T) -> u64>(&self, hash: F, other: &Self) -> bool {
        other.is_subset_with(hash, self)
    }

    /// Reports whether the two sets hold no value in common, hashed with
    /// `hash`.
    pub fn is_disjoint_with<F: Fn(&T) -> u64>(&self, hash: F, other: &Self) -> bool {
        !self.iter().any(|value| other.contains(&hash, value))
    }

    /// Builds a set from `values`, ordering the entries once.
    ///
    /// A value that appears twice is kept once, and the entries end up ordered by
    /// `(hash, key)`, which is the order the map keeps them in. The hash is a
    /// function of the value, so two equal values carry the same number and the
    /// repeat sits next to its twin: one sort finds them, and one pass over the
    /// ordered entries removes them.
    fn build<F: Fn(&T) -> u64>(
        hash: &F,
        allocator: AbiAllocator,
        values: impl Iterator<Item = T>,
        capacity: u64,
    ) -> AbiResult<Self> {
        let mut kept = StableVec::with_capacity_and_allocator(allocator, capacity)?;
        for value in values {
            kept.try_push(StableHashEntry::new(hash(&value), value, ()))?;
        }
        kept.try_sort_by(|left, right| {
            left.hash
                .cmp(&right.hash)
                .then_with(|| left.key.cmp(&right.key))
        })?;
        kept.dedup_by(|kept, candidate| kept.hash == candidate.hash && kept.key == candidate.key);
        Ok(Self {
            entries: StableHashMap { entries: kept },
        })
    }

    /// Removes every value, releasing it.
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Removes every value for which `keep` answers false.
    pub fn retain<F: FnMut(&T) -> bool>(&mut self, mut keep: F) {
        self.entries.retain(|key, _| keep(key));
    }

    /// Removes every value and yields them in the order the set stores them.
    ///
    /// The set stops covering the values at once, so a value the iterator has
    /// not reached is released when the iterator is dropped and never twice.
    pub fn drain(&mut self) -> impl Iterator<Item = T> + '_ {
        self.entries.drain().map(|(value, ())| value)
    }

    /// The values, in the order the set stores them.
    pub fn iter(&self) -> impl Iterator<Item = &T> {
        self.entries.as_entries().iter().map(|entry| &entry.key)
    }

    /// Copies the set, using the same allocator.
    ///
    /// The values are copied by [`TryClone`](crate::clone::TryClone), and the
    /// stored hash of each value is copied with it.
    pub fn try_clone(&self) -> AbiResult<Self>
    where
        T: crate::clone::TryClone,
    {
        Ok(Self {
            entries: self.entries.try_clone()?,
        })
    }

    /// Adopts storage that arrived from another build.
    ///
    /// # Safety
    ///
    /// `pointer` must be a live block from `allocator` with at least `capacity`
    /// slots, of which the first `length` hold initialised entries ordered by
    /// `(hash, key)`.
    pub unsafe fn from_raw_parts(
        pointer: *mut StableHashEntry<T, ()>,
        length: u64,
        capacity: u64,
        allocator: AbiAllocator,
    ) -> AbiResult<Self> {
        // SAFETY: the caller guarantees the block and its contents.
        let entries = unsafe {
            StableHashMap::from_raw_parts(pointer, length, capacity, allocator)
        }?;
        Ok(Self { entries })
    }

    /// Takes the storage apart without releasing it.
    ///
    /// # Safety
    ///
    /// The returned block must be released exactly once, after the values it
    /// holds are released.
    pub unsafe fn into_raw_parts(self) -> (*mut StableHashEntry<T, ()>, u64, u64, AbiAllocator) {
        // SAFETY: the caller takes over the block as this value gives it up.
        unsafe { self.entries.into_raw_parts() }
    }
}

impl<T: StableElement + Ord + PartialEq> PartialEq for StableHashSet<T> {
    /// Compares the values in the order the sets store them.
    ///
    /// The order is a property of the contents, so two sets that hold the same
    /// values compare equal whatever order they were filled in.
    fn eq(&self, other: &Self) -> bool {
        self.entries == other.entries
    }
}

impl<T: StableElement + Ord + Eq> Eq for StableHashSet<T> {}

impl<T> core::hash::Hash for StableHashSet<T>
where
    T: StableElement + Ord + core::hash::Hash,
{
    /// Folds the values in the order the set stores them, number included, for
    /// the reason `StableHashMap` folds its entries that way.
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        self.entries.hash(state);
    }
}

impl<T: StableElement + Ord> Ord for StableHashSet<T> {
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        self.entries.cmp(&other.entries)
    }
}

impl<T: StableElement + Ord> PartialOrd for StableHashSet<T> {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl<T: StableElement + Ord + Clone> Clone for StableHashSet<T> {
    /// Copies the set, or panics when the allocation fails.
    ///
    /// # Panics
    ///
    /// Panics when the copy of a value, or the allocation, fails; use
    /// [`StableHashSet::try_clone`] for a fallible version.
    fn clone(&self) -> Self {
        Self {
            entries: self.entries.clone(),
        }
    }
}

impl<T: StableElement + Ord + fmt::Debug> fmt::Debug for StableHashSet<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StableHashSet")
            .field("len", &self.entries.len())
            .finish_non_exhaustive()
    }
}

// SAFETY: the set owns one map, and dropping it releases exactly the values that
// map holds.
unsafe impl<T: StableElement + Ord> StableElement for StableHashSet<T> {
    unsafe fn drop_in_place(ptr: *mut Self) {
        // SAFETY: the caller guarantees an initialised value.
        unsafe { core::ptr::drop_in_place(ptr) }
    }
}

impl<T: StableElement + Ord + crate::zeroize::Wipe> crate::zeroize::Wipe for StableHashSet<T> {
    fn wipe_in_place(&mut self) {
        self.entries.wipe_in_place();
    }
}

impl<K: StableElement + Ord, V: StableElement> StableHashMap<K, V> {
    /// The entries, for writing.
    ///
    /// A caller that changes the key or the number is responsible for the order:
    /// the map stays searchable only while the entries remain sorted by
    /// `(hash, key)`. [`StableHashMap::sort_entries`] restores it.
    pub fn as_entries_mut(&mut self) -> &mut [StableHashEntry<K, V>] {
        self.entries.as_mut_slice()
    }

    /// The values, for writing. The keys are untouched, so the map stays ordered.
    pub fn values_mut(&mut self) -> impl Iterator<Item = &mut V> {
        self.entries
            .as_mut_slice()
            .iter_mut()
            .map(|entry| &mut entry.value)
    }

    /// The entries, for writing, as the keys and values they hold.
    ///
    /// The value may be changed here; the key may not, because the number each
    /// entry is stored under is a function of the key and this form keeps them
    /// together. A caller that has to change a key takes the entries through
    /// [`StableHashMap::as_entries_mut`], recomputes the numbers and calls
    /// [`StableHashMap::sort_entries`].
    pub fn iter_mut(&mut self) -> impl Iterator<Item = (&K, &mut V)> {
        self.entries
            .as_mut_slice()
            .iter_mut()
            .map(|entry| (&entry.key, &mut entry.value))
    }

    /// Puts the entries back in the order the search expects.
    ///
    /// The map is ordered by `(hash, key)`; a caller that changed either through
    /// [`StableHashMap::as_entries_mut`] calls this to restore the guarantee. Two
    /// entries that hold one key under one number are the same entry listed
    /// twice, which the order alone cannot fix, so they are reported instead.
    pub fn sort_entries(&mut self) -> AbiResult<()> {
        self.entries.try_sort_by(|left, right| {
            left.hash
                .cmp(&right.hash)
                .then_with(|| left.key.cmp(&right.key))
        })?;
        let entries = self.entries.as_slice();
        let mut index = 1;
        while index < entries.len() {
            let previous = &entries[index - 1];
            let current = &entries[index];
            if previous.hash == current.hash && previous.key == current.key {
                return Err(AbiError::new(AbiStatus::InvalidDescriptor));
            }
            index += 1;
        }
        Ok(())
    }

    /// Gives back the spare capacity of the block.
    pub fn shrink_to_fit(&mut self) {
        self.entries.shrink_to_fit();
    }

    /// Gives back the spare capacity of the block, reporting failure.
    ///
    /// The block is replaced, so the entries move once; a map that is about to
    /// keep its size is the one to call this on.
    pub fn try_shrink_to_fit(&mut self) -> AbiResult<()> {
        self.entries.try_shrink_to_fit()
    }

    /// Stores every entry the iterator yields, keeping the last value written for
    /// a repeated key.
    ///
    /// The iterator is read to the end before the map changes, so a failure to
    /// allocate leaves this value as it was. The incoming entries are ordered and
    /// deduplicated in their own block and then merged with the entries the map
    /// already holds, which is linear; inserting them one at a time would shift
    /// the block once per entry, which is quadratic in the number of entries.
    /// Filling a map in one step is [`StableHashMap::from_entries`].
    pub fn try_extend<I: IntoIterator<Item = (K, V)>, F: Fn(&K) -> u64>(
        &mut self,
        hash: F,
        entries: I,
    ) -> AbiResult<()> {
        let allocator = self.entries.allocator();
        let mut incoming = StableVec::with_capacity_and_allocator(allocator, 0)?;
        for (key, value) in entries {
            incoming.try_push(StableHashEntry::new(hash(&key), key, value))?;
        }
        if incoming.is_empty() {
            return Ok(());
        }
        // The last write for a key is the one an insert loop would leave behind,
        // so it has to come first in its run: reversing before a stable sort does
        // that, and the deduplication keeps the first of the run.
        incoming.as_mut_slice().reverse();
        incoming.try_sort_by(|left, right| {
            left.hash
                .cmp(&right.hash)
                .then_with(|| left.key.cmp(&right.key))
        })?;
        incoming
            .dedup_by(|kept, candidate| kept.hash == candidate.hash && kept.key == candidate.key);

        // Both sides are ordered by the same pair, so one walk merges them. The
        // comparison is the caller's `Ord`, which may panic; it runs before any
        // entry moves, so a panic leaves the map holding what it held.
        crate::vec::try_merge_sorted(&mut self.entries, &mut incoming, false, |left, right| {
            left.hash
                .cmp(&right.hash)
                .then_with(|| left.key.cmp(&right.key))
        })
    }
}

impl<T: StableElement + Ord> StableHashSet<T> {
    /// Gives back the spare capacity of the block.
    pub fn shrink_to_fit(&mut self) {
        self.entries.shrink_to_fit();
    }

    /// Gives back the spare capacity of the block, reporting failure.
    pub fn try_shrink_to_fit(&mut self) -> AbiResult<()> {
        self.entries.try_shrink_to_fit()
    }

    /// The entries, for writing.
    ///
    /// A caller that changes a value or the number is responsible for the order,
    /// as with [`StableHashMap::as_entries_mut`]; [`StableHashSet::sort_entries`]
    /// restores it.
    pub fn as_entries_mut(&mut self) -> &mut [StableHashEntry<T, ()>] {
        self.entries.as_entries_mut()
    }

    /// Puts the entries back in the order the search expects.
    ///
    /// The set is ordered by `(hash, value)`, so a caller that changed either
    /// through [`StableHashSet::as_entries_mut`] calls this. Two entries that
    /// hold one value under one number are reported rather than left in, because
    /// they name the same entry twice.
    pub fn sort_entries(&mut self) -> AbiResult<()> {
        self.entries.sort_entries()
    }

    /// The value stored first, in the order the set keeps its entries.
    ///
    /// The order is by `(hash, value)`, which is the search order rather than
    /// the order `Ord` gives: this is not the smallest value by its own
    /// comparison.
    pub fn first(&self) -> Option<&T> {
        self.entries.as_entries().first().map(|entry| &entry.key)
    }

    /// The value stored last, in the order the set keeps its entries.
    ///
    /// The order is by `(hash, value)`; this is not the largest value by its
    /// own comparison.
    pub fn last(&self) -> Option<&T> {
        self.entries.as_entries().last().map(|entry| &entry.key)
    }

    /// Adds every value the iterator yields, keeping one of a repeated value.
    ///
    /// The values are ordered and deduplicated in their own block and then
    /// merged with the ones the set already holds, so the call is linear in the
    /// number of values; a set built in one step is
    /// [`StableHashSet::from_values`]. The iterator is read to the end before the
    /// set changes, so a failure leaves it as it was.
    pub fn try_extend<I: IntoIterator<Item = T>, F: Fn(&T) -> u64>(
        &mut self,
        hash: F,
        values: I,
    ) -> AbiResult<()> {
        self.entries
            .try_extend(hash, values.into_iter().map(|value| (value, ())))
    }
}

/// The methods that number a key with this crate's own hash.
///
/// A map built through them can be checked by the receiver on its own, because
/// both sides compute the same number for the same key; a map built through a
/// function of the caller's own needs that function on both sides, which is what
/// [`StableHashMap::validate_with`] asks for.
impl<K: StableElement + Ord + crate::stable_hash::StableHash, V: StableElement>
    StableHashMap<K, V>
{
    /// The number this crate gives `key`.
    ///
    /// It is what every method below passes to the containers, and it is the
    /// number [`StableHashMap::validate_stable`] recomputes.
    pub fn stable_hash(key: &K) -> u64 {
        crate::stable_hash::StableHash::stable_hash(key)
    }

    /// Stores `value` under `key`.
    pub fn insert_stable(&mut self, key: K, value: V) -> AbiResult<Option<V>> {
        self.insert(Self::stable_hash, key, value)
    }

    /// The value stored for `key`.
    pub fn get_stable(&self, key: &K) -> Option<&V> {
        self.get(Self::stable_hash, key)
    }

    /// Removes `key`, answering with its value.
    pub fn remove_stable(&mut self, key: &K) -> Option<V> {
        self.remove(Self::stable_hash, key)
    }

    /// Checks the entries against this crate's hash as well as against the
    /// layout.
    ///
    /// This is the check that needs nothing from the caller: a map the other side
    /// built with [`StableHashMap::insert_stable`] answers with numbers this
    /// build computes the same way, and a map built with a function of its own
    /// fails here unless that function is the one this crate ships.
    pub fn validate_stable(&self) -> AbiResult<()>
    where
        K: crate::boundary::AbiType,
        V: crate::boundary::AbiType,
    {
        self.validate_with(Self::stable_hash)
    }
}

impl<T: StableElement + Ord + crate::stable_hash::StableHash> StableHashSet<T> {
    /// Adds `value`, numbering it with this crate's hash.
    pub fn insert_stable(&mut self, value: T) -> AbiResult<bool> {
        self.insert(StableHashMap::<T, ()>::stable_hash, value)
    }

    /// Reports whether `value` is stored.
    pub fn contains_stable(&self, value: &T) -> bool {
        self.contains(StableHashMap::<T, ()>::stable_hash, value)
    }

    /// Removes `value`, answering whether it was there.
    pub fn remove_stable(&mut self, value: &T) -> bool {
        self.remove(StableHashMap::<T, ()>::stable_hash, value)
    }

    /// Checks the values against this crate's hash as well as against the
    /// layout.
    pub fn validate_stable(&self) -> AbiResult<()>
    where
        T: crate::boundary::AbiType,
    {
        self.entries.validate_stable()
    }
}

impl<K: StableElement, V: StableElement> core::iter::IntoIterator for StableHashMap<K, V> {
    type Item = (K, V);
    type IntoIter = core::iter::Map<
        crate::vec::StableIntoIter<StableHashEntry<K, V>>,
        fn(StableHashEntry<K, V>) -> (K, V),
    >;

    fn into_iter(self) -> Self::IntoIter {
        self.entries.into_iter().map(owned_entry::<K, V>)
    }
}

/// The key and value of an owned entry.
fn owned_entry<K, V>(entry: StableHashEntry<K, V>) -> (K, V) {
    (entry.key, entry.value)
}

/// The key and value of a borrowed entry.
fn entry_of<K, V>(entry: &StableHashEntry<K, V>) -> (&K, &V) {
    (&entry.key, &entry.value)
}

impl<'a, K: StableElement + Ord, V: StableElement> core::iter::IntoIterator
    for &'a StableHashMap<K, V>
{
    type Item = (&'a K, &'a V);
    type IntoIter = core::iter::Map<
        core::slice::Iter<'a, StableHashEntry<K, V>>,
        fn(&StableHashEntry<K, V>) -> (&K, &V),
    >;

    fn into_iter(self) -> Self::IntoIter {
        self.as_entries().iter().map(entry_of::<K, V>)
    }
}

impl<'a, K: StableElement, V: StableElement> core::iter::IntoIterator
    for &'a mut StableHashMap<K, V>
{
    type Item = (&'a K, &'a mut V);
    type IntoIter = core::iter::Map<
        core::slice::IterMut<'a, StableHashEntry<K, V>>,
        fn(&mut StableHashEntry<K, V>) -> (&K, &mut V),
    >;

    fn into_iter(self) -> Self::IntoIter {
        fn borrow<K, V>(entry: &mut StableHashEntry<K, V>) -> (&K, &mut V) {
            (&entry.key, &mut entry.value)
        }
        self.entries.as_mut_slice().iter_mut().map(borrow::<K, V>)
    }
}

impl<T: StableElement + Ord> core::iter::IntoIterator for StableHashSet<T> {
    type Item = T;
    type IntoIter = core::iter::Map<
        crate::vec::StableIntoIter<StableHashEntry<T, ()>>,
        fn(StableHashEntry<T, ()>) -> T,
    >;

    fn into_iter(self) -> Self::IntoIter {
        fn value<T>(entry: StableHashEntry<T, ()>) -> T {
            entry.key
        }
        self.entries.entries.into_iter().map(value::<T>)
    }
}

impl<'a, T: StableElement + Ord> core::iter::IntoIterator for &'a StableHashSet<T> {
    type Item = &'a T;
    type IntoIter = core::iter::Map<
        core::slice::Iter<'a, StableHashEntry<T, ()>>,
        fn(&StableHashEntry<T, ()>) -> &T,
    >;

    fn into_iter(self) -> Self::IntoIter {
        fn value<T>(entry: &StableHashEntry<T, ()>) -> &T {
            &entry.key
        }
        self.as_entries().iter().map(value::<T>)
    }
}

impl<'a, T: StableElement + Ord> core::iter::IntoIterator for &'a mut StableHashSet<T> {
    type Item = &'a mut T;
    type IntoIter = core::iter::Map<
        core::slice::IterMut<'a, StableHashEntry<T, ()>>,
        fn(&mut StableHashEntry<T, ()>) -> &mut T,
    >;

    /// Borrows every value for writing.
    ///
    /// A caller that changes a value is responsible for the order, as with
    /// [`StableHashMap::as_entries_mut`].
    fn into_iter(self) -> Self::IntoIter {
        fn value<T>(entry: &mut StableHashEntry<T, ()>) -> &mut T {
            &mut entry.key
        }
        self.as_entries_mut().iter_mut().map(value::<T>)
    }
}
