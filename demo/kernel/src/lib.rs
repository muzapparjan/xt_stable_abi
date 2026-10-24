//! A text-analysis kernel.
//!
//! This crate is an ordinary Rust library: it knows nothing about stable
//! layouts, dynamic libraries or the ABI crate in this workspace. It exists to
//! be a realistic subject for one: the API crate next to it is designed from
//! what this library already does.
//!
//! The pieces are deliberately varied, because the API built on top of them has
//! to carry values of many shapes across a boundary:
//!
//! - [`text`] tokenises with an iterator that borrows the input, and folds case
//!   and whitespace without allocating for the borrowed form;
//! - [`metrics`] counts terms, keeps a bounded ranking with a generic heap and
//!   measures the distribution with a `dyn` trait;
//! - [`codec`] hashes, checksums, parses a cursor and represents a fixed-point
//!   number, with a `const`-built table and a trait with an associated type;
//! - [`graph`] walks a directed graph and detects cycles, with recursion, a
//!   queue and a union-find structure;
//! - [`state`] is a byte-level state machine whose transitions are a table of
//!   function pointers;
//! - [`bits`] is a transparent bit-set newtype with `const fn` constructors;
//! - [`engine`] ties them together, keeps a cache behind a mutex and counts what
//!   it has done.
//!
//! Everything here is public because the library has users; the API crate
//! decides which of these operations are promised to a foreign caller and which
//! stay inside.

#![deny(missing_docs)]
#![deny(rust_2018_idioms)]

pub mod bits;
pub mod codec;
pub mod engine;
pub mod error;
pub mod graph;
pub mod metrics;
pub mod state;
pub mod text;

pub use crate::engine::{Counters, Engine, Session, Summary};
pub use crate::error::{KernelError, KernelResult};
