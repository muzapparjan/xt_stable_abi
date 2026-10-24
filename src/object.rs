//! Trait objects with a hand-written virtual table.
//!
//! `dyn Trait` embeds the compiler's own vtable pointer, whose layout is not
//! part of any stable contract. [`StableDyn`] replaces it with two words, the
//! address of the value and the address of a table whose first entry is a
//! [`VTablePrefix`], plus the allocator that owns the value. The methods after
//! the prefix are laid out by [`crate::stable_object!`], which also generates
//! the typed accessors that read them.

use core::any::TypeId;
use core::fmt;
use core::mem::{align_of, size_of, ManuallyDrop};
use core::ptr;
use core::sync::atomic::{AtomicBool, AtomicPtr, Ordering};

use crate::heap::boxed::Box;
use crate::heap::vec::Vec;

use crate::alloc::AbiAllocator;
use crate::cache::{Once, SpinLock};
use crate::error::{AbiError, AbiResult, AbiStatus};
use crate::vec::StableElement;

/// Orders the entries of a table by the signature each one declares.
///
/// A call finds its slot by comparing identifiers, so the order the entries sit
/// in is free to be the one the search wants: ascending. The macro that builds a
/// table calls this once, when the table is built, so every call after it is a
/// binary search rather than a walk.
///
/// # Safety
///
/// `table` must point at the first byte of a table and carry permission for the
/// whole of it — the header, the `slots` entries after it and the sentinel — and
/// that table must not be shared with another thread while this runs.
pub unsafe fn sort_table_entries(table: *mut u8, slots: u64) {
    let count = slots.min(MAX_SLOTS) as usize;
    // SAFETY: the caller grants the whole table, so the header may be read and
    // written.
    let prefix = table as *mut VTablePrefix;
    if count >= 2 {
        // SAFETY: the caller grants the whole table, and `slots` counts the
        // entries that follow the header.
        let entries = unsafe { table.add(size_of::<VTablePrefix>()) } as *mut VTableSlot;
        // SAFETY: the range is the one the caller described, and it is writable.
        let entries = unsafe { core::slice::from_raw_parts_mut(entries, count) };
        entries.sort_unstable_by_key(|entry| entry.signature);
    }
    // SAFETY: the caller grants the whole table, and the count is written back
    // with the flag that records what this function did.
    unsafe { (*prefix).slots = count as u64 | SORTED_FLAG };
}

/// The value a guarded trampoline writes into the status word when the
/// implementation panicked.
pub const PANICKED: u32 = 1;

/// The receiver forms a slot can declare.
///
/// The form is part of the declaration: a slot that writes through the receiver
/// is not the one that only reads it, and a guarded slot is not the one that
/// lets a panic unwind. The six values name the six shapes a method can take.
pub const RECEIVER_SHARED: u64 = 1;
/// A method that takes `&mut self`.
pub const RECEIVER_MUTABLE: u64 = 2;
/// A method that takes `self` by value.
pub const RECEIVER_BY_VALUE: u64 = 3;
/// A `&self` method whose table contains panics.
pub const RECEIVER_SHARED_GUARDED: u64 = 4;
/// A `&mut self` method whose table contains panics.
pub const RECEIVER_MUTABLE_GUARDED: u64 = 5;
/// A by-value method whose table contains panics.
pub const RECEIVER_BY_VALUE_GUARDED: u64 = 6;

/// The identifier of one slot, built from the method, the receiver form and the
/// types the method names.
///
/// The types enter as their own [`AbiIdentity`](crate::identity::AbiIdentity)
/// numbers, so how a declaration spells them does not matter: an alias, a path
/// prefix and an elided lifetime all name the type they stand for.
///
/// The answer is never zero, because zero ends the entry list.
pub const fn slot_id(name: &str, receiver: u64, arguments: &[u64], returned: u64) -> u64 {
    let mut hash = declaration_id(name) ^ receiver.rotate_left(32);
    let mut index = 0;
    while index < arguments.len() {
        hash = mix_word(hash, arguments[index]);
        index += 1;
    }
    hash = mix_word(hash, returned);
    if hash == 0 { 1 } else { hash }
}

/// Reports whether `size` at `align` describes a block a layout can name.
///
/// A block is described by a layout, and a layout rounds its size up to the
/// alignment. A size that is addressable on its own may still fail to round
/// within `isize::MAX`, and every call that takes a layout - `deallocate`,
/// `reallocate`, and the wipe that overwrites the block - would then be handed
/// one it may not accept. The value's own header is where that has to be
/// refused, because nothing later in the chain reads the table's account again.
const fn is_layout_size_valid(size: u64, align: u64) -> bool {
    match size.checked_add(align - 1) {
        Some(rounded) => rounded <= isize::MAX as u64,
        None => false,
    }
}

/// Folds one word into `hash`, byte by byte, so the order of the words counts.
const fn mix_word(hash: u64, value: u64) -> u64 {
    let mut hash = hash;
    let mut index = 0;
    while index < 8 {
        hash ^= (value >> (index * 8)) & 0xff;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        index += 1;
    }
    hash
}

/// The value a guarded trampoline writes into the status word when an argument
/// was refused, which is a statement about the call rather than about the
/// implementation.
pub const ARGUMENT_REFUSED: u32 = 2;

/// The error a guarded call reports from the word its trampoline wrote.
///
/// The two failures are different facts. A panic says the implementation began
/// and gave up; a refused argument says the call never reached it, because a
/// value the sender described would not have been readable. A caller that retries
/// or blames the implementation on the wrong one of those acts on a false
/// premise, so the status distinguishes them.
pub fn outcome_error(word: u32) -> AbiError {
    if word == ARGUMENT_REFUSED {
        AbiError::with_message(
            AbiStatus::InvalidArgument,
            "an argument is not a value its type defines",
        )
    } else {
        AbiError::with_message(AbiStatus::Panic, "the implementation panicked")
    }
}

/// The revision of this crate's boundary layouts.
///
/// The number changes when the layout of a boundary type changes, and when the
/// descriptor layout changes: a table, a descriptor and the host description all
/// record it, and all are compared before the bytes they describe are used. A
/// value built by a build with a different layout is rejected even if the trait
/// declaration reads the same.
pub const LAYOUT_REVISION: u64 = 5;

/// The largest number of slots a table may declare.
///
/// A table records its own slot count; this bound is what keeps a number that
/// arrived from another build from being trusted past the end of any table this
/// crate could have built.
pub const MAX_SLOTS: u64 = 1024;

/// The bit of [`VTablePrefix::slots`] that records ordered entries.
///
/// A count of slots never reaches this bit ([`MAX_SLOTS`] is far below it), so
/// the two share one field without a second layout revision. A table this crate
/// builds sets it when it sorts its entries, which is what lets a call skip the
/// scan that a table written by hand still needs after a miss.
pub const SORTED_FLAG: u64 = 1 << 63;

/// The length up to which a table is walked rather than searched.
///
/// Below this the walk is the cheaper of the two: the ordered search pays for
/// its loop and for the divisions before it has read as many entries as a walk
/// would. The number is where the two meet for the entries this crate builds.
pub const SMALL_TABLE: u64 = 16;

/// Abandons `value` without running its drop glue.
///
/// A call whose argument was refused abandons that argument rather than
/// releasing it: the bytes do not describe an owner this side may hand back, so
/// running the destructor would read numbers the sender did not stand behind.
///
/// This is what [`core::mem::forget`] does. It is wrapped so that a build which
/// lints its dependencies reads one operation here instead of a `forget` inside
/// an expanded declaration, where the type of the argument is not visible and a
/// type without drop glue reads as a mistake.
#[doc(hidden)]
#[inline]
pub fn abandon<T>(value: T) {
    core::mem::forget(value);
}

/// The header every generated virtual table starts with.
///
/// Because it is the first field of a `#[repr(C)]` table, a pointer to the
/// table and a pointer to the prefix have the same address, which is how
/// [`StableDyn`] stays independent of the trait it carries.
#[repr(C)]
pub struct VTablePrefix {
    /// A stable identifier chosen by whoever created the value.
    pub type_id: u64,
    /// An identifier over the whole set of methods.
    ///
    /// It answers "is this the same declaration" with one comparison. The fold
    /// is commutative ([`combined_signature`]), so the order the declaration
    /// lists its methods in does not change the number. A receiver that must
    /// also accept a peer which declares fewer methods compares the slots
    /// themselves instead.
    pub signature: u64,
    /// The revision of the boundary layouts the table was built against.
    pub layout_revision: u64,
    /// The size of the value in bytes.
    pub size: u64,
    /// The alignment of the value in bytes.
    pub align: u64,
    /// The number of slots that follow the header, not counting the sentinel.
    ///
    /// The count is recorded rather than inferred by scanning, because a scan
    /// has to trust a sentinel that a foreign table may not have. A receiver
    /// reads at most this many entries, and at most [`MAX_SLOTS`].
    ///
    /// The top bit is not part of the count: it records that the entries are
    /// ordered, which is what [`sort_table_entries`] sets (see
    /// [`SORTED_FLAG`]). A table written by hand leaves it clear and is searched
    /// by a scan as well.
    pub slots: u64,
    /// Releases the value but not its storage.
    ///
    /// The field is optional so that a table which records no destructor is a
    /// value this build can read: a function pointer that arrived as zero would
    /// be an invalid value the moment the header is read. A table without one is
    /// refused by [`StableDyn::validate`], because a value nothing can release
    /// is not one this build may accept.
    pub drop: Option<unsafe extern "C" fn(*mut u8)>,
}

impl fmt::Debug for VTablePrefix {
    // The drop function is an address in the library that built the table;
    // printing it would put the layout of that library into logs.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VTablePrefix")
            .field("type_id", &self.type_id)
            .field("signature", &self.signature)
            .field("layout_revision", &self.layout_revision)
            .field("size", &self.size)
            .field("align", &self.align)
            .field("slots", &self.slots)
            .finish_non_exhaustive()
    }
}

/// One entry of a generated table.
///
/// The signature names the method the entry calls, so a receiver finds the slot
/// it needs by comparing numbers rather than by trusting a position. A method a
/// peer does not declare simply has no entry, which is what lets one side
/// declare a method the other does not have yet.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VTableSlot {
    /// The identifier of the method this entry calls.
    pub signature: u64,
    /// The address of the trampoline.
    ///
    /// The function has the parameter list the signing side declared: the value
    /// address, then any arguments. It is stored untyped because the parameter
    /// list differs per slot; a receiver transmutes it only after the signature
    /// matched.
    pub call: *const (),
}

impl fmt::Debug for VTableSlot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VTableSlot")
            .field("signature", &self.signature)
            .finish_non_exhaustive()
    }
}

/// An owned trait object with a declared table layout.
///
/// Dropping the value calls the table's `drop` entry and then releases the
/// storage through the allocator recorded here.
///
/// The handle is neither `Send` nor `Sync`: it offers a raw address and a table
/// for whatever the table was built for, and whether that value may cross a
/// thread boundary is a property of the implementation. A wrapper that knows
/// its implementation can state that guarantee itself.
#[repr(C)]
pub struct StableDyn {
    data: *mut u8,
    vtable: *const VTablePrefix,
    allocator: AbiAllocator,
}

impl StableDyn {
    /// A value with no object behind it.
    pub const fn empty() -> Self {
        Self {
            data: ptr::null_mut(),
            vtable: ptr::null(),
            allocator: AbiAllocator::system(),
        }
    }

    /// Rebuilds a value from its parts.
    ///
    /// # Safety
    ///
    /// `data` must point at an initialised value whose layout matches
    /// `vtable.size`/`vtable.align`, the table must stay alive for the life of
    /// the value, and `allocator` must be the one that produced `data`.
    pub const unsafe fn from_raw_parts(
        data: *mut u8,
        vtable: *const VTablePrefix,
        allocator: AbiAllocator,
    ) -> Self {
        Self {
            data,
            vtable,
            allocator,
        }
    }

    /// Takes the value apart without dropping it.
    ///
    /// # Safety
    ///
    /// The returned parts must be handed back to
    /// [`StableDyn::from_raw_parts`] (or released by hand) exactly once.
    pub unsafe fn into_raw_parts(self) -> (*mut u8, *const VTablePrefix, AbiAllocator) {
        let this = ManuallyDrop::new(self);
        (this.data, this.vtable, this.allocator)
    }

    /// Reports whether the value carries no object.
    pub const fn is_empty(&self) -> bool {
        self.vtable.is_null()
    }

    /// The address of the value.
    pub const fn data_ptr(&self) -> *mut u8 {
        self.data
    }

    /// The address of the table prefix.
    pub const fn vtable_ptr(&self) -> *const VTablePrefix {
        self.vtable
    }

    /// The allocator that owns the value's storage.
    pub const fn allocator(&self) -> AbiAllocator {
        self.allocator
    }

    /// The identifier recorded when the value was built, or zero when the value
    /// carries no object.
    pub fn type_id(&self) -> u64 {
        if self.vtable.is_null() {
            0
        } else {
            // SAFETY: a non-null table stays alive for the value's lifetime.
            unsafe { (*self.vtable).type_id }
        }
    }

    /// The table signature recorded when the value was built, or zero when the
    /// value carries no object.
    pub fn signature(&self) -> u64 {
        if self.vtable.is_null() {
            0
        } else {
            // SAFETY: a non-null table stays alive for the value's lifetime.
            unsafe { (*self.vtable).signature }
        }
    }

    /// The number of slots the table declares, or zero when the value carries
    /// no object.
    pub fn slots(&self) -> u64 {
        if self.vtable.is_null() {
            return 0;
        }
        // SAFETY: a non-null table stays alive for the value's lifetime.
        unsafe { ((*self.vtable).slots & !SORTED_FLAG).min(MAX_SLOTS) }
    }

    /// Checks that the handle describes one value.
    ///
    /// A handle with no table is the empty value, which must have no address
    /// too; a handle with a table must sit on an address the header can be read
    /// at, must agree on the layout revision, must record a destructor, and must
    /// have an address that is aligned for the layout the table records.
    pub fn validate(&self) -> AbiResult<()> {
        if self.vtable.is_null() {
            return if self.data.is_null() {
                Ok(())
            } else {
                Err(crate::error::AbiError::new(
                    crate::error::AbiStatus::InvalidDescriptor,
                ))
            };
        }
        if self.data.is_null() {
            return Err(crate::error::AbiError::new(
                crate::error::AbiStatus::NullPointer,
            ));
        }
        // The header is read below, so the table address has to be one this
        // build could have written a header at before any field of it is used.
        if !(self.vtable as usize).is_multiple_of(align_of::<VTablePrefix>()) {
            return Err(crate::error::AbiError::new(
                crate::error::AbiStatus::InvalidLayout,
            ));
        }
        // A table describes entries by offsets this build computes from its own
        // header layout, so a table written against another revision is refused
        // before any of it is read.
        // SAFETY: a non-null table stays alive for the value's lifetime, and the
        // address is aligned for the header.
        let prefix = unsafe { &*self.vtable };
        if prefix.layout_revision != LAYOUT_REVISION {
            return Err(crate::error::AbiError::new(
                crate::error::AbiStatus::VersionMismatch,
            ));
        }
        // A value nothing can release is not one this build may accept: the
        // block would be left behind, and the destructor is the only place that
        // knows what the value owns.
        if prefix.drop.is_none() {
            return Err(crate::error::AbiError::new(
                crate::error::AbiStatus::InvalidDescriptor,
            ));
        }
        // The count is recorded by the other side, so it is checked against the
        // same bound a receiver applies when it reads the entries.
        if prefix.slots & !SORTED_FLAG > MAX_SLOTS {
            return Err(crate::error::AbiError::new(
                crate::error::AbiStatus::InvalidLayout,
            ));
        }
        let (size, align) = (prefix.size, prefix.align);
        if !align.is_power_of_two() || align > isize::MAX as u64 {
            return Err(crate::error::AbiError::new(
                crate::error::AbiStatus::InvalidLayout,
            ));
        }
        // A block is described by a layout, and a layout rounds its size up to
        // the alignment. The rounded size is what `deallocate`, `Wipe` and the
        // destructor hand to the allocator, so a size that cannot be rounded
        // within `isize::MAX` is one this build must refuse before any of those
        // runs.
        if !is_layout_size_valid(size, align) || !(self.data as usize).is_multiple_of(align as usize)
        {
            return Err(crate::error::AbiError::new(
                crate::error::AbiStatus::InvalidLayout,
            ));
        }
        Ok(())
    }

    /// Reports whether the handle describes a block this build may release.
    ///
    /// The test is [`StableDyn::validate`]: a table written against another
    /// layout revision, an alignment that cannot be a real one, and an address
    /// that does not sit on that alignment all say the block cannot be named
    /// from what the table records. Such a value is dropped without calling the
    /// table's `drop` and without releasing anything, because both would use
    /// numbers whose meaning this build cannot establish.
    pub fn is_releasable(&self) -> bool {
        self.validate().is_ok()
    }

    /// The boundary layout revision the table was built against, or zero when
    /// the value carries no object.
    pub fn layout_revision(&self) -> u64 {
        if self.vtable.is_null() {
            0
        } else {
            // SAFETY: a non-null table stays alive for the value's lifetime.
            unsafe { (*self.vtable).layout_revision }
        }
    }

    /// The trampoline of the method identified by `signature`, if the table
    /// declares it.
    ///
    /// The entries start right after the header, which is why the address of
    /// the table and the address of the header are the same. A table this crate
    /// builds is ordered by signature ([`sort_table_entries`]) and says so in its
    /// header, so the search is a binary one and a miss is final. A table written
    /// by hand records no order, so a miss is followed by a scan. The header
    /// records how many entries there are, so neither walk passes the end of the
    /// table whatever the entries contain.
    pub fn slot(&self, signature: u64) -> Option<*const ()> {
        if self.vtable.is_null() || signature == 0 {
            return None;
        }
        // SAFETY: a non-null table stays alive for the value's lifetime.
        let (entries_address, recorded) = unsafe {
            (
                (self.vtable as *const u8).add(size_of::<VTablePrefix>()),
                (*self.vtable).slots,
            )
        };
        let count = (recorded & !SORTED_FLAG).min(MAX_SLOTS);
        let ordered = recorded & SORTED_FLAG != 0;
        let entries = entries_address as *const VTableSlot;
        // A short table is cheaper to walk than to divide: the point where the
        // two meet was measured at sixteen entries, and most declarations never
        // reach it.
        if ordered && count > SMALL_TABLE {
            let mut low = 0u64;
            let mut high = count;
            while low < high {
                let middle = low + (high - low) / 2;
                // SAFETY: the middle index is inside the count the header records.
                let entry = unsafe { *entries.add(middle as usize) };
                match entry.signature.cmp(&signature) {
                    core::cmp::Ordering::Less => low = middle + 1,
                    core::cmp::Ordering::Greater => high = middle,
                    core::cmp::Ordering::Equal => {
                        // An entry that names a signature but carries no address
                        // has nothing to call. Answering with the null address
                        // would make the accessor transmute it into a function
                        // pointer and call it, so it is treated as absent.
                        return if entry.call.is_null() {
                            None
                        } else {
                            Some(entry.call)
                        };
                    }
                }
            }
            return None;
        }
        // A short table is walked, and a long one is walked only when it records
        // no order: a table this build did not order (a hand-written one, for
        // instance) may hold its entries in any order.
        let mut index = 0;
        while index < count {
            // SAFETY: the index is inside the count the header records.
            let entry = unsafe { *entries.add(index as usize) };
            if entry.signature == signature {
                return if entry.call.is_null() {
                    None
                } else {
                    Some(entry.call)
                };
            }
            index += 1;
        }
        None
    }

    /// The signature a table entry declares, or `None` when the table is empty
    /// or the index is past the entries it records.
    ///
    /// A receiver that must accept a peer declaring fewer methods than this
    /// build reads the entries this way and checks each one against its own
    /// declaration; the header's `signature` answers the all-or-nothing case in
    /// one comparison.
    pub fn slot_signature(&self, index: u64) -> Option<u64> {
        if self.vtable.is_null() {
            return None;
        }
        // SAFETY: a non-null table stays alive for the value's lifetime.
        let count = self.slots();
        if index >= count {
            return None;
        }
        let entries = unsafe {
            (self.vtable as *const u8).add(size_of::<VTablePrefix>()) as *const VTableSlot
        };
        // SAFETY: the index is inside the entries the header records.
        Some(unsafe { (*entries.add(index as usize)).signature })
    }

    /// Reports whether a table declares exactly the methods in `own`.
    ///
    /// The order the entries are written in is not part of a declaration: a call
    /// finds its slot by comparing signatures, not by counting entries, so two
    /// sides that list the same methods in a different order call the same code.
    /// The answer is therefore equality of the two multisets: every entry of the
    /// table names a different method of `own`, and the counts agree.
    ///
    /// A table that names one signature twice - a table written by hand, or one
    /// assembled from another declaration - is refused even when the same
    /// signature appears in `own`, because the other entry has displaced a
    /// method: the call for that method would find nothing.
    pub fn declarations_match(&self, own: &[u64]) -> bool {
        let count = self.slots();
        if count != own.len() as u64 {
            return false;
        }
        if own.len() > MAX_SLOTS as usize {
            return false;
        }
        // The header folds the same list of method identifiers this declaration
        // folds, so two equal sets necessarily agree on it. A difference is
        // therefore a refusal in one comparison, which is the case a host sees
        // when it is handed an object built from another declaration.
        if self.signature() != crate::object::combined_signature(own) {
            return false;
        }
        // Each entry takes one position of `own`, and a position is taken once.
        // The count equality above then makes the two lists the same multiset.
        let mut taken = [false; MAX_SLOTS as usize];
        let mut index = 0;
        while index < count {
            let signature = match self.slot_signature(index) {
                Some(signature) => signature,
                None => return false,
            };
            let mut position = 0;
            let mut found = false;
            while position < own.len() {
                if own[position] == signature && !taken[position] {
                    taken[position] = true;
                    found = true;
                    break;
                }
                position += 1;
            }
            if !found {
                return false;
            }
            index += 1;
        }
        true
    }

    /// Reports whether a table names one signature more than once.
    ///
    /// A declaration names each method once, so a table that repeats a signature
    /// has displaced a method: the call for the displaced one finds nothing. Such
    /// a table is not a revision of any declaration, whichever direction the
    /// comparison would take.
    ///
    /// The entries of a long table generated here are in ascending order, so a
    /// repeat sits next to its twin and one pass finds it. A table from elsewhere
    /// need not be ordered, and is compared pairwise instead; that fallback costs
    /// one comparison per pair, which is why it only runs when the entries are not
    /// ordered.
    fn repeats_a_signature(&self) -> bool {
        let count = self.slots();
        if count < 2 {
            return false;
        }
        let mut ordered = true;
        let mut index = 0;
        while index + 1 < count {
            let (Some(left), Some(right)) = (self.slot_signature(index), self.slot_signature(index + 1))
            else {
                // An entry that cannot be read names nothing this build can
                // compare, so the table is refused rather than trusted.
                return true;
            };
            if left == right {
                return true;
            }
            if left > right {
                ordered = false;
            }
            index += 1;
        }
        if ordered {
            return false;
        }
        let mut first = 0;
        while first < count {
            let Some(signature) = self.slot_signature(first) else {
                return true;
            };
            let mut second = first + 1;
            while second < count {
                if self.slot_signature(second) == Some(signature) {
                    return true;
                }
                second += 1;
            }
            first += 1;
        }
        false
    }

    /// Reports whether a table and a declaration are revisions of one another.
    ///
    /// The boundary has no nominal identity for a declaration: two sides built
    /// independently agree by spelling out the same methods. Compatibility is
    /// therefore structural - one list of methods has to contain the other - and
    /// a table that shares no whole list with `own` belongs to another
    /// declaration, which is refused before any of its entries is used.
    ///
    /// A table that declares fewer methods is the older revision and one that
    /// declares more is the newer one; both are accepted, because every call
    /// either finds the signature it names or reports the absence at the call. A
    /// table that names one signature twice is neither: it is refused, because it
    /// has displaced a method.
    pub fn declarations_overlap(&self, own: &[u64]) -> bool {
        let count = self.slots();
        // A table and a declaration that names no methods describe nothing to
        // compare: the only declaration such a table is a revision of is the
        // one that also names nothing.
        if own.is_empty() {
            return count == 0;
        }
        // A table that declares no methods shares no method with any declaration
        // that has one, so it is not an older revision of it: it is something
        // else, and every call through it would report an absent slot.
        if count == 0 {
            return false;
        }
        // Containing every method this build declares is not enough: a repeat
        // means some method of `own` has no entry of its own.
        if self.repeats_a_signature() {
            return false;
        }
        // Two equal sets agree on the header signature. The fold is a summary of
        // the list, not a proof of it - two different lists can answer one
        // number - so the entries decide. The counts agree and no signature
        // repeats, so every method of `own` having an entry makes the two lists
        // the same set.
        if count == own.len() as u64 && self.signature() == crate::object::combined_signature(own) {
            return own.iter().all(|signature| self.slot(*signature).is_some());
        }
        let mut peer_inside = true;
        let mut index = 0;
        while index < count {
            match self.slot_signature(index) {
                Some(signature) if own.contains(&signature) => {}
                _ => {
                    peer_inside = false;
                    break;
                }
            }
            index += 1;
        }
        if peer_inside {
            return true;
        }
        // The other direction: an older declaration reading a newer object has
        // to find every method it declares in the peer's table.
        for signature in own {
            let mut found = false;
            let mut scan = 0;
            while scan < count {
                if self.slot_signature(scan) == Some(*signature) {
                    found = true;
                    break;
                }
                scan += 1;
            }
            if !found {
                return false;
            }
        }
        true
    }

    /// Releases the storage without running the value's destructor.
    ///
    /// This is for a value whose contents were moved out by the callee, which is
    /// what a method that takes the receiver by value does. Nothing in the block
    /// is dropped afterwards, so this must only be called when no initialised
    /// value is left inside it.
    ///
    /// # Safety
    ///
    /// The value inside the block must have been moved out already.
    pub unsafe fn forget_value(self) {
        if self.vtable.is_null() {
            return;
        }
        let size = self.size();
        let align = self.align();
        let (data, _table, allocator) = unsafe { self.into_raw_parts() };
        // SAFETY: the block came from `allocator` with this layout, and the
        // caller states that no initialised value is left inside it.
        unsafe { allocator.deallocate(data, size, align) };
    }

    /// The size of the value in bytes, or zero when the value carries no
    /// object.
    pub fn size(&self) -> u64 {
        if self.vtable.is_null() {
            0
        } else {
            // SAFETY: a non-null table stays alive for the value's lifetime.
            unsafe { (*self.vtable).size }
        }
    }

    /// The alignment of the value in bytes, or zero when the value carries no
    /// object.
    pub fn align(&self) -> u64 {
        if self.vtable.is_null() {
            0
        } else {
            // SAFETY: a non-null table stays alive for the value's lifetime.
            unsafe { (*self.vtable).align }
        }
    }

    /// Reports whether the value carries an object with `type_id`.
    pub fn has_type_id(&self, type_id: u64) -> bool {
        !self.is_empty() && self.type_id() == type_id
    }
}

impl Drop for StableDyn {
    fn drop(&mut self) {
        // A table this build cannot check says nothing about the block: the
        // address, the size and the alignment cannot be trusted, so neither the
        // table's `drop` nor the allocator is called. The storage is left to
        // whoever produced the value.
        if !self.is_releasable() {
            return;
        }
        if self.vtable.is_null() {
            return;
        }
        // SAFETY: the table and the value were built together and the table
        // outlives the value.
        let prefix = unsafe { &*self.vtable };
        if !self.data.is_null() {
            // SAFETY: `drop` releases a value of exactly this table's type, and
            // `validate` refused a table that records none.
            if let Some(drop) = prefix.drop {
                unsafe { drop(self.data) };
            }
            // SAFETY: the block was produced by this allocator with the size
            // and alignment recorded in the table.
            unsafe {
                self.allocator
                    .deallocate(self.data, prefix.size, prefix.align)
            };
        }
        self.data = ptr::null_mut();
        self.vtable = ptr::null();
    }
}

impl crate::zeroize::Wipe for StableDyn {
    /// Releases the value, overwrites the bytes it occupied, and gives the block
    /// back.
    ///
    /// The bytes are overwritten after the destructor runs, because a destructor
    /// reads the value it releases. What the allocator does with the block after
    /// it is returned is outside this crate's control, which is the same limit
    /// every other wipe here has.
    ///
    /// Only the object's own block is overwritten. Values the implementation
    /// holds elsewhere - a [`StableString`](crate::string::StableString) or a
    /// [`StableVec`](crate::vec::StableVec) inside it, which live in blocks of
    /// their own - are released by the destructor, and a release does not
    /// overwrite what it gives back to the allocator. The table has no wipe slot,
    /// so this build cannot ask the implementation to wipe them; an
    /// implementation that keeps secrets in nested storage wipes them itself
    /// before it releases them.
    fn wipe_in_place(&mut self) {
        // The same test as the destructor: a table this build cannot check does
        // not describe a value whose bytes it may overwrite.
        if !self.is_releasable() {
            return;
        }
        if self.vtable.is_null() {
            return;
        }
        // SAFETY: the table and the value were built together and the table
        // outlives the value.
        let prefix = unsafe { &*self.vtable };
        if !self.data.is_null() {
            // SAFETY: `drop` releases a value of exactly this table's type, and
            // `validate` refused a table that records none.
            if let Some(drop) = prefix.drop {
                unsafe { drop(self.data) };
            }
            // SAFETY: the block is the one the value occupied, and it is
            // writable for the whole size the table records.
            let _ = unsafe {
                crate::zeroize::volatile_zero_raw(self.data.cast(), prefix.size, 1)
            };
            // SAFETY: the block was produced by this allocator with the size and
            // alignment recorded in the table.
            unsafe {
                self.allocator
                    .deallocate(self.data, prefix.size, prefix.align)
            };
        }
        self.data = ptr::null_mut();
        self.vtable = ptr::null();
    }
}

/// Checks a handle a declaration generated.
///
/// The layout the table describes is checked first, and then that the table
/// shares the methods this declaration compiled. The second check is what stands
/// between a library's table and a host's call when the value arrives as an
/// argument or as the answer to a method: the accessor checks every value it
/// hands out, and a handle that names another declaration would otherwise be
/// accepted on its table header alone and fail at the first call.
///
/// The test is [`StableDyn::declarations_overlap`] rather than an exact match,
/// because a peer is allowed to be another revision of the declaration.
///
/// # Errors
///
/// [`AbiStatus::NullPointer`] when the handle names no object, whatever the
/// declaration: a value that carries no table has nothing to call.
pub fn validate_handle(value: &StableDyn, own: &[u64]) -> AbiResult<()> {
    value.validate()?;
    if value.is_empty() {
        return Err(AbiError::with_message(
            AbiStatus::NullPointer,
            "the handle carries no object",
        ));
    }
    if !value.declarations_overlap(own) {
        return Err(AbiError::with_message(
            AbiStatus::InvalidDescriptor,
            "the object was built from a different trait declaration",
        ));
    }
    Ok(())
}

impl fmt::Debug for StableDyn {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StableDyn")
            .field("type_id", &self.type_id())
            .field("size", &self.size())
            .field("align", &self.align())
            .finish_non_exhaustive()
    }
}

impl Default for StableDyn {
    fn default() -> Self {
        Self::empty()
    }
}

// SAFETY: the handle has a fixed layout, and dropping it releases the value
// through the table and the allocator it records.
unsafe impl StableElement for StableDyn {
    unsafe fn drop_in_place(ptr: *mut Self) {
        // SAFETY: the caller guarantees an initialised value.
        unsafe { core::ptr::drop_in_place(ptr) }
    }
}

/// Writes `value` into freshly allocated storage and returns the address.
///
/// # Safety
///
/// The caller must release the returned address through `allocator` with the
/// layout of `T`, after running the value's destructor.
pub unsafe fn allocate_value<T>(value: T, allocator: AbiAllocator) -> AbiResult<*mut u8> {
    let size = size_of::<T>() as u64;
    let align = align_of::<T>() as u64;
    let storage = allocator.try_allocate(size, align)?;
    // SAFETY: the block can hold one `T`, and a zero-sized `T` accepts the
    // aligned dangling address returned for zero-sized requests.
    unsafe { storage.cast::<T>().write(value) };
    Ok(storage)
}

/// A process-wide cache of virtual tables, keyed by concrete type and by the
/// identifier the caller chose.
///
/// A table depends only on the concrete type and the identifier, never on the
/// allocator or on a particular value, so one table serves every instance. The
/// generated constructor keeps one cache per trait.
///
/// A table stays until [`release_cached_tables`] frees it, which is what makes
/// it usable from every value of the type and what a module that is about to be
/// unloaded has to call.
///
/// The cache is keyed by the concrete implementation type, not by the identifier
/// that type was registered with: a type has one set of methods, so it has one
/// table, and registering the same type under a second identifier is refused
/// instead of leaking another table. Identity of an individual value belongs to
/// [`crate::handle::Handle`], not to the table.
///
/// A registration is a cold path: it takes a lock, builds the table and
/// publishes an immutable copy of the entries. Every construction after it reads
/// the published copy without locking, which is why building a value does not
/// serialize the threads that build values of one trait.
pub struct VTableCache {
    entries: Once<SpinLock<Vec<(TypeId, CachedTable)>>>,
    snapshot: AtomicPtr<Snapshot>,
    registered: AtomicBool,
    next: AtomicPtr<VTableCache>,
}

impl fmt::Debug for VTableCache {
    // The fields hold addresses of blocks and of other caches, and a log that
    // printed them would hand out the layout of the process. The debug form
    // reports the shape instead.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VTableCache")
            .field("registered", &self.registered.load(Ordering::Relaxed))
            .field(
                "published",
                &!self.snapshot.load(Ordering::Relaxed).is_null(),
            )
            .finish_non_exhaustive()
    }
}

/// A table built for a cache, with the function that frees it.
///
/// A cache keeps one table per implementation type for as long as the module
/// lives, so it cannot free a table on its own: the size and the alignment the
/// table was allocated with belong to the type the build site wrote. The build
/// site hands both over together, which is what lets
/// [`release_cached_tables`] give the memory back.
pub struct BuiltTable {
    table: *const VTablePrefix,
    release: unsafe fn(*const VTablePrefix),
}

impl BuiltTable {
    /// Pairs a table with the function that frees it.
    ///
    /// # Safety
    ///
    /// `table` must point at a table allocated for the type `release` frees, and
    /// `release` must free exactly that allocation, once, with the layout it was
    /// allocated with.
    pub const unsafe fn new(
        table: *const VTablePrefix,
        release: unsafe fn(*const VTablePrefix),
    ) -> Self {
        Self { table, release }
    }
}

impl fmt::Debug for BuiltTable {
    // The address is the layout of this process rather than anything about the
    // table, so it is not printed; what a reader needs to know is that the
    // value is a table and that something frees it.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BuiltTable")
            .field("has_table", &!self.table.is_null())
            .finish_non_exhaustive()
    }
}

/// One table a cache holds.
///
/// The address is a pointer rather than a number: a table built here is reached
/// through the pointer it was allocated as, which is what keeps the address
/// inside the allocation the allocator handed out.
struct CachedTable {
    table: *const VTablePrefix,
    release: unsafe fn(*const VTablePrefix),
}

// SAFETY: the pointer names a table that is built once and never moved or
// written again, and the cache only reads it. Sending the record to another
// thread therefore sends the address of an immutable block.
unsafe impl Send for CachedTable {}

// SAFETY: two threads may read the same address; neither writes it.
unsafe impl Sync for CachedTable {}

/// One entry of a published copy, as a reader sees it.
struct CacheEntry {
    implementation: TypeId,
    table: *const VTablePrefix,
}

/// A copy of a cache's entries that is never changed after it is published.
///
/// Building a value of a type the cache already holds is the common case, and it
/// used to take the cache's lock. A reader now loads this copy and scans it
/// without locking: the copy is immutable, and it is freed only by
/// [`release_cached_tables`], which requires that no thread is inside a
/// construction. Each registration publishes a new copy and keeps the previous
/// one in this chain, so a reader that loaded the older one still finds memory
/// it may read.
///
/// The chain is the price of that read: `N` implementation types retain
/// `1 + 2 + ... + N` entries, each one a type identifier and a table address.
/// It grows with the number of types and not with the number of values built,
/// and [`release_cached_tables`] gives all of it back. Freeing a copy earlier
/// would need every reader to announce that it is inside a scan, and that
/// announcement is a read-modify-write on a line all threads share - the cost
/// this design exists to avoid on the path that builds a value.
struct Snapshot {
    entries: crate::heap::boxed::Box<[CacheEntry]>,
    older: *mut Snapshot,
}

/// The caches that have built a table, linked through the caches themselves.
///
/// A release has to reach every cache a module holds, and the caches are
/// separate statics generated once per trait, so each one joins this list the
/// first time it is used. The link lives in the cache, so joining costs no
/// allocation: a module that is loaded and unloaded many times must not leave a
/// block behind per load.
static CACHES: AtomicPtr<VTableCache> = AtomicPtr::new(ptr::null_mut());

/// Puts `cache` at the head of [`CACHES`].
///
/// Nothing is ever removed from the list, so a cache that was released and is
/// used again is still found by the next release.
fn register(cache: &'static VTableCache) {
    let mut head = CACHES.load(Ordering::Acquire);
    loop {
        cache.next.store(head, Ordering::Release);
        match CACHES.compare_exchange_weak(
            head,
            cache as *const VTableCache as *mut VTableCache,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => return,
            Err(current) => head = current,
        }
    }
}

impl VTableCache {
    /// An empty cache.
    pub const fn new() -> Self {
        Self {
            entries: Once::new(),
            snapshot: AtomicPtr::new(ptr::null_mut()),
            registered: AtomicBool::new(false),
            next: AtomicPtr::new(ptr::null_mut()),
        }
    }

    /// The table a published copy holds for `implementation`, if any.
    ///
    /// This is the path every value of a type the cache already holds takes: it
    /// loads one pointer and searches an immutable, sorted copy, so it takes no
    /// lock.
    fn cached(&self, implementation: TypeId) -> Option<*const VTablePrefix> {
        let snapshot = self.snapshot.load(Ordering::Acquire);
        if snapshot.is_null() {
            return None;
        }
        // SAFETY: a published copy lives until `release_cached_tables`, which
        // requires that no thread is inside a construction.
        let entries = unsafe { (*snapshot).entries.as_ref() };
        entries
            .binary_search_by(|entry| entry.implementation.cmp(&implementation))
            .ok()
            .map(|index| entries[index].table)
    }

    /// Publishes a copy of `entries` for the readers that take no lock.
    ///
    /// # Safety
    ///
    /// The caller holds this cache's lock, so no other thread publishes at the
    /// same time, and every table in `entries` stays valid until
    /// [`release_cached_tables`].
    unsafe fn publish(&self, entries: &[(TypeId, CachedTable)]) {
        let mut published = Vec::with_capacity(entries.len());
        for (implementation, cached) in entries {
            published.push(CacheEntry {
                implementation: *implementation,
                table: cached.table,
            });
        }
        // The copy is read without a lock, so it is ordered once here: every
        // later lookup is a binary search over it.
        published.sort_unstable_by_key(|entry| entry.implementation);
        let snapshot = Box::into_raw(Box::new(Snapshot {
            entries: published.into_boxed_slice(),
            older: self.snapshot.load(Ordering::Acquire),
        }));
        self.snapshot.store(snapshot, Ordering::Release);
    }

    /// Returns the table for the implementation type in `key`.
    ///
    /// `key.1` is recorded in the table the first time; a later call for the
    /// same implementation type with a different identifier is a programming
    /// error: one type has one method list, so one table, and a second
    /// identifier would otherwise leak a table per instance. Instance identity
    /// belongs to [`crate::handle::Handle`].
    ///
    /// # Panics
    ///
    /// Panics when the implementation type was already registered under a
    /// different identifier.
    ///
    /// # Safety
    ///
    /// The table `build` returns must stay valid until [`release_cached_tables`]
    /// frees it, through the function it carries: it is cached and reached from
    /// every value built afterwards, including values that live on another
    /// thread. A table allocated on the stack, or freed by anything other than
    /// that function, violates this. The cache must itself live for the rest of
    /// the process, which a `static` does.
    pub unsafe fn get_or_insert(
        &'static self,
        key: (TypeId, u64),
        build: impl FnOnce() -> BuiltTable,
    ) -> *const VTablePrefix {
        let (implementation, type_id) = key;
        // A cache joins the list before it builds its first table, so a release
        // never misses one that is about to exist.
        if !self.registered.swap(true, Ordering::AcqRel) {
            register(self);
        }
        // The common case takes no lock: the published copy is immutable, and a
        // table is built once per implementation type.
        if let Some(table) = self.cached(implementation) {
            // SAFETY: a cached table stays valid until `release_cached_tables`
            // frees it, and that call requires nothing to reach it afterwards.
            let recorded = unsafe { (*table).type_id };
            assert!(
                recorded == type_id,
                "one implementation type cannot be registered under two identifiers"
            );
            return table;
        }
        let entries = self.entries.get_or_init(|| SpinLock::new(Vec::new()));
        let mut entries = entries.lock();

        // The list holds one entry per implementation type, so a linear scan
        // costs less than a hashed lookup. A table is built at most once per
        // type; the lock is held while it is built so a second thread waits for
        // the same table instead of allocating one that would be dropped.
        if let Some((_, cached)) = entries.iter().find(|(key, _)| *key == implementation) {
            let table = cached.table;
            // SAFETY: a cached table stays valid until `release_cached_tables`
            // frees it, and that call requires nothing to reach it afterwards.
            let recorded = unsafe { (*table).type_id };
            assert!(
                recorded == type_id,
                "one implementation type cannot be registered under two identifiers"
            );
            return table;
        }

        let built = build();
        entries.push((
            implementation,
            CachedTable {
                table: built.table,
                release: built.release,
            },
        ));
        // SAFETY: the lock is held, and the table was just built to live until a
        // release.
        unsafe { self.publish(&entries) };
        built.table
    }

    /// Frees the tables this cache holds, and answers how many it freed.
    ///
    /// # Safety
    ///
    /// The caller guarantees that no value built here is alive and that no
    /// thread is inside a call through one of these tables.
    unsafe fn release_tables(&self) -> usize {
        let entries = self.entries.get_or_init(|| SpinLock::new(Vec::new()));
        let mut entries = entries.lock();
        let mut released = 0;
        for (_, cached) in entries.drain(..) {
            // SAFETY: the caller guarantees nothing reaches this table.
            unsafe { (cached.release)(cached.table) };
            released += 1;
        }
        // The list's own storage is process memory too, and an empty list with
        // capacity would keep it after the tables are gone.
        entries.shrink_to_fit();
        // The published copies are process memory as well: they hold the
        // addresses that were just freed, and nothing may reach them now.
        let mut snapshot = self.snapshot.swap(ptr::null_mut(), Ordering::AcqRel);
        while !snapshot.is_null() {
            // SAFETY: every address in this chain was published by `publish`,
            // and the caller guarantees no one is reading one.
            let node = unsafe { Box::from_raw(snapshot) };
            snapshot = node.older;
            drop(node);
        }
        released
    }
}

/// Frees every table the caches built, and answers how many it freed.
///
/// A table is built once per implementation type and then shared by every value
/// of that type, so it outlives all of them. That is right for a module that
/// stays loaded and wrong for one that is unloaded: the allocator is the
/// process's, so the memory stays allocated and nothing can reach it once the
/// module is gone. Calling this from a shutdown hook, before the host unloads
/// the module, gives it back; the next load builds its own tables.
///
/// Calling it again is harmless: a second call frees nothing and answers zero.
/// A cache that is used after being released builds a new table.
///
/// # Safety
///
/// No value built from a cached table may be alive and no thread may be inside a
/// call through one, because the table is freed here. That is the same
/// obligation as unloading the module the tables were built from.
pub unsafe fn release_cached_tables() -> usize {
    let mut released = 0;
    let mut node = CACHES.load(Ordering::Acquire);
    while !node.is_null() {
        // SAFETY: every address in the list was pushed from a `&'static` cache,
        // and the list never drops one.
        let cache = unsafe { &*node };
        // SAFETY: the caller guarantees nothing reaches a table a cache holds.
        released += unsafe { cache.release_tables() };
        node = cache.next.load(Ordering::Acquire);
    }
    released
}

impl Default for VTableCache {
    fn default() -> Self {
        Self::new()
    }
}

/// The identifier of a whole method list, folded from one identifier per
/// method.
///
/// Folding the per-method identifiers, rather than hashing the declaration text
/// again, is what makes the number describe the slots the build actually
/// contains: a method that `#[cfg]` removed contributes no identifier, so two
/// builds of the same source with different features answer with different
/// numbers.
///
/// The fold is commutative, so the order the methods are listed in does not
/// change the answer. A call finds its slot by comparing signatures, never by
/// counting entries, so two sides that list the same methods in a different order
/// call the same code; a number that said otherwise would disagree with the
/// adoption rule in [`StableDyn::declarations_match`].
pub const fn combined_signature(signatures: &[u64]) -> u64 {
    let mut sum: u64 = 0;
    let mut twist: u64 = 0;
    let mut index = 0;
    while index < signatures.len() {
        let mixed = mix_identifier(signatures[index]);
        sum = sum.wrapping_add(mixed);
        twist ^= mixed.rotate_left(17);
        index += 1;
    }
    (LAYOUT_REVISION ^ 0xcbf2_9ce4_8422_2325)
        ^ (signatures.len() as u64)
        ^ sum
        ^ twist
}

/// Spreads the bits of one identifier so that adding several of them does not
/// keep the low bits of each separate.
const fn mix_identifier(identifier: u64) -> u64 {
    let mut hash = identifier ^ 0xcbf2_9ce4_8422_2325;
    hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    hash ^ (hash >> 29)
}

/// Fails the build when a method's return type cannot be one slot.
///
/// `declaration` is the text of the return type as it was written, which the
/// generated code records in a `const` item. Three spellings stop there:
///
/// - a type that names a lifetime. A slot lends its value only for the call, so
///   a returned borrow would have to outlive the storage it points into, and the
///   generated trampoline has no input lifetime for the borrow to elide to.
///   `'static` is refused as well: a value that points into the library dangles
///   as soon as the library is unloaded, which the linker does not tell the type
///   system about. Return an owned value (`StableString`, `StableVec`,
///   `StableBox`, ...) or have the caller pass the storage it keeps.
/// - a borrowed type with the lifetime left out (`StableStr`, `StableSlice<u8>`,
///   `StableSliceMut<u8>`), which is the same mistake written in the shortest
///   form: the slot's value cannot name how long the storage lives.
/// - `Self`. The slot returns a value of a boundary type, and `Self` in the
///   declaration names the implementation type, which the caller does not know.
///
/// Without this check these spellings fail later, inside generated code, with an
/// error that does not name the declaration that caused it.
///
/// The check reads the text of the declaration rather than its meaning, so it is
/// a guard against the common mistakes and not a proof: a type alias to a
/// borrowed type (`type Text = StableStr<'static>;`) keeps its borrow out of
/// sight, and a `'` that belongs to a const-generic argument reads as a lifetime.
/// What it cannot see, the borrow checker refuses where the value would have to
/// outlive the call.
///
/// # Panics
///
/// Panics when `declaration` names a lifetime, names a borrowed type without
/// one, or is `Self`.
pub const fn require_owned_return(declaration: &str) {
    let bytes = declaration.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'\'' {
            panic!(
                "a method's return type names a lifetime: a slot lends its value only for the call, so the returned borrow would outlive the storage. `'static` is refused too, because a value that points into the library dangles once the library is unloaded. Return an owned value (`StableString`, `StableVec`, `StableBox`, ...) or have the caller pass the storage it keeps."
            );
        }
        index += 1;
    }
    // Outer parentheses and the spaces around them name the same type, so both
    // are removed before the name is compared: `-> (Self)` or `-> ( Self )`
    // would otherwise pass and fail later, inside generated code.
    let mut start = 0;
    let mut end = bytes.len();
    loop {
        while start < end && bytes[start] == b' ' {
            start += 1;
        }
        while end > start && bytes[end - 1] == b' ' {
            end -= 1;
        }
        if end > start + 1 && bytes[start] == b'(' && bytes[end - 1] == b')' {
            start += 1;
            end -= 1;
            continue;
        }
        break;
    }
    if end - start == 4
        && bytes[start] == b'S'
        && bytes[start + 1] == b'e'
        && bytes[start + 2] == b'l'
        && bytes[start + 3] == b'f'
    {
        panic!(
            "a method cannot return `Self`: the slot returns a value of a boundary type, and `Self` names the implementation type, which the caller does not know. Name the boundary type the method returns."
        );
    }
    // The same mistake in its shortest form: a borrowed type written without the
    // lifetime it needs. The name before the first `<` is what decides.
    const BORROWED: [&str; 7] = [
        "StableStr",
        "StableSlice",
        "StableSliceMut",
        "StableCowStr",
        "StableCowSlice",
        "StableOsStr",
        "StablePath",
    ];
    let mut base_end = start;
    while base_end < end && bytes[base_end] != b'<' {
        base_end += 1;
    }
    // Spaces may separate the name from the arguments (`StableSlice < u8 >`),
    // and they are not part of the name.
    while base_end > start && bytes[base_end - 1] == b' ' {
        base_end -= 1;
    }
    // The name of the type is its last path segment, so a declaration that
    // writes `xt_stable_abi::StableStr` is refused under the same rule as one
    // that writes `StableStr` instead of failing later with a lifetime error.
    let mut name_start = base_end;
    while name_start > start && is_ident_continue(bytes[name_start - 1]) {
        name_start -= 1;
    }
    let mut borrowed_index = 0;
    while borrowed_index < BORROWED.len() {
        let candidate = BORROWED[borrowed_index].as_bytes();
        if candidate.len() == base_end - name_start {
            let mut offset = 0;
            let mut same = true;
            while offset < candidate.len() {
                if bytes[name_start + offset] != candidate[offset] {
                    same = false;
                    break;
                }
                offset += 1;
            }
            if same {
                panic!(
                    "a method cannot return a borrowed type: a slot lends its value only for the call, so the returned borrow would outlive the storage. Return an owned value (`StableString`, `StableVec`, `StableBox`, ...) or have the caller pass the storage it keeps."
                );
            }
        }
        borrowed_index += 1;
    }
    // The same mistake nested inside another type is the same mistake in a
    // longer spelling: `StableOption<StableStr>` returns a borrow the slot cannot
    // lend, and the name that decides above is the outer one. The declaration is
    // therefore searched for these names wherever they appear as a whole
    // identifier, which is how a nested one is refused here rather than later by
    // the borrow checker's lifetime error.
    let mut borrowed_index = 0;
    while borrowed_index < BORROWED.len() {
        let candidate = BORROWED[borrowed_index].as_bytes();
        let mut offset = 0;
        while offset + candidate.len() <= bytes.len() {
            let mut same = true;
            let mut step = 0;
            while step < candidate.len() {
                if bytes[offset + step] != candidate[step] {
                    same = false;
                    break;
                }
                step += 1;
            }
            if same {
                let starts_a_name = offset == 0 || !is_ident_continue(bytes[offset - 1]);
                let after = offset + candidate.len();
                let ends_a_name = after == bytes.len() || !is_ident_continue(bytes[after]);
                if starts_a_name && ends_a_name {
                    panic!(
                        "a method cannot return a borrowed type, even one inside another type: a slot lends its value only for the call, so the returned borrow would outlive the storage. Return an owned value (`StableString`, `StableVec`, `StableBox`, ...) or have the caller pass the storage it keeps."
                    );
                }
            }
            offset += 1;
        }
        borrowed_index += 1;
    }
}

/// A stable identifier derived from `name`.
///
/// The hash is FNV-1a over the UTF-8 bytes: it depends only on the text, so two
/// builds that write the same name produce the same number.
///
/// The hash names a type; it does not prove anything. FNV-1a is not collision
/// resistant, so the number is not an identity credential and must not be used
/// to decide whether a value is safe to accept.
pub const fn type_id_of(name: &str) -> u64 {
    let bytes = name.as_bytes();
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let mut index = 0;
    while index < bytes.len() {
        hash ^= bytes[index] as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        index += 1;
    }
    hash
}

/// A stable identifier for a piece of text.
///
/// This names text, not a type: a slot's identifier is built by [`slot_id`] from
/// the method's name and the numbers that name the types it mentions
/// ([`AbiIdentity`](crate::identity::AbiIdentity)), so how a declaration spells a
/// type does not change the identifier. This function is what names the method,
/// and what names a type a caller outside this crate declares.
///
/// The text enters character for character, so two different texts produce
/// different numbers unless the hash collides. A collision is not a proof of
/// anything, and this number is not an identity credential on its own: what a
/// table may be used for is decided by comparing the numbers of a declaration's
/// slots.
pub const fn declaration_id(text: &str) -> u64 {
    let bytes = text.as_bytes();
    let mut hash: u64 = LAYOUT_REVISION ^ 0xcbf2_9ce4_8422_2325;
    let mut index = 0;
    while index < bytes.len() {
        hash ^= bytes[index] as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        index += 1;
    }
    // Zero is the signature of the entry that ends a table, so no name may
    // produce it.
    if hash == 0 { 1 } else { hash }
}

const fn is_ident_start(byte: u8) -> bool {
    byte == b'_' || byte.is_ascii_alphabetic()
}

const fn is_ident_continue(byte: u8) -> bool {
    is_ident_start(byte) || byte.is_ascii_digit()
}
