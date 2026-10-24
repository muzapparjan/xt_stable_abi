//! A set kept as a sorted sequence.
//!
//! As with [`crate::map::StableMap`], ordering makes the bytes of equal sets
//! equal and iteration order independent of a hash seed.

use core::fmt;

use crate::alloc::AbiAllocator;
use crate::error::{AbiError, AbiResult, AbiStatus};
use crate::vec::{StableElement, StableVec};

/// A set of values in ascending order.
#[repr(C)]
pub struct StableSet<T: StableElement> {
    values: StableVec<T>,
}

impl<T: StableElement> StableSet<T> {
    /// An empty set that uses the process allocator.
    pub const fn new() -> Self {
        Self {
            values: StableVec::new(),
        }
    }

    /// An empty set that uses `allocator`.
    pub const fn new_with_allocator(allocator: AbiAllocator) -> Self {
        Self {
            values: StableVec::new_with_allocator(allocator),
        }
    }

    /// The allocator that owns the storage.
    pub const fn allocator(&self) -> AbiAllocator {
        self.values.allocator()
    }

    /// The number of values.
    pub const fn len(&self) -> u64 {
        self.values.len()
    }

    /// The number of values the storage can hold without reallocating.
    pub const fn capacity(&self) -> u64 {
        self.values.capacity()
    }

    /// Reports whether the set holds no values.
    pub const fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// The values in ascending order.
    pub fn as_values(&self) -> &[T] {
        self.values.as_slice()
    }

    /// Iterates over the values in ascending order.
    pub fn iter(&self) -> core::slice::Iter<'_, T> {
        self.values.iter()
    }

    /// Removes every value.
    pub fn clear(&mut self) {
        self.values.clear();
    }

    /// Makes room for `additional` more values.
    pub fn try_reserve(&mut self, additional: u64) -> AbiResult<()> {
        self.values.try_reserve(additional)
    }

    /// Gives back the room the set is not using.
    ///
    /// The block is replaced, so the values move once; a set that is about to
    /// keep its size is the one to call this on.
    pub fn try_shrink_to_fit(&mut self) -> AbiResult<()> {
        self.values.try_shrink_to_fit()
    }

    /// Keeps only the values for which `keep` returns true.
    pub fn retain<F: FnMut(&T) -> bool>(&mut self, mut keep: F) {
        self.values.retain(|value| keep(value));
    }

    /// An empty set whose storage can hold `capacity` values without
    /// reallocating, using `allocator`.
    pub fn with_capacity_and_allocator(
        allocator: AbiAllocator,
        capacity: u64,
    ) -> AbiResult<Self> {
        Ok(Self {
            values: StableVec::with_capacity_and_allocator(allocator, capacity)?,
        })
    }

    /// An empty set whose storage can hold `capacity` values without
    /// reallocating, using the process allocator.
    pub fn with_capacity(capacity: u64) -> AbiResult<Self> {
        Self::with_capacity_and_allocator(AbiAllocator::system(), capacity)
    }

    /// Checks that the values are in ascending order without repeats, and that
    /// every value is one its type defines.
    pub fn validate(&self) -> AbiResult<()>
    where
        T: crate::boundary::AbiType + Ord,
    {
        self.values.validate_header()?;
        // A value of no size carries nothing but the length, so no byte count,
        // alignment or address can say whether that number is true; the same
        // rule the vector applies to its own elements.
        if core::mem::size_of::<T>() == 0 && !self.is_empty() {
            return Err(AbiError::new(AbiStatus::InvalidDescriptor));
        }
        let mut previous: Option<&T> = None;
        for value in self.as_values() {
            crate::boundary::check(value)?;
            if let Some(previous) = previous
                && previous >= value
            {
                return Err(AbiError::new(AbiStatus::InvalidDescriptor));
            }
            previous = Some(value);
        }        Ok(())
    }
}

impl<T: StableElement + Ord> StableSet<T> {
    /// The index of `value`, or the position where it would be inserted.
    fn find(&self, value: &T) -> core::result::Result<usize, usize> {
        self.as_values().binary_search(value)
    }

    /// Reports whether `value` is in the set.
    pub fn contains(&self, value: &T) -> bool {
        self.find(value).is_ok()
    }

    /// Adds `value`, reporting whether it was absent.
    pub fn insert(&mut self, value: T) -> AbiResult<bool> {
        match self.find(&value) {
            Ok(_) => Ok(false),
            Err(index) => {
                self.values.insert(index as u64, value)?;
                Ok(true)
            }
        }
    }

    /// Removes `value`, reporting whether it was present.
    pub fn remove(&mut self, value: &T) -> bool {
        match self.find(value) {
            Ok(index) => {
                self.values.remove(index as u64);
                true
            }
            Err(_) => false,
        }
    }

    /// Removes the smallest value and hands it back.
    pub fn pop_first(&mut self) -> Option<T> {
        if self.values.is_empty() {
            return None;
        }
        self.values.remove(0)
    }

    /// Removes the largest value and hands it back.
    pub fn pop_last(&mut self) -> Option<T> {
        let length = self.values.len();
        if length == 0 {
            return None;
        }
        self.values.remove(length - 1)
    }

    /// Builds a set from values.
    pub fn from_values(values: &[T]) -> AbiResult<Self>
    where
        T: Clone,
    {
        Self::from_values_with_allocator(AbiAllocator::system(), values)
    }

    /// Builds a set from values that uses `allocator`.
    ///
    /// A value that appears more than once is kept once: the last occurrence is
    /// the one that stays, which is the rule a map applies when the same key is
    /// inserted twice. The vector is reversed before the sort because a stable
    /// sort keeps the first of a run of equal values.
    pub fn from_values_with_allocator(
        allocator: AbiAllocator,
        values: &[T],
    ) -> AbiResult<Self>
    where
        T: Clone,
    {
        let mut kept = StableVec::with_capacity_and_allocator(allocator, values.len() as u64)?;
        for value in values {
            kept.try_push(value.clone())?;
        }
        kept.reverse();
        kept.try_sort()?;
        // Equal values sit next to each other after the sort, so the duplicates
        // are dropped in one pass instead of one shift per duplicate.
        kept.dedup_by(|kept, candidate| kept == candidate);
        Ok(Self { values: kept })
    }

    /// Wraps values that are already in order.
    ///
    /// A copy of a set is already ordered, so it is built by moving its values
    /// into a block rather than by searching for each one.
    pub(crate) fn from_sorted_values(values: StableVec<T>) -> Self {
        Self { values }
    }

    /// The values in `self` or `other`, without duplicates.
    pub fn union(&self, other: &Self) -> AbiResult<Self>
    where
        T: Clone,
    {
        let one = self.values.as_slice();
        let two = other.values.as_slice();
        let mut kept =
            StableVec::with_capacity_and_allocator(self.values.allocator(), (one.len() + two.len()) as u64)?;
        let (mut left, mut right) = (0usize, 0usize);
        while left < one.len() || right < two.len() {
            let take_left = match (one.get(left), two.get(right)) {
                (Some(first), Some(second)) => first <= second,
                (Some(_), None) => true,
                _ => false,
            };
            let value = if take_left {
                let value = &one[left];
                left += 1;
                value
            } else {
                let value = &two[right];
                right += 1;
                value
            };
            // Equal neighbours are the same value reached from both sets.
            if kept.last() != Some(value) {
                kept.try_push(value.clone())?;
            }
        }
        Ok(Self { values: kept })
    }

    /// The values both sets hold.
    pub fn intersection(&self, other: &Self) -> AbiResult<Self>
    where
        T: Clone,
    {
        let one = self.values.as_slice();
        let two = other.values.as_slice();
        let mut kept =
            StableVec::with_capacity_and_allocator(self.values.allocator(), one.len() as u64)?;
        let (mut left, mut right) = (0usize, 0usize);
        while left < one.len() && right < two.len() {
            match one[left].cmp(&two[right]) {
                core::cmp::Ordering::Less => left += 1,
                core::cmp::Ordering::Greater => right += 1,
                core::cmp::Ordering::Equal => {
                    kept.try_push(one[left].clone())?;
                    left += 1;
                    right += 1;
                }
            }
        }
        Ok(Self { values: kept })
    }

    /// The values in `self` that `other` does not hold.
    pub fn difference(&self, other: &Self) -> AbiResult<Self>
    where
        T: Clone,
    {
        let one = self.values.as_slice();
        let two = other.values.as_slice();
        let mut kept =
            StableVec::with_capacity_and_allocator(self.values.allocator(), one.len() as u64)?;
        let mut right = 0usize;
        for value in one {
            while right < two.len() && two[right] < *value {
                right += 1;
            }
            let excluded = right < two.len() && two[right] == *value;
            if !excluded {
                kept.try_push(value.clone())?;
            }
        }
        Ok(Self { values: kept })
    }

    /// The values in either set but not in both.
    pub fn symmetric_difference(&self, other: &Self) -> AbiResult<Self>
    where
        T: Clone,
    {
        let one = self.values.as_slice();
        let two = other.values.as_slice();
        let mut kept = StableVec::with_capacity_and_allocator(
            self.values.allocator(),
            one.len() as u64 + two.len() as u64,
        )?;
        let mut left = 0usize;
        let mut right = 0usize;
        while left < one.len() && right < two.len() {
            match one[left].cmp(&two[right]) {
                core::cmp::Ordering::Less => {
                    kept.try_push(one[left].clone())?;
                    left += 1;
                }
                core::cmp::Ordering::Greater => {
                    kept.try_push(two[right].clone())?;
                    right += 1;
                }
                core::cmp::Ordering::Equal => {
                    left += 1;
                    right += 1;
                }
            }
        }
        while left < one.len() {
            kept.try_push(one[left].clone())?;
            left += 1;
        }
        while right < two.len() {
            kept.try_push(two[right].clone())?;
            right += 1;
        }
        Ok(Self { values: kept })
    }

    /// Reports whether every value of `self` is in `other`.
    pub fn is_subset(&self, other: &Self) -> bool {
        let one = self.values.as_slice();
        let two = other.values.as_slice();
        // Both sides are ordered, so one walk decides it.
        let mut right = 0usize;
        for value in one {
            while right < two.len() && two[right] < *value {
                right += 1;
            }
            if right == two.len() || two[right] != *value {
                return false;
            }
        }
        true
    }

    /// Reports whether every value of `other` is in `self`.
    pub fn is_superset(&self, other: &Self) -> bool {
        other.is_subset(self)
    }

    /// Reports whether the two sets hold no value in common.
    pub fn is_disjoint(&self, other: &Self) -> bool {
        let one = self.values.as_slice();
        let two = other.values.as_slice();
        let mut left = 0usize;
        let mut right = 0usize;
        while left < one.len() && right < two.len() {
            match one[left].cmp(&two[right]) {
                core::cmp::Ordering::Less => left += 1,
                core::cmp::Ordering::Greater => right += 1,
                core::cmp::Ordering::Equal => return false,
            }
        }
        true
    }

    /// Adds every value the iterator yields, keeping one of a repeated value.
    ///
    /// A failure leaves the set as it was: the values are collected and ordered
    /// before any of them reaches the set, and the block that holds the result is
    /// taken before the set is touched.
    pub fn try_extend<I: IntoIterator<Item = T>>(&mut self, items: I) -> AbiResult<()> {
        let allocator = self.values.allocator();
        let mut added = StableVec::new_with_allocator(allocator);
        for value in items {
            added.try_push(value)?;
        }
        if added.is_empty() {
            return Ok(());
        }
        added.try_sort()?;
        added.dedup_by(|left, right| left == right);
        // The comparison is the caller's `Ord`, which may panic; it runs before
        // any value moves, so a panic leaves the set holding what it held. A
        // value both sides hold keeps the one the set already had.
        crate::vec::try_merge_sorted(&mut self.values, &mut added, true, |left, right| {
            left.cmp(right)
        })
    }

    /// The smallest value.
    pub fn first(&self) -> Option<&T> {
        self.values.as_slice().first()
    }

    /// The largest value.
    pub fn last(&self) -> Option<&T> {
        self.values.as_slice().last()
    }

    /// The stored value equal to `value`.
    ///
    /// This is `contains` with the value handed back: a caller that holds a
    /// larger type or a borrowed form can look the set's own copy up.
    pub fn get(&self, value: &T) -> Option<&T> {
        match self.find(value) {
            Ok(index) => Some(&self.as_values()[index]),
            Err(_) => None,
        }
    }

    /// The values that fall in `range`, in order.
    ///
    /// # Panics
    ///
    /// Panics when the start of the range is past its end, which is a range that
    /// names nothing: the ordered collections of the standard library refuse it
    /// the same way, and answering with no values would hide the mistake. Both
    /// ends excluded around one value is that same shape, because every value
    /// would have to be greater than it and smaller than it; it is refused
    /// whether or not the value is in the set.
    pub fn range<R: core::ops::RangeBounds<T>>(&self, range: R) -> impl Iterator<Item = &T> {
        if let (core::ops::Bound::Excluded(start), core::ops::Bound::Excluded(end)) =
            (range.start_bound(), range.end_bound())
        {
            assert!(
                start != end,
                "the range excludes the same value at both ends"
            );
        }
        let values = self.values.as_slice();
        let start = match range.start_bound() {
            core::ops::Bound::Included(value) => values.partition_point(|held| held < value),
            core::ops::Bound::Excluded(value) => values.partition_point(|held| held <= value),
            core::ops::Bound::Unbounded => 0,
        };
        let end = match range.end_bound() {
            core::ops::Bound::Included(value) => {
                Some(values.partition_point(|held| held <= value))
            }
            core::ops::Bound::Excluded(value) => {
                Some(values.partition_point(|held| held < value))
            }
            core::ops::Bound::Unbounded => None,
        };
        let end = end.unwrap_or(values.len());
        assert!(
            start <= end,
            "the range starts past the value its end names"
        );
        values[start..end].iter()
    }
}

impl<T: StableElement> StableSet<T> {
    /// Adopts the storage of a set that arrived from another build.
    ///
    /// The values must be in ascending order, which is the invariant every set
    /// maintains; the length and capacity describe the block they sit in.
    ///
    /// # Safety
    ///
    /// `pointer` must be a live block from `allocator` with at least `capacity`
    /// slots, `length` of which hold initialised, sorted values.
    pub unsafe fn from_raw_parts(
        pointer: *mut T,
        length: u64,
        capacity: u64,
        allocator: AbiAllocator,
    ) -> AbiResult<Self> {
        // SAFETY: the caller guarantees the block and its contents.
        let values = unsafe { StableVec::from_raw_parts(pointer, length, capacity, allocator)? };
        Ok(Self { values })
    }

    /// Takes the set apart without releasing its storage.
    ///
    /// # Safety
    ///
    /// The returned block must be released exactly once, through the returned
    /// allocator, after its values are dropped.
    pub unsafe fn into_raw_parts(self) -> (*mut T, u64, u64, AbiAllocator) {
        // SAFETY: the caller takes over the obligation carried by the parts.
        unsafe { self.values.into_raw_parts() }
    }
}

impl<T: StableElement> Default for StableSet<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: StableElement + crate::zeroize::Wipe> crate::zeroize::Wipe for StableSet<T> {
    fn wipe_in_place(&mut self) {
        self.values.wipe_in_place();
    }
}

impl<T: StableElement + PartialEq> PartialEq for StableSet<T> {
    fn eq(&self, other: &Self) -> bool {
        self.as_values() == other.as_values()
    }
}

impl<T: StableElement + Eq> Eq for StableSet<T> {}

impl<T: StableElement> fmt::Debug for StableSet<T> {
    // The values can be content a log must not repeat, so the debug form reports
    // the shape. `iter` visits the values when a caller asks for them.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StableSet")
            .field("len", &self.len())
            .finish()
    }
}

impl<T: StableElement + Clone> Clone for StableSet<T> {
    /// Copies the values through [`Clone`], which raises a copy or an
    /// allocation that fails.
    fn clone(&self) -> Self {
        Self {
            values: self.values.clone(),
        }
    }
}

// SAFETY: the set is a `#[repr(C)]` struct holding one vector, and its drop
// releases every value through the vector's contract.
unsafe impl<T: StableElement> StableElement for StableSet<T> {
    unsafe fn drop_in_place(ptr: *mut Self) {
        // SAFETY: the caller guarantees an initialised value.
        unsafe { core::ptr::drop_in_place(ptr) }
    }
}

impl<T: StableElement + core::hash::Hash> core::hash::Hash for StableSet<T> {
    /// Folds the values, in the order the set stores them.
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        self.as_values().hash(state);
    }
}

impl<T: StableElement + Ord> Ord for StableSet<T> {
    /// Orders two sets value by value, then by length.
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        let left = self.as_values();
        let right = other.as_values();
        let shared = left.len().min(right.len());
        let mut index = 0;
        while index < shared {
            let ordering = left[index].cmp(&right[index]);
            if ordering != core::cmp::Ordering::Equal {
                return ordering;
            }
            index += 1;
        }
        left.len().cmp(&right.len())
    }
}

impl<T: StableElement + Ord> PartialOrd for StableSet<T> {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl<T: StableElement + Ord> core::iter::Extend<T> for StableSet<T> {
    /// Adds every value, panicking when the storage fails.
    ///
    /// The work is one merge rather than one insertion per value: use
    /// [`StableSet::try_extend`] where a failure has to be reported instead.
    fn extend<I: IntoIterator<Item = T>>(&mut self, values: I) {
        if let Err(error) = self.try_extend(values) {
            panic!("failed to extend a stable set: {error}");
        }
    }
}

impl<T: StableElement + Ord> core::iter::FromIterator<T> for StableSet<T> {
    /// Builds a set from values, panicking when an insertion fails.
    fn from_iter<I: IntoIterator<Item = T>>(values: I) -> Self {
        let mut set = Self::new();
        set.extend(values);
        set
    }
}

impl<T: StableElement> core::iter::IntoIterator for StableSet<T> {
    type Item = T;
    type IntoIter = crate::vec::StableIntoIter<T>;

    fn into_iter(self) -> Self::IntoIter {
        self.values.into_iter()
    }
}

impl<'a, T: StableElement> core::iter::IntoIterator for &'a StableSet<T> {
    type Item = &'a T;
    type IntoIter = core::slice::Iter<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.as_values().iter()
    }
}

impl<'a, T: StableElement> core::iter::IntoIterator for &'a mut StableSet<T> {
    type Item = &'a mut T;
    type IntoIter = core::slice::IterMut<'a, T>;

    /// Borrows every value for writing.
    ///
    /// A caller that changes a value is responsible for the order, as with
    /// [`StableMap::as_entries_mut`](crate::StableMap::as_entries_mut).
    fn into_iter(self) -> Self::IntoIter {
        self.values.as_mut_slice().iter_mut()
    }
}
