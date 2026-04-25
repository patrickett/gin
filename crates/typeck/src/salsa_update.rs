//! [`salsa::Update`] for type IR so [`TyEnv`](crate::TyEnv) can be stored in Salsa memos.

use crate::{Ty, TyEnv};

unsafe impl salsa::Update for Ty {
    unsafe fn maybe_update(old_pointer: *mut Self, new_value: Self) -> bool {
        let old_ref: &mut Self = unsafe { &mut *old_pointer };
        if *old_ref != new_value {
            *old_ref = new_value;
            true
        } else {
            false
        }
    }
}

unsafe impl salsa::Update for TyEnv {
    unsafe fn maybe_update(old_pointer: *mut Self, new_value: Self) -> bool {
        let old_ref: &mut Self = unsafe { &mut *old_pointer };
        if *old_ref != new_value {
            *old_ref = new_value;
            true
        } else {
            false
        }
    }
}
