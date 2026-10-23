//! Password interfaces (7-Zip `CPP/7zip/IPassword.h`). IID group 5.
//!
//! The handler obtains these by querying the open or extract callback.
//! The returned BSTR belongs to the caller; dropping the `BSTR` frees it.

use windows_core::{BSTR, HRESULT, IUnknown, IUnknown_Vtbl, OutRef, interface};

#[interface("23170F69-40C1-278A-0000-000500100000")]
pub unsafe trait ICryptoGetTextPassword: IUnknown {
    pub fn CryptoGetTextPassword(&self, password: OutRef<BSTR>) -> HRESULT;
}

/// `password_is_defined` is 0 when the user gave no password.
#[interface("23170F69-40C1-278A-0000-000500110000")]
pub unsafe trait ICryptoGetTextPassword2: IUnknown {
    pub fn CryptoGetTextPassword2(
        &self,
        password_is_defined: OutRef<i32>,
        password: OutRef<BSTR>,
    ) -> HRESULT;
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_core::Interface;

    #[test]
    fn iids_match_7zip() {
        for (iid, sub) in [
            (ICryptoGetTextPassword::IID, 0x10),
            (ICryptoGetTextPassword2::IID, 0x11),
        ] {
            assert_eq!(
                (iid.data1, iid.data2, iid.data3),
                (0x23170F69, 0x40C1, 0x278A)
            );
            assert_eq!(iid.data4, [0, 0, 0, 0x05, 0, sub, 0, 0]);
        }
    }
}
