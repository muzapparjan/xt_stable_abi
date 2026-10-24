//! Names for objects that outlive a single call.
//!
//! A handle is a `u64` made of an index and a generation counter. The counter
//! is what makes a stale handle harmless: once an entry is removed, its index
//! is reused with a different generation, and a handle carrying the old
//! generation no longer resolves.

use core::fmt;
use core::mem;

use crate::heap::vec::Vec;

use crate::error::{AbiError, AbiResult, AbiStatus};
use crate::vec::StableElement;

/// The first generation written into a slot.
const FIRST_GENERATION: u32 = 1;

// A slot's generation is 32 bits wide, so a table that frees and fills the same
// slot about four billion times wraps back to a generation it used before, and a
// name retired that long ago would resolve again. A host that must never present
// a stale name should rebuild the table (or advance it past a full cycle) instead
// of relying on the counter alone; there is no cheaper way to name `2^32` 
// distinct lifetimes through a 32-bit field.

/// A name for an entry in a [`HandleTable`].
///
/// The low half indexes the slot and the high half holds the generation. The
/// all-ones word is reserved for [`Handle::NONE`], so a handle that was never
/// produced by a table never resolves.
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Handle(u64);

impl Handle {
    /// A handle that names nothing.
    pub const NONE: Self = Handle(u64::MAX);

    /// Builds a handle from a slot index and a generation.
    pub const fn from_parts(index: u32, generation: u32) -> Self {
        Handle(((generation as u64) << 32) | index as u64)
    }

    /// Reports whether this handle names nothing.
    pub const fn is_none(self) -> bool {
        self.0 == u64::MAX
    }

    /// The slot index, or `None` for [`Handle::NONE`].
    pub const fn index(self) -> Option<u32> {
        if self.is_none() {
            None
        } else {
            Some(self.0 as u32)
        }
    }

    /// The generation stored in the handle.
    pub const fn generation(self) -> u32 {
        (self.0 >> 32) as u32
    }

    /// The raw word, for storing in a boundary struct.
    pub const fn to_raw(self) -> u64 {
        self.0
    }

    /// Rebuilds a handle from a raw word.
    pub const fn from_raw(raw: u64) -> Self {
        Handle(raw)
    }
}

impl fmt::Debug for Handle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_none() {
            formatter.write_str("Handle(NONE)")
        } else {
            formatter
                .debug_struct("Handle")
                .field("index", &self.index().unwrap_or_default())
                .field("generation", &self.generation())
                .finish()
        }
    }
}

impl fmt::Display for Handle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.index() {
            Some(index) => write!(formatter, "{}:{}:{}", index, self.generation(), self.0),
            None => formatter.write_str("none"),
        }
    }
}

// SAFETY: the handle is one word with the same layout on both sides, and it owns
// nothing.
unsafe impl StableElement for Handle {
    unsafe fn drop_in_place(ptr: *mut Self) {
        // SAFETY: the caller guarantees an initialised value.
        unsafe { core::ptr::drop_in_place(ptr) }
    }
}

impl crate::zeroize::Wipe for Handle {
    fn wipe_in_place(&mut self) {
        // The reserved word names nothing, so the result is a handle rather than
        // a number that might resolve to an entry. The store is volatile for the
        // reason every other wipe here is: a plain assignment to memory that is
        // about to be released is the compiler's to remove.
        // SAFETY: `self` is a live, writable value of this type.
        unsafe { core::ptr::write_volatile(self, Handle::NONE) };
        core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
    }
}

/// One slot of a [`HandleTable`].
enum Entry<T> {
    Free { generation: u32 },
    Occupied { generation: u32, value: T },
}

impl<T> Entry<T> {
    fn generation(&self) -> u32 {
        match self {
            Entry::Free { generation } | Entry::Occupied { generation, .. } => *generation,
        }
    }
}

/// The next generation after `generation`.
///
/// Zero and `u32::MAX` never name a live entry: zero keeps a freshly built
/// table distinguishable from a real slot, and `u32::MAX` is the generation
/// part of [`Handle::NONE`].
fn next_generation(generation: u32) -> u32 {
    let next = generation.wrapping_add(1);
    if next == 0 || next == u32::MAX {
        FIRST_GENERATION
    } else {
        next
    }
}

/// A table that hands out [`Handle`]s for values it owns.
///
/// The table is the side that owns the values; the other side only ever holds
/// the `u64` name, so it can pass the name back without knowing the type.
pub struct HandleTable<T> {
    slots: Vec<Entry<T>>,
    free: Vec<u32>,
    length: usize,
    /// The generation a slot of a later block starts with, or zero before the
    /// block is first given up.
    ///
    /// Emptying the block ([`HandleTable::wipe_in_place`]) throws away the
    /// generations the slots carried, so a new slot cannot start from
    /// [`FIRST_GENERATION`] again: a handle the caller still holds would then be
    /// the word a new slot answers to. The number recorded here is the one after
    /// the highest generation the block ever held, and a later slot begins there.
    base_generation: u32,
}

impl<T> fmt::Debug for HandleTable<T> {
    // The values can be content a log must not repeat, so the debug form reports
    // the shape of the table instead of the entries it holds.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("HandleTable")
            .field("len", &self.length)
            .field("slots", &self.slots.len())
            .field("free", &self.free.len())
            .finish()
    }
}

impl<T> HandleTable<T> {
    /// An empty table.
    pub const fn new() -> Self {
        Self {
            slots: Vec::new(),
            free: Vec::new(),
            length: 0,
            base_generation: 0,
        }
    }

    /// An empty table with room for `capacity` entries.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            slots: Vec::with_capacity(capacity),
            free: Vec::new(),
            length: 0,
            base_generation: 0,
        }
    }

    /// The number of live entries.
    pub fn len(&self) -> usize {
        self.length
    }

    /// The number of entries the slot storage can hold without growing.
    pub fn capacity(&self) -> usize {
        self.slots.capacity()
    }

    /// Reserves slot storage for `additional` entries.
    pub fn reserve(&mut self, additional: usize) {
        self.slots.reserve(additional);
        self.free.reserve(additional);
    }

    /// Reports whether the table holds no live entries.
    pub fn is_empty(&self) -> bool {
        self.length == 0
    }

    /// Stores `value` and returns the name that resolves to it.
    ///
    /// # Panics
    ///
    /// Panics when the table already holds `u32::MAX` slots; slot indices are
    /// 32 bits wide by construction. It also panics when the slot storage cannot
    /// grow: this type keeps its entries in a `Vec`, so asking it for memory is
    /// the allocation library's question, which answers by aborting. A host that
    /// fills a table on a path that must not stop calls
    /// [`HandleTable::reserve`] while it can still report a failure.
    pub fn insert(&mut self, value: T) -> Handle {
        if let Some(index) = self.free.pop() {
            let generation = self.slots[index as usize].generation();
            self.slots[index as usize] = Entry::Occupied { generation, value };
            self.length += 1;
            return Handle::from_parts(index, generation);
        }
        assert!(
            self.slots.len() < u32::MAX as usize,
            "handle table exhausted its 32-bit slot space"
        );
        let index = self.slots.len() as u32;
        let generation = self.starting_generation();
        self.slots.push(Entry::Occupied {
            generation,
            value,
        });
        self.length += 1;
        Handle::from_parts(index, generation)
    }

    /// The generation a slot that has never been used starts with.
    ///
    /// A table that was never wiped begins at [`FIRST_GENERATION`]. One that was
    /// begins past every generation its block held, which is what keeps a handle
    /// taken before the wipe from naming a value stored after it.
    fn starting_generation(&self) -> u32 {
        if self.base_generation == 0 {
            FIRST_GENERATION
        } else {
            self.base_generation
        }
    }

    /// Resolves `handle` to the value it names.
    pub fn get(&self, handle: Handle) -> Option<&T> {
        let index = handle.index()? as usize;
        match self.slots.get(index) {
            Some(Entry::Occupied { generation, value }) if *generation == handle.generation() => {
                Some(value)
            }
            _ => None,
        }
    }

    /// Resolves `handle` to the value it names, or reports
    /// [`AbiStatus::InvalidHandle`].
    pub fn require(&self, handle: Handle) -> AbiResult<&T> {
        match self.get(handle) {
            Some(value) => Ok(value),
            None => Err(AbiError::with_message(
                AbiStatus::InvalidHandle,
                "the handle does not name a live entry",
            )),
        }
    }

    /// Resolves `handle` to the value it names, mutably, or reports
    /// [`AbiStatus::InvalidHandle`].
    pub fn require_mut(&mut self, handle: Handle) -> AbiResult<&mut T> {
        match self.get_mut(handle) {
            Some(value) => Ok(value),
            None => Err(AbiError::with_message(
                AbiStatus::InvalidHandle,
                "the handle does not name a live entry",
            )),
        }
    }

    /// Resolves `handle` to the value it names, mutably.
    pub fn get_mut(&mut self, handle: Handle) -> Option<&mut T> {
        let index = handle.index()? as usize;
        match self.slots.get_mut(index) {
            Some(Entry::Occupied { generation, value }) if *generation == handle.generation() => {
                Some(value)
            }
            _ => None,
        }
    }

    /// Reports whether `handle` names a live entry.
    pub fn contains(&self, handle: Handle) -> bool {
        self.get(handle).is_some()
    }

    /// Removes the entry `handle` names and returns its value.
    ///
    /// The slot becomes free with a new generation, so the same handle never
    /// resolves again.
    pub fn remove(&mut self, handle: Handle) -> Option<T> {
        let index = handle.index()? as usize;
        match self.slots.get(index) {
            Some(Entry::Occupied { generation, .. }) if *generation == handle.generation() => {}
            _ => return None,
        }
        let next = next_generation(handle.generation());
        let previous = mem::replace(&mut self.slots[index], Entry::Free { generation: next });
        self.free.push(index as u32);
        self.length -= 1;
        match previous {
            Entry::Occupied { value, .. } => Some(value),
            Entry::Free { .. } => None,
        }
    }

    /// Removes every entry.
    ///
    /// Slots stay allocated and every live generation is advanced, so handles
    /// taken before the call never resolve afterwards.
    pub fn clear(&mut self) {
        for index in 0..self.slots.len() {
            let is_occupied = matches!(self.slots[index], Entry::Occupied { .. });
            if !is_occupied {
                continue;
            }
            let next = next_generation(self.slots[index].generation());
            self.slots[index] = Entry::Free { generation: next };
            self.free.push(index as u32);
        }
        self.length = 0;
    }

    /// Removes the entries for which `keep` returns false.
    ///
    /// The keys see the entries in slot order. A slot the predicate rejects is
    /// freed and its generation advanced, so a handle taken before the call does
    /// not resolve to the value that arrives later.
    pub fn retain<F: FnMut(Handle, &mut T) -> bool>(&mut self, mut keep: F) {
        for index in 0..self.slots.len() {
            let keep_entry = match &mut self.slots[index] {
                Entry::Occupied { generation, value } => {
                    keep(Handle::from_parts(index as u32, *generation), value)
                }
                Entry::Free { .. } => continue,
            };
            if keep_entry {
                continue;
            }
            let next = next_generation(self.slots[index].generation());
            self.slots[index] = Entry::Free { generation: next };
            self.free.push(index as u32);
            self.length -= 1;
        }
    }

    /// Removes every entry and yields the values with the handles that named
    /// them.
    ///
    /// Each slot the iteration reaches is freed with a new generation, so a
    /// handle taken before the call never resolves to a value that arrives
    /// later. Slots the iteration has not reached still hold their values, and
    /// dropping the iterator leaves them in the table.
    pub fn drain(&mut self) -> Drain<'_, T> {
        Drain {
            table: self,
            index: 0,
        }
    }

    /// Iterates over the live entries and their handles.
    pub fn iter(&self) -> impl Iterator<Item = (Handle, &T)> {
        self.slots
            .iter()
            .enumerate()
            .filter_map(|(index, entry)| match entry {
                Entry::Occupied { generation, value } => {
                    Some((Handle::from_parts(index as u32, *generation), value))
                }
                Entry::Free { .. } => None,
            })
    }

    /// The live values.
    pub fn values(&self) -> impl Iterator<Item = &T> {
        self.iter().map(|(_, value)| value)
    }

    /// Iterates over the live entries and their handles, mutably.
    pub fn iter_mut(&mut self) -> impl Iterator<Item = (Handle, &mut T)> {
        self.slots
            .iter_mut()
            .enumerate()
            .filter_map(|(index, entry)| match entry {
                Entry::Occupied { generation, value } => {
                    Some((Handle::from_parts(index as u32, *generation), value))
                }
                Entry::Free { .. } => None,
            })
    }

    /// The live values, mutably.
    pub fn values_mut(&mut self) -> impl Iterator<Item = &mut T> {
        self.iter_mut().map(|(_, value)| value)
    }

    /// The handles that currently resolve.
    pub fn handles(&self) -> impl Iterator<Item = Handle> + '_ {
        self.iter().map(|(handle, _)| handle)
    }
}

/// The iteration [`HandleTable::drain`] answers with.
///
/// Each step frees the slot it takes and advances that slot's generation, so the
/// table stays consistent at every point: a value the iteration has not reached
/// is still live, and a handle taken before the drain no longer resolves for any
/// slot the iteration has passed.
pub struct Drain<'a, T> {
    table: &'a mut HandleTable<T>,
    index: usize,
}

impl<T> Iterator for Drain<'_, T> {
    type Item = (Handle, T);

    fn next(&mut self) -> Option<(Handle, T)> {
        while self.index < self.table.slots.len() {
            let index = self.index;
            self.index += 1;
            let generation = match self.table.slots[index] {
                Entry::Occupied { generation, .. } => generation,
                Entry::Free { .. } => continue,
            };
            let next = next_generation(generation);
            let previous =
                mem::replace(&mut self.table.slots[index], Entry::Free { generation: next });
            self.table.free.push(index as u32);
            self.table.length -= 1;
            match previous {
                Entry::Occupied { value, .. } => {
                    return Some((Handle::from_parts(index as u32, generation), value));
                }
                // The match above established that this slot was occupied.
                Entry::Free { .. } => unreachable!(),
            }
        }
        None
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.table.length, Some(self.table.length))
    }
}

impl<T> ExactSizeIterator for Drain<'_, T> {}

impl<T> Drop for Drain<'_, T> {
    /// Leaves the slots this iteration did not reach in place.
    ///
    /// Each slot it did reach was freed as it was yielded, so there is nothing
    /// to finish here; the destructor exists to document that an abandoned
    /// iteration is a table that still holds the entries after the cursor.
    fn drop(&mut self) {}
}

impl<T> fmt::Debug for Drain<'_, T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Drain")
            .field("remaining", &self.table.length)
            .finish_non_exhaustive()
    }
}

impl<T> Default for HandleTable<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: crate::zeroize::Wipe> crate::zeroize::Wipe for HandleTable<T> {
    /// Wipes every value the table holds, then the memory that held them.
    ///
    /// Freeing a slot writes the bookkeeping over the value's place, it does not
    /// overwrite the value, so the block of slots can still hold what an earlier
    /// value was. The values are wiped while they are still there, the slots are
    /// released, and the block is written over afterwards: the table is left
    /// empty rather than holding a scrubbed value.
    ///
    /// Every slot's generation is advanced before the block is cleared, so a
    /// handle taken before the call names nothing afterwards: the slots this
    /// call empties are not handed out again, and the ones a later insert
    /// creates carry generations past the ones that were retired here.
    fn wipe_in_place(&mut self) {
        // The starting generation for a later block is decided before anything is
        // released: it is one past the highest this block ever held, so a handle
        // taken before this call cannot name a slot created after it.
        let mut highest = 0;
        for slot in &self.slots {
            highest = highest.max(slot.generation());
        }
        if highest > 0 {
            let next = next_generation(highest);
            // A wrap makes the new base no higher than the old ones; that is the
            // 32-bit limit this table already documents, and it is not made worse
            // by the wipe.
            self.base_generation = next;
        }
        for index in 0..self.slots.len() {
            if let Entry::Occupied { value, .. } = &mut self.slots[index] {
                value.wipe_in_place();
            }
            // The slot is retired whether it was occupied or already free: a
            // free slot's generation is what a later insert would reuse, and a
            // handle may still carry it.
            let next = next_generation(self.slots[index].generation());
            self.slots[index] = Entry::Free { generation: next };
        }
        // The retired generations stay in the block that is about to be written
        // over, so the table starts past them rather than at the first slot.
        let capacity = self.slots.capacity();
        let pointer = self.slots.as_mut_ptr();
        self.slots.clear();
        self.free.clear();
        self.length = 0;
        if capacity > 0 {
            // SAFETY: the block belongs to this table, no initialised entry is
            // left in it, and the write stays inside the allocation.
            let _ = unsafe {
                crate::zeroize::volatile_zero_raw(
                    pointer.cast(),
                    capacity as u64,
                    core::mem::size_of::<Entry<T>>() as u64,
                )
            };
        }
    }
}
