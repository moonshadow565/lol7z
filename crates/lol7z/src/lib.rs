//! The 7-Zip plugin DLL: every League of Legends format in one `lol7z.dll`, which goes
//! in the `Formats` folder next to `7z.dll`. The formats live in their own crates
//! ([`lol7z_wad`], [`lol7z_modpkg`], [`lol7z_wxy`]); this one only exports them, since
//! a DLL has one set of exports.

lol7z_api::formats! {
    update lol7z_wad::Wad,
    update lol7z_modpkg::Modpkg,
    read lol7z_wxy::Wxy,
}
