//! The errors the kernel reports.
//!
//! The enum is non-exhaustive so a later revision can add a cause without
//! breaking a caller's match, and it implements [`std::error::Error`] so it can
//! be boxed next to any other failure.

use core::fmt;

/// A failure inside the kernel.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum KernelError {
    /// The input was empty where a value was required.
    EmptyInput,
    /// A byte did not belong to the syntax being read.
    UnexpectedByte {
        /// Where in the input the byte sat.
        offset: usize,
        /// The byte itself.
        byte: u8,
    },
    /// Parentheses or quotes were left open.
    Unbalanced {
        /// How deep the input still was when it ended.
        depth: i64,
    },
    /// A number could not be read, or carried more digits than the type holds.
    InvalidNumber,
    /// Arithmetic left the range of the type.
    Overflow,
    /// A division had a zero divisor.
    DivisionByZero,
    /// A graph edge named a node that does not exist.
    UnknownNode(u64),
    /// A graph contains a cycle where an order was required.
    GraphCycle {
        /// The number of nodes involved.
        nodes: usize,
    },
    /// A requested size was larger than the kernel accepts.
    CapacityExceeded {
        /// The bound the request passed.
        limit: usize,
        /// The size that was asked for.
        requested: usize,
    },
    /// An operator was not one this expression language knows.
    UnknownOperator(char),
}

impl fmt::Display for KernelError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyInput => formatter.write_str("the input is empty"),
            Self::UnexpectedByte { offset, byte } => {
                write!(formatter, "unexpected byte 0x{byte:02x} at offset {offset}")
            }
            Self::Unbalanced { depth } => write!(formatter, "unbalanced input at depth {depth}"),
            Self::InvalidNumber => formatter.write_str("the number could not be read"),
            Self::Overflow => formatter.write_str("the result does not fit the type"),
            Self::DivisionByZero => formatter.write_str("division by zero"),
            Self::UnknownNode(node) => write!(formatter, "unknown node {node}"),
            Self::GraphCycle { nodes } => write!(formatter, "the graph has a cycle over {nodes} nodes"),
            Self::CapacityExceeded { limit, requested } => {
                write!(formatter, "asked for {requested} items, the bound is {limit}")
            }
            Self::UnknownOperator(operator) => write!(formatter, "unknown operator {operator}"),
        }
    }
}

impl std::error::Error for KernelError {}

/// The result type the kernel answers with.
pub type KernelResult<T> = Result<T, KernelError>;
