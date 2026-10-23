use windows_core::{HRESULT, IUnknown, IUnknown_Vtbl, interface};

#[interface("23170F69-40C1-278A-0000-000000050000")]
pub unsafe trait IProgress: IUnknown {
    pub fn SetTotal(&self, total: u64) -> HRESULT;
    pub fn SetCompleted(&self, complete_value: Option<&u64>) -> HRESULT;
}
