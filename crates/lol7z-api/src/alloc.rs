//! [`TryAlloc`]: allocations that fail with `E_OUTOFMEMORY` instead of aborting 7-Zip.

use crate::error::Result;

pub trait TryAlloc: Sized {
    fn try_with_capacity(n: usize) -> Result<Self>;
}

impl<T> TryAlloc for Vec<T> {
    fn try_with_capacity(n: usize) -> Result<Self> {
        let mut v = Vec::new();
        v.try_reserve_exact(n)?;
        Ok(v)
    }
}
