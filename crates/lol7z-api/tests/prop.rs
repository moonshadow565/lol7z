use lol7z_api::sys::propvar::PropVariant;
use lol7z_api::{Prop, PropId, standard_vt};
use std::time::{Duration, UNIX_EPOCH};
use windows::Win32::System::Variant::{VT_BOOL, VT_BSTR, VT_EMPTY, VT_FILETIME, VT_UI4, VT_UI8};

/// Ticks between 1601-01-01 and 1970-01-01.
const UNIX_EPOCH_TICKS: u64 = 116_444_736_000_000_000;

/// `k7z_PROPID_To_VARTYPE` in 7-Zip's PropId.cpp, in order, comments stripped.
const TABLE: &str = "EMPTY UI4 UI4 BSTR BSTR BSTR BOOL UI8 UI8 UI4 FILETIME FILETIME FILETIME \
    BOOL BOOL BOOL BOOL BOOL UI4 UI4 BSTR BOOL BSTR BSTR BSTR BSTR BSTR UI8 BSTR UI8 BSTR UI8 \
    UI8 BSTR UI4 BOOL UI8 UI8 UI8 UI8 UI4 BOOL BOOL BSTR UI8 UI8 UI4 BSTR UI8 BSTR BSTR BSTR \
    UI4 UI4 BSTR BSTR UI8 UI8 UI4 BSTR BSTR BSTR BSTR BOOL BOOL BOOL BOOL BSTR BSTR BSTR UI8 \
    UI4 UI4 BSTR UI8 UI4 UI8 UI8 UI8 UI8 UI8 BSTR BSTR BSTR BOOL BOOL BOOL UI8 UI8 BSTR BSTR \
    UI8 UI8 BOOL BSTR BSTR BSTR BOOL FILETIME UI4 UI4 UI4 UI4 UI4 UI4";

#[test]
fn standard_vt_matches_7zip_table() {
    let table: Vec<_> = TABLE.split_whitespace().collect();
    assert_eq!(table.len() as u32, PropId::DevMinor.0 + 1);
    for (i, name) in table.iter().enumerate() {
        let want = match *name {
            "EMPTY" => VT_EMPTY,
            "UI4" => VT_UI4,
            "UI8" => VT_UI8,
            "BSTR" => VT_BSTR,
            "BOOL" => VT_BOOL,
            "FILETIME" => VT_FILETIME,
            _ => unreachable!(),
        };
        let id = PropId(i as u32);
        assert_eq!(standard_vt(id), want, "{id:?}");
    }
    assert_eq!(standard_vt(PropId::UserDefined), VT_EMPTY);
}

#[test]
fn variant_round_trip() {
    let props = [
        Prop::Bool(true),
        Prop::U32(7),
        Prop::U64(1 << 40),
        Prop::from("a/b.bin"),
        Prop::FileTime(UNIX_EPOCH_TICKS + 1),
    ];
    for p in props {
        assert_eq!(Prop::from_variant(&p.clone().into_variant()), Some(p));
    }
    assert_eq!(Prop::from_variant(&PropVariant::default()), None);
}

#[test]
fn system_time() {
    assert_eq!(Prop::time(UNIX_EPOCH), Prop::FileTime(UNIX_EPOCH_TICKS));
    let t = UNIX_EPOCH + Duration::from_micros(1);
    assert_eq!(Prop::time(t), Prop::FileTime(UNIX_EPOCH_TICKS + 10));
}
