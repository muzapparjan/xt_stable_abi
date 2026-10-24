//! What a `#[cfg]` on a method and on an enumeration variant does.

xt_stable_abi::stable_object! {
    /// A handle.
    pub struct CfgObject;
    /// The table.
    pub struct CfgVTable;
    /// The declaration.
    pub trait Cfg {
        /// A method this build does not have.
        #[cfg(any())]
        fn gone(&self) -> u64;
        /// The method this build keeps.
        fn kept(&self) -> u64;
    }
}

struct Impl;

impl Cfg for Impl {
    fn kept(&self) -> u64 {
        3
    }
}

#[test]
fn a_method_a_build_does_not_have_leaves_no_slot() {
    let object = CfgObject::abi_from_value(Impl);
    assert_eq!(unsafe { object.kept() }, 3);
    // The declaration records one method, not two.
    assert_eq!(CfgObject::ABI_SLOT_SIGNATURES.len(), 1);
    assert_eq!(object.abi_as_stable().slots(), 1);
    // A signature computed for the method that is not there names no slot.
    assert!(object.abi_as_stable().slot(1).is_none());
}
