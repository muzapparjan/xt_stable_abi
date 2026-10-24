//! Groups of values with the fields laid out in order.
//!
//! A group carries two, three or four values, which is the widest a declaration
//! can enumerate at once; [`crate::array`] covers a run of one type.

use core::fmt;

use crate::vec::StableElement;

/// A pair whose fields sit side by side.
#[repr(C)]
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct StablePair<A, B> {
    /// The first value.
    pub left: A,
    /// The second value.
    pub right: B,
}

impl<A, B> StablePair<A, B> {
    /// Builds a pair.
    pub const fn new(left: A, right: B) -> Self {
        Self { left, right }
    }

    /// The two values as references.
    pub const fn as_ref(&self) -> (&A, &B) {
        (&self.left, &self.right)
    }

    /// The two values as mutable references.
    pub const fn as_mut(&mut self) -> (&mut A, &mut B) {
        (&mut self.left, &mut self.right)
    }

    /// Takes the pair apart.
    pub fn into_tuple(self) -> (A, B) {
        (self.left, self.right)
    }
}

impl<A: PartialOrd, B: PartialOrd> PartialOrd for StablePair<A, B> {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        match self.left.partial_cmp(&other.left) {
            Some(core::cmp::Ordering::Equal) => self.right.partial_cmp(&other.right),
            ordering => ordering,
        }
    }
}

impl<A: Ord, B: Ord> Ord for StablePair<A, B> {
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        self.left
            .cmp(&other.left)
            .then_with(|| self.right.cmp(&other.right))
    }
}

impl<A, B> From<(A, B)> for StablePair<A, B> {
    fn from(value: (A, B)) -> Self {
        Self::new(value.0, value.1)
    }
}

impl<A, B> From<StablePair<A, B>> for (A, B) {
    fn from(value: StablePair<A, B>) -> Self {
        value.into_tuple()
    }
}

// SAFETY: the pair is a `#[repr(C)]` struct of its two fields, and dropping it
// drops both of them in declaration order.
unsafe impl<A: StableElement, B: StableElement> StableElement for StablePair<A, B> {
    unsafe fn drop_in_place(ptr: *mut Self) {
        // SAFETY: the caller guarantees an initialised value.
        unsafe { core::ptr::drop_in_place(ptr) }
    }
}

impl<A, B> fmt::Debug for StablePair<A, B> {
    // The fields can be content a log must not repeat, so the debug form reports
    // the shape. `as_ref` prints the values themselves.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("StablePair { .. }")
    }
}

impl<A: crate::zeroize::Wipe, B: crate::zeroize::Wipe> crate::zeroize::Wipe
    for StablePair<A, B>
{
    fn wipe_in_place(&mut self) {
        self.left.wipe_in_place();
        self.right.wipe_in_place();
    }
}

/// A group of three values with the fields laid out in order.
#[repr(C)]
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct StableTriple<A, B, C> {
    /// The first value.
    pub first: A,
    /// The second value.
    pub second: B,
    /// The third value.
    pub third: C,
}

impl<A, B, C> StableTriple<A, B, C> {
    /// Builds a triple.
    pub const fn new(first: A, second: B, third: C) -> Self {
        Self {
            first,
            second,
            third,
        }
    }

    /// The three values as references.
    pub const fn as_ref(&self) -> (&A, &B, &C) {
        (&self.first, &self.second, &self.third)
    }

    /// Takes the triple apart.
    pub fn into_tuple(self) -> (A, B, C) {
        (self.first, self.second, self.third)
    }
}

impl<A: PartialOrd, B: PartialOrd, C: PartialOrd> PartialOrd for StableTriple<A, B, C> {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        match self.first.partial_cmp(&other.first) {
            Some(core::cmp::Ordering::Equal) => match self.second.partial_cmp(&other.second) {
                Some(core::cmp::Ordering::Equal) => self.third.partial_cmp(&other.third),
                ordering => ordering,
            },
            ordering => ordering,
        }
    }
}

impl<A: Ord, B: Ord, C: Ord> Ord for StableTriple<A, B, C> {
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        self.first
            .cmp(&other.first)
            .then_with(|| self.second.cmp(&other.second))
            .then_with(|| self.third.cmp(&other.third))
    }
}

impl<A, B, C> From<(A, B, C)> for StableTriple<A, B, C> {
    fn from(value: (A, B, C)) -> Self {
        Self::new(value.0, value.1, value.2)
    }
}

impl<A, B, C> From<StableTriple<A, B, C>> for (A, B, C) {
    fn from(value: StableTriple<A, B, C>) -> Self {
        value.into_tuple()
    }
}

impl<A, B, C> fmt::Debug for StableTriple<A, B, C> {
    // The fields can be content a log must not repeat, so the debug form reports
    // the shape. `as_ref` prints the values themselves.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("StableTriple { .. }")
    }
}

impl<A: crate::zeroize::Wipe, B: crate::zeroize::Wipe, C: crate::zeroize::Wipe>
    crate::zeroize::Wipe for StableTriple<A, B, C>
{
    fn wipe_in_place(&mut self) {
        self.first.wipe_in_place();
        self.second.wipe_in_place();
        self.third.wipe_in_place();
    }
}

// SAFETY: the triple is a `#[repr(C)]` struct of its three fields, and dropping
// it drops all of them in declaration order.
unsafe impl<A: StableElement, B: StableElement, C: StableElement> StableElement
    for StableTriple<A, B, C>
{
    unsafe fn drop_in_place(ptr: *mut Self) {
        // SAFETY: the caller guarantees an initialised value.
        unsafe { core::ptr::drop_in_place(ptr) }
    }
}

impl<A: fmt::Display, B: fmt::Display> fmt::Display for StablePair<A, B> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "({}, {})", self.left, self.right)
    }
}

impl<A: fmt::Display, B: fmt::Display, C: fmt::Display> fmt::Display for StableTriple<A, B, C> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "({}, {}, {})",
            self.first, self.second, self.third
        )
    }
}

impl<A: fmt::Display, B: fmt::Display, C: fmt::Display, D: fmt::Display> fmt::Display
    for StableQuad<A, B, C, D>
{
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "({}, {}, {}, {})",
            self.first, self.second, self.third, self.fourth
        )
    }
}

/// A group of four values with the fields laid out in order.
#[repr(C)]
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct StableQuad<A, B, C, D> {
    /// The first value.
    pub first: A,
    /// The second value.
    pub second: B,
    /// The third value.
    pub third: C,
    /// The fourth value.
    pub fourth: D,
}

impl<A, B, C, D> StableQuad<A, B, C, D> {
    /// Builds a group of four.
    pub const fn new(first: A, second: B, third: C, fourth: D) -> Self {
        Self {
            first,
            second,
            third,
            fourth,
        }
    }

    /// The four values as references.
    pub const fn as_ref(&self) -> (&A, &B, &C, &D) {
        (&self.first, &self.second, &self.third, &self.fourth)
    }

    /// Takes the group apart.
    pub fn into_tuple(self) -> (A, B, C, D) {
        (self.first, self.second, self.third, self.fourth)
    }
}

impl<A: PartialOrd, B: PartialOrd, C: PartialOrd, D: PartialOrd> PartialOrd
    for StableQuad<A, B, C, D>
{
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        match self.first.partial_cmp(&other.first) {
            Some(core::cmp::Ordering::Equal) => match self.second.partial_cmp(&other.second) {
                Some(core::cmp::Ordering::Equal) => {
                    match self.third.partial_cmp(&other.third) {
                        Some(core::cmp::Ordering::Equal) => self.fourth.partial_cmp(&other.fourth),
                        ordering => ordering,
                    }
                }
                ordering => ordering,
            },
            ordering => ordering,
        }
    }
}

impl<A: Ord, B: Ord, C: Ord, D: Ord> Ord for StableQuad<A, B, C, D> {
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        self.first
            .cmp(&other.first)
            .then_with(|| self.second.cmp(&other.second))
            .then_with(|| self.third.cmp(&other.third))
            .then_with(|| self.fourth.cmp(&other.fourth))
    }
}

impl<A, B, C, D> From<(A, B, C, D)> for StableQuad<A, B, C, D> {
    fn from(value: (A, B, C, D)) -> Self {
        Self::new(value.0, value.1, value.2, value.3)
    }
}

impl<A, B, C, D> From<StableQuad<A, B, C, D>> for (A, B, C, D) {
    fn from(value: StableQuad<A, B, C, D>) -> Self {
        value.into_tuple()
    }
}

impl<A, B, C, D> fmt::Debug for StableQuad<A, B, C, D> {
    // The fields can be content a log must not repeat, so the debug form reports
    // the shape. `as_ref` prints the values themselves.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("StableQuad { .. }")
    }
}

impl<
        A: crate::zeroize::Wipe,
        B: crate::zeroize::Wipe,
        C: crate::zeroize::Wipe,
        D: crate::zeroize::Wipe,
    > crate::zeroize::Wipe for StableQuad<A, B, C, D>
{
    fn wipe_in_place(&mut self) {
        self.first.wipe_in_place();
        self.second.wipe_in_place();
        self.third.wipe_in_place();
        self.fourth.wipe_in_place();
    }
}

// SAFETY: the group is a `#[repr(C)]` struct of its four fields, and dropping it
// drops all of them in declaration order.
unsafe impl<A: StableElement, B: StableElement, C: StableElement, D: StableElement>
    StableElement for StableQuad<A, B, C, D>
{
    unsafe fn drop_in_place(ptr: *mut Self) {
        // SAFETY: the caller guarantees an initialised value.
        unsafe { core::ptr::drop_in_place(ptr) }
    }
}
