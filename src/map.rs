//! A map kept as a sorted sequence of pairs.
//!
//! The representation is ordered, so the bytes of two maps that hold the same
//! entries are the same and iteration order does not depend on a hash seed.

use core::fmt;

use crate::alloc::AbiAllocator;
use crate::error::{AbiError, AbiResult, AbiStatus};
use crate::tuple::StablePair;
use crate::vec::{StableElement, StableVec};

/// A map from keys to values, ordered by key.
///
/// A single insert or removal moves every entry after the key, so filling a map
/// one entry at a time costs O(n) per entry. Building a map from many pairs
/// belongs to [`StableMap::from_pairs`], which sorts and deduplicates in one
/// pass instead.
#[repr(C)]
pub struct StableMap<K: StableElement, V: StableElement> {
    entries: StableVec<StablePair<K, V>>,
}

impl<K: StableElement, V: StableElement> StableMap<K, V> {
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

    /// The number of entries the storage can hold without reallocating.
    pub const fn capacity(&self) -> u64 {
        self.entries.capacity()
    }

    /// Reports whether the map holds no entries.
    pub const fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The entries as sorted pairs.
    pub fn as_entries(&self) -> &[StablePair<K, V>] {
        self.entries.as_slice()
    }

    /// Iterates over the entries in key order.
    pub fn iter(&self) -> impl Iterator<Item = (&K, &V)> {
        self.entries.iter().map(|pair| (&pair.left, &pair.right))
    }

    /// The keys in order.
    pub fn keys(&self) -> impl Iterator<Item = &K> {
        self.entries.iter().map(|pair| &pair.left)
    }

    /// The values in key order.
    pub fn values(&self) -> impl Iterator<Item = &V> {
        self.entries.iter().map(|pair| &pair.right)
    }

    /// Keeps only the entries for which `keep` returns true.
    ///
    /// This is the operation behind several of the methods below; it is public
    /// so a caller can filter without rebuilding the map.
    pub fn retain<F: FnMut(&K, &V) -> bool>(&mut self, mut keep: F) {
        self.entries.retain(|pair| keep(&pair.left, &pair.right));
    }

    /// Removes every entry.
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Makes room for `additional` more entries.
    pub fn try_reserve(&mut self, additional: u64) -> AbiResult<()> {
        self.entries.try_reserve(additional)
    }

    /// Gives back the room the map is not using.
    ///
    /// The block is replaced, so the entries move once; a map that is about to
    /// keep its size is the one to call this on.
    pub fn try_shrink_to_fit(&mut self) -> AbiResult<()> {
        self.entries.try_shrink_to_fit()
    }
}

impl<K: StableElement + Ord, V: StableElement> StableMap<K, V> {
    /// The index of `key`, or the position where it would be inserted.
    fn find(&self, key: &K) -> core::result::Result<usize, usize> {
        self.as_entries().binary_search_by(|pair| pair.left.cmp(key))
    }

    /// The value stored for `key`.
    pub fn get(&self, key: &K) -> Option<&V> {
        match self.find(key) {
            Ok(index) => Some(&self.as_entries()[index].right),
            Err(_) => None,
        }
    }

    /// The value stored for `key`, mutably.
    pub fn get_mut(&mut self, key: &K) -> Option<&mut V> {
        match self.find(key) {
            Ok(index) => Some(&mut self.entries.as_mut_slice()[index].right),
            Err(_) => None,
        }
    }

    /// Reports whether `key` has an entry.
    pub fn contains_key(&self, key: &K) -> bool {
        self.find(key).is_ok()
    }

    /// Stores `value` for `key`, returning the value it replaced.
    pub fn insert(&mut self, key: K, value: V) -> AbiResult<Option<V>> {
        match self.find(&key) {
            Ok(index) => {
                let slot = &mut self.entries.as_mut_slice()[index];
                let previous = core::mem::replace(&mut slot.right, value);
                Ok(Some(previous))
            }
            Err(index) => {
                self.entries.insert(index as u64, StablePair::new(key, value))?;
                Ok(None)
            }
        }
    }

    /// Removes the entry for `key`.
    pub fn remove(&mut self, key: &K) -> Option<V> {
        match self.find(key) {
            Ok(index) => self.entries.remove(index as u64).map(|pair| pair.right),
            Err(_) => None,
        }
    }

    /// The key and value stored for `key`.
    ///
    /// The key is the one the map holds, which is what a caller that looked the
    /// entry up with a borrowed key needs in order to keep it.
    pub fn get_key_value(&self, key: &K) -> Option<(&K, &V)> {
        match self.find(key) {
            Ok(index) => {
                let pair = &self.as_entries()[index];
                Some((&pair.left, &pair.right))
            }
            Err(_) => None,
        }
    }

    /// Removes the entry for `key`, answering with the key and the value.
    pub fn remove_entry(&mut self, key: &K) -> Option<(K, V)> {
        match self.find(key) {
            Ok(index) => self.entries.remove(index as u64).map(StablePair::into_tuple),
            Err(_) => None,
        }
    }

    /// Removes the entry with the smallest key and hands it back.
    pub fn pop_first(&mut self) -> Option<(K, V)> {
        if self.entries.is_empty() {
            return None;
        }
        self.entries.remove(0).map(StablePair::into_tuple)
    }

    /// Removes the entry with the largest key and hands it back.
    pub fn pop_last(&mut self) -> Option<(K, V)> {
        let length = self.entries.len();
        if length == 0 {
            return None;
        }
        self.entries
            .remove(length - 1)
            .map(StablePair::into_tuple)
    }

    /// Removes every entry and yields them in key order.
    ///
    /// The map stops covering the entries at once, so an entry the iterator has
    /// not reached is released when the iterator is dropped and never twice.
    pub fn drain(&mut self) -> impl Iterator<Item = (K, V)> + '_ {
        self.entries.drain().map(owned_pair::<K, V>)
    }

    /// Takes the map apart, yielding its keys in order and releasing the values.
    pub fn into_keys(self) -> impl Iterator<Item = K> {
        self.entries.into_iter().map(|pair| pair.left)
    }

    /// Takes the map apart, yielding its values in key order and releasing the
    /// keys.
    pub fn into_values(self) -> impl Iterator<Item = V> {
        self.entries.into_iter().map(|pair| pair.right)
    }

    /// Builds a map from pairs, keeping the last value written for a repeated
    /// key.
    pub fn from_pairs(pairs: &[(K, V)]) -> AbiResult<Self>
    where
        K: Clone,
        V: Clone,
    {
        Self::from_pairs_with_allocator(AbiAllocator::system(), pairs)
    }

    /// Builds a map from pairs that uses `allocator`.
    pub fn from_pairs_with_allocator(
        allocator: AbiAllocator,
        pairs: &[(K, V)],
    ) -> AbiResult<Self>
    where
        K: Clone,
        V: Clone,
    {
        let mut entries = StableVec::with_capacity_and_allocator(allocator, pairs.len() as u64)?;
        for (key, value) in pairs {
            entries.try_push(StablePair::new(key.clone(), value.clone()))?;
        }
        // A stable sort keeps the input order among equal keys, so the *first*
        // write for a key would come first. Reversing before the sort puts the
        // last write first in its run, and the deduplication below keeps the
        // first of every run: that is the value an insert of these pairs one
        // after another would leave behind.
        entries.as_mut_slice().reverse();
        entries.try_sort_by(|left, right| left.left.cmp(&right.left))?;
        // One pass, not one `remove` per duplicate: a shift per duplicate would
        // make building the map quadratic in the number of repeated keys.
        entries
            .dedup_by(|kept, candidate| kept.left == candidate.left);
        Ok(Self { entries })
    }

    /// Wraps entries that are already in key order.
    ///
    /// A copy of a map is already ordered, so it is built by moving its entries
    /// into a block rather than by searching for each one.
    pub(crate) fn from_sorted_entries(entries: StableVec<StablePair<K, V>>) -> Self {
        Self { entries }
    }

    /// Borrows the entry for `key`, which the caller may fill in.
    pub fn entry(&mut self, key: K) -> Entry<'_, K, V> {
        match self.find(&key) {
            Ok(index) => Entry {
                map: self,
                key: None,
                index,
            },
            Err(index) => Entry {
                map: self,
                key: Some(key),
                index,
            },
        }
    }

    /// Checks that the entries are sorted by key and that every key and value is
    /// a value its type defines.
    pub fn validate(&self) -> AbiResult<()>
    where
        K: crate::boundary::AbiType,
        V: crate::boundary::AbiType,
    {
        self.entries.validate_header()?;
        // A pair of zero-sized keys and values carries nothing but the length,
        // so no byte count, alignment or address can say whether that number is
        // true; the same rule the vector applies to its own elements.
        if core::mem::size_of::<crate::tuple::StablePair<K, V>>() == 0 && !self.is_empty() {
            return Err(AbiError::new(AbiStatus::InvalidDescriptor));
        }
        let mut previous: Option<&K> = None;
        for pair in self.as_entries() {
            crate::boundary::check(&pair.left)?;
            crate::boundary::check(&pair.right)?;
            // A key that is not greater than the one before it breaks the order
            // a lookup relies on, so the whole map is refused.
            if let Some(previous) = previous
                && previous >= &pair.left
            {
                return Err(AbiError::new(AbiStatus::InvalidDescriptor));
            }
            previous = Some(&pair.left);
        }
        Ok(())
    }
}

impl<K: StableElement, V: StableElement> StableMap<K, V> {
    /// Adopts the storage of a map that arrived from another build.
    ///
    /// The pairs must be sorted by key, which is the invariant every map
    /// maintains; the length and capacity describe the block the pairs sit in.
    ///
    /// # Safety
    ///
    /// `pointer` must be a live block from `allocator` with at least `capacity`
    /// slots, `length` of which hold initialised, sorted pairs.
    pub unsafe fn from_raw_parts(
        pointer: *mut StablePair<K, V>,
        length: u64,
        capacity: u64,
        allocator: AbiAllocator,
    ) -> AbiResult<Self> {
        // SAFETY: the caller guarantees the block and its contents.
        let entries = unsafe { StableVec::from_raw_parts(pointer, length, capacity, allocator)? };
        Ok(Self { entries })
    }

    /// Takes the map apart without releasing its storage.
    ///
    /// # Safety
    ///
    /// The returned block must be released exactly once, through the returned
    /// allocator, after its pairs are dropped.
    pub unsafe fn into_raw_parts(self) -> (*mut StablePair<K, V>, u64, u64, AbiAllocator) {
        // SAFETY: the caller takes over the obligation carried by the parts.
        unsafe { self.entries.into_raw_parts() }
    }
}

impl<K: StableElement, V: StableElement> Default for StableMap<K, V> {
    fn default() -> Self {
        Self::new()
    }
}

impl<K: StableElement, V: StableElement> StableMap<K, V> {
    /// An empty map whose storage can hold `capacity` entries without
    /// reallocating, using `allocator`.
    pub fn with_capacity_and_allocator(
        allocator: AbiAllocator,
        capacity: u64,
    ) -> AbiResult<Self> {
        Ok(Self {
            entries: StableVec::with_capacity_and_allocator(allocator, capacity)?,
        })
    }

    /// An empty map whose storage can hold `capacity` entries without
    /// reallocating, using the process allocator.
    pub fn with_capacity(capacity: u64) -> AbiResult<Self> {
        Self::with_capacity_and_allocator(AbiAllocator::system(), capacity)
    }
}

/// A map entry that may already hold a value.
///
/// Built by [`StableMap::entry`]; the lookup is done once, so filling a vacant
/// entry does not search the map a second time.
pub struct Entry<'a, K: StableElement, V: StableElement> {
    map: &'a mut StableMap<K, V>,
    /// The key to insert, or `None` when the entry already holds a value.
    key: Option<K>,
    /// Where the entry is, or where it would be inserted.
    index: usize,
}

impl<'a, K: StableElement, V: StableElement> Entry<'a, K, V> {
    /// The key this entry is about.
    pub fn key(&self) -> &K {
        match &self.key {
            Some(key) => key,
            None => &self.map.as_entries()[self.index].left,
        }
    }

    /// Reports whether the map already holds a value for the key.
    pub fn is_occupied(&self) -> bool {
        self.key.is_none()
    }
}

impl<'a, K: StableElement + Ord, V: StableElement> Entry<'a, K, V> {
    /// Returns the value, filling the entry with `value` when it was vacant.
    pub fn or_insert(self, value: V) -> AbiResult<&'a mut V> {
        self.or_insert_with(|| value)
    }

    /// Returns the value, filling the entry with `make()` when it was vacant.
    pub fn or_insert_with<F: FnOnce() -> V>(self, make: F) -> AbiResult<&'a mut V> {
        let this = self;
        // An occupied entry keeps the value it already holds: `or_insert` fills
        // a place, it does not replace what is there.
        if let Some(key) = this.key {
            this.map
                .entries
                .insert(this.index as u64, StablePair::new(key, make()))?;
        }
        Ok(&mut this.map.entries.as_mut_slice()[this.index].right)
    }

    /// Applies `update` to the value, filling the entry first from `make` when
    /// it was vacant.
    ///
    /// This is the shape a counter or an accumulator needs: one lookup, then one
    /// change, whether or not the key was there.
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

impl<K: StableElement + fmt::Debug, V: StableElement> fmt::Debug for Entry<'_, K, V> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Entry")
            .field("occupied", &self.is_occupied())
            .finish()
    }
}

impl<K: StableElement + crate::zeroize::Wipe, V: StableElement + crate::zeroize::Wipe>
    crate::zeroize::Wipe for StableMap<K, V>
{
    fn wipe_in_place(&mut self) {
        self.entries.wipe_in_place();
    }
}

impl<K: StableElement + PartialEq, V: StableElement + PartialEq> PartialEq for StableMap<K, V> {
    fn eq(&self, other: &Self) -> bool {
        self.as_entries() == other.as_entries()
    }
}

impl<K: StableElement + Eq, V: StableElement + Eq> Eq for StableMap<K, V> {}

impl<K: StableElement + core::hash::Hash, V: StableElement + core::hash::Hash> core::hash::Hash
    for StableMap<K, V>
{
    /// Folds the entries, in the order the map stores them.
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        self.as_entries().hash(state);
    }
}

impl<K: StableElement + Ord, V: StableElement + Ord> Ord for StableMap<K, V> {
    /// Orders two maps entry by entry, then by length.
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        let left = self.as_entries();
        let right = other.as_entries();
        let shared = left.len().min(right.len());
        let mut index = 0;
        while index < shared {
            let ordering = left[index]
                .left
                .cmp(&right[index].left)
                .then_with(|| left[index].right.cmp(&right[index].right));
            if ordering != core::cmp::Ordering::Equal {
                return ordering;
            }
            index += 1;
        }
        left.len().cmp(&right.len())
    }
}

impl<K: StableElement + Ord, V: StableElement + Ord> PartialOrd for StableMap<K, V> {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl<K: StableElement + Ord, V: StableElement> core::iter::Extend<(K, V)> for StableMap<K, V> {
    /// Inserts every entry, panicking when the storage fails.
    ///
    /// The work is one merge rather than one insertion per entry: use
    /// [`StableMap::try_extend`] where a failure has to be reported instead.
    fn extend<I: IntoIterator<Item = (K, V)>>(&mut self, entries: I) {
        if let Err(error) = self.try_extend(entries) {
            panic!("failed to extend a stable map: {error}");
        }
    }
}

impl<K: StableElement + Ord, V: StableElement> core::iter::FromIterator<(K, V)>
    for StableMap<K, V>
{
    /// Builds a map from entries, panicking when an insertion fails.
    fn from_iter<I: IntoIterator<Item = (K, V)>>(entries: I) -> Self {
        let mut map = Self::new();
        map.extend(entries);
        map
    }
}

impl<K: StableElement, V: StableElement> fmt::Debug for StableMap<K, V> {
    // The entries can be content a log must not repeat, so the debug form
    // reports the shape. `iter` visits the entries when a caller asks for them.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StableMap")
            .field("len", &self.len())
            .finish()
    }
}

/// A map from keys to values, ordered by key.
impl<K: StableElement + Clone, V: StableElement + Clone> Clone for StableMap<K, V> {
    /// Copies the entries through [`Clone`], which raises a copy or an
    /// allocation that fails.
    fn clone(&self) -> Self {
        Self {
            entries: self.entries.clone(),
        }
    }
}

// SAFETY: the map is a `#[repr(C)]` struct holding one vector, and its drop
// releases every key and value through the vector's contract.
unsafe impl<K: StableElement, V: StableElement> StableElement for StableMap<K, V> {
    unsafe fn drop_in_place(ptr: *mut Self) {
        // SAFETY: the caller guarantees an initialised value.
        unsafe { core::ptr::drop_in_place(ptr) }
    }
}

/// The entries of a borrowed map, as the pairs it stores.
fn pair_of<K, V>(entry: &StablePair<K, V>) -> (&K, &V) {
    (&entry.left, &entry.right)
}

/// The entries of an owned map.
fn owned_pair<K, V>(entry: StablePair<K, V>) -> (K, V) {
    (entry.left, entry.right)
}

impl<K: StableElement, V: StableElement> StableMap<K, V> {
    /// The entries, for writing.
    ///
    /// A caller that changes a key is responsible for the order: the map stays
    /// sorted only while every key keeps its place. [`StableMap::sort_entries`]
    /// restores it.
    pub fn as_entries_mut(&mut self) -> &mut [StablePair<K, V>] {
        self.entries.as_mut_slice()
    }

    /// The values, for writing.
    ///
    /// The keys are untouched, so the map stays ordered.
    pub fn values_mut(&mut self) -> impl Iterator<Item = &mut V> {
        self.entries.as_mut_slice().iter_mut().map(|entry| &mut entry.right)
    }

    /// The entries, for writing, as the keys and values they hold.
    ///
    /// Only the values are reachable for writing: the keys are borrowed, so the
    /// order this form hands out cannot change, and the map stays sorted. A
    /// caller that has to change a key takes the entries through
    /// [`StableMap::as_entries_mut`] and then calls [`StableMap::sort_entries`].
    pub fn iter_mut(&mut self) -> impl Iterator<Item = (&K, &mut V)> {
        self.entries
            .as_mut_slice()
            .iter_mut()
            .map(|entry| (&entry.left, &mut entry.right))
    }

    /// The entry with the smallest key, as the key and the value stored for it.
    pub fn first_key_value(&self) -> Option<(&K, &V)> {
        self.entries.as_slice().first().map(pair_of)
    }

    /// The entry with the largest key.
    pub fn last_key_value(&self) -> Option<(&K, &V)> {
        self.entries.as_slice().last().map(pair_of)
    }
}

impl<K: StableElement + Ord, V: StableElement> StableMap<K, V> {
    /// Inserts every entry the iterator yields, keeping the last value written
    /// for a repeated key.
    ///
    /// The iterator is read to the end before the map changes, so a failure to
    /// allocate leaves this value as it was. The incoming entries are ordered and
    /// deduplicated in their own block and then merged with the entries the map
    /// already holds, which is linear; inserting them one at a time would shift
    /// the block once per entry, which is quadratic in the number of entries.
    /// Filling a map in one step is [`StableMap::from_pairs`].
    pub fn try_extend<I: IntoIterator<Item = (K, V)>>(&mut self, entries: I) -> AbiResult<()> {
        let allocator = self.entries.allocator();
        let mut incoming = StableVec::with_capacity_and_allocator(allocator, 0)?;
        for (key, value) in entries {
            incoming.try_push(StablePair::new(key, value))?;
        }
        if incoming.is_empty() {
            return Ok(());
        }
        // The last write for a key is the one an insert loop would leave behind,
        // so it has to come first in its run: reversing before a stable sort does
        // that, and the deduplication keeps the first of the run.
        incoming.as_mut_slice().reverse();
        incoming.try_sort_by(|left, right| left.left.cmp(&right.left))?;
        incoming.dedup_by(|kept, candidate| kept.left == candidate.left);

        // Both sides are ordered by key, so one walk merges them. The
        // comparison is the caller's `Ord`, which may panic; it runs before any
        // entry moves, so a panic leaves the map holding what it held.
        crate::vec::try_merge_sorted(&mut self.entries, &mut incoming, false, |left, right| {
            left.left.cmp(&right.left)
        })
    }

    /// Puts the entries back in key order.
    ///
    /// The map is ordered by key, so a caller that changed a key through
    /// [`StableMap::as_entries_mut`] or [`StableMap::iter_mut`] calls this to
    /// restore the guarantee. An entry that repeats a key means the map no longer
    /// describes a function, and the order alone cannot fix that.
    pub fn sort_entries(&mut self) -> AbiResult<()>
    where
        K: Ord,
    {
        self.entries
            .try_sort_by(|left, right| left.left.cmp(&right.left))?;
        let entries = self.entries.as_slice();
        let mut index = 1;
        while index < entries.len() {
            if entries[index - 1].left == entries[index].left {
                return Err(AbiError::new(AbiStatus::InvalidDescriptor));
            }
            index += 1;
        }
        Ok(())
    }

    /// The entries whose keys fall in `range`, in key order.
    ///
    /// # Panics
    ///
    /// Panics when the start of the range is past its end, which is a range that
    /// names nothing: the ordered collections of the standard library refuse it
    /// the same way, and answering with no entries would hide the mistake.
    ///
    /// Both ends excluded around one key is that same shape: every entry would
    /// have to be greater than the key and smaller than it. The standard library
    /// refuses it too, and it is refused here whether or not the key has an
    /// entry, so the same range is never empty for one map and fatal for
    /// another.
    pub fn range<R: core::ops::RangeBounds<K>>(&self, range: R) -> impl Iterator<Item = (&K, &V)> {
        if let (core::ops::Bound::Excluded(start), core::ops::Bound::Excluded(end)) =
            (range.start_bound(), range.end_bound())
        {
            assert!(
                start != end,
                "the range excludes the same key at both ends"
            );
        }
        let entries = self.entries.as_slice();
        let start = match range.start_bound() {
            core::ops::Bound::Included(key) => {
                entries.partition_point(|entry| entry.left < *key)
            }
            core::ops::Bound::Excluded(key) => {
                entries.partition_point(|entry| entry.left <= *key)
            }
            core::ops::Bound::Unbounded => 0,
        };
        let end = match range.end_bound() {
            core::ops::Bound::Included(key) => {
                Some(entries.partition_point(|entry| entry.left <= *key))
            }
            core::ops::Bound::Excluded(key) => {
                Some(entries.partition_point(|entry| entry.left < *key))
            }
            core::ops::Bound::Unbounded => None,
        };
        let end = end.unwrap_or(entries.len());
        assert!(
            start <= end,
            "the range starts past the key its end names"
        );
        entries[start..end].iter().map(pair_of)
    }

    /// Splits the map at `key`: `self` keeps the entries before it and the answer
    /// holds the rest.
    ///
    /// The value for `key` itself goes to the answer, which is what
    /// `split_off` means for an ordered map.
    pub fn split_off(&mut self, key: &K) -> AbiResult<Self> {
        let at = self.entries.as_slice().partition_point(|entry| entry.left < *key);
        let tail = self.entries.try_split_off(at as u64)?;
        Ok(Self { entries: tail })
    }

    /// Moves every entry of `other` into `self`, leaving `other` empty.
    ///
    /// A key that both maps hold keeps the value from `other`, as an insert of
    /// each entry of `other` would leave it.
    ///
    /// Each map keeps the allocator it arrived with: the merged block belongs to
    /// `self`'s, and `other` stays empty but still allocates through its own.
    /// A failure to allocate, or a key comparison that panics, leaves both maps
    /// as they were: the comparison runs before any entry moves.
    pub fn append(&mut self, other: &mut Self) -> AbiResult<()> {
        crate::vec::try_merge_sorted(&mut self.entries, &mut other.entries, false, |left, right| {
            left.left.cmp(&right.left)
        })
    }
}

impl<K: StableElement, V: StableElement> core::iter::IntoIterator for StableMap<K, V> {
    type Item = (K, V);
    type IntoIter =
        core::iter::Map<crate::vec::StableIntoIter<StablePair<K, V>>, fn(StablePair<K, V>) -> (K, V)>;

    fn into_iter(self) -> Self::IntoIter {
        self.entries.into_iter().map(owned_pair::<K, V>)
    }
}

impl<'a, K: StableElement, V: StableElement> core::iter::IntoIterator for &'a StableMap<K, V> {
    type Item = (&'a K, &'a V);
    type IntoIter =
        core::iter::Map<core::slice::Iter<'a, StablePair<K, V>>, fn(&StablePair<K, V>) -> (&K, &V)>;

    fn into_iter(self) -> Self::IntoIter {
        self.as_entries().iter().map(pair_of::<K, V>)
    }
}

impl<'a, K: StableElement, V: StableElement> core::iter::IntoIterator for &'a mut StableMap<K, V> {
    type Item = (&'a K, &'a mut V);
    type IntoIter = core::iter::Map<
        core::slice::IterMut<'a, StablePair<K, V>>,
        fn(&mut StablePair<K, V>) -> (&K, &mut V),
    >;

    fn into_iter(self) -> Self::IntoIter {
        fn entry_of<K, V>(entry: &mut StablePair<K, V>) -> (&K, &mut V) {
            (&entry.left, &mut entry.right)
        }
        self.entries.as_mut_slice().iter_mut().map(entry_of::<K, V>)
    }
}
