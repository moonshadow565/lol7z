//! The grammar by example, hostile names, and every over-long name in the game's tables.

use lol7z_fold::{FoldError, MAX_DECODED, Split, decode, encode, split};

fn units(s: &str) -> usize {
    s.encode_utf16().count()
}

/// `«_skins_skin{N}»` for each N, in bytewise order.
fn skins(ns: impl Iterator<Item = u32>) -> String {
    let mut v: Vec<String> = ns.map(|n| n.to_string()).collect();
    v.sort();
    v.iter().map(|n| format!("_skins_skin{n}")).collect()
}

#[test]
fn sightward() {
    let name = format!("sightward_multi{}.bin", skins((0..=29).chain(32..=39)));
    let folded = "sightward_multi«_skins_skin{0…29,32…39}».bin";
    assert_eq!(decode(folded).unwrap(), name);
    assert_eq!(encode(&name).unwrap(), folded);
}

#[test]
fn ahri() {
    let name = format!("ahri_multi{}.bin", skins(0..=84));
    let folded = "ahri_multi«_skins_skin{0…84}».bin";
    assert_eq!(decode(folded).unwrap(), name);
    assert_eq!(encode(&name).unwrap(), folded);
}

/// Two names from Companions.wad.client, as they fold.
#[test]
fn pengu() {
    let a = "petpenguknight_multi«_{base,birthdaypengu}{_root,_tier{1…3}}_cosmic{_root}\
             _{esport,ruined}{_root,_tier1}_{set5launchbattlepass,set5updatebattlepass}\
             {_root,_tier{1…3}}_skins{_root,_skin{1…19,31…53,57…71}}\
             _spiritfairy{_root,_tier{1…3}}».bin";
    let b = "petpenguknight_multi«_{base,birthdaypengu}{_root,_tier{1…3}}\
             _{esport,ruined}{_root,_tier1}_{set5launchbattlepass,set5updatebattlepass}\
             {_root,_tier{1…3}}_skins{_root,_skin{1…19,31…53,57…71}}\
             _spiritfairy{_root,_tier{1…3}}».bin";
    for (folded, len) in [(a, 238), (b, 224)] {
        let name = decode(folded).unwrap();
        assert!(units(&name) > 255);
        assert_eq!(encode(&name).as_deref(), Some(folded));
        assert_eq!(units(folded), len);
    }
    assert!(
        decode(a)
            .unwrap()
            .contains("_cosmic_root_esport_root_esport_tier1_ruined_root")
    );
}

#[test]
fn digit_widths() {
    assert_eq!(decode("«{007…011}»").unwrap(), "007008009010011");
    assert_eq!(decode("«{8…11}»").unwrap(), "101189");
    assert_eq!(decode("«{0…10}»").unwrap(), "011023456789");
    assert_eq!(decode("«{2,10,1,2}»").unwrap(), "1102");
    assert_eq!(decode("«{07,7}»").unwrap(), "077");
    for bad in [
        "«{9…1}»",
        "«{01…100}»",
        "«{1…010}»",
        "«{10…1}»",
        "«{0123456789}»",
        "«a{1234567890}»",
    ] {
        assert_eq!(decode(bad), Err(FoldError::BadRange), "{bad}");
    }
}

/// What isn't a number set reads as alternatives, or not at all.
#[test]
fn near_number_sets() {
    assert_eq!(decode("«{1,2}{a,b}»").unwrap(), "1a1b2a2b");
    assert_eq!(decode("«{1,}»").unwrap(), "1");
    assert_eq!(decode("«{,1}»").unwrap(), "1");
    assert_eq!(decode("«{1…}»"), Err(FoldError::Syntax));
    assert_eq!(decode("«{…1}»"), Err(FoldError::Syntax));
}

#[test]
fn non_ascii_in_block() {
    assert_eq!(decode("«é{1,2}»").unwrap(), "é1é2");
    let name = format!("x{}.bin", skins(0..=3).replace("skins", "skïns"));
    let folded = encode(&name).unwrap();
    assert!(
        folded[folded.find('«').unwrap()..].contains('ï'),
        "{folded}"
    );
    assert_eq!(decode(&folded).unwrap(), name);
}

#[test]
fn encode_edge_cases() {
    for s in ["", ".", "abc", "0123456789", "é_ü_é_ü"] {
        if let Some(e) = encode(s) {
            assert_eq!(decode(&e).unwrap(), s, "{s}");
        }
    }
}

#[test]
fn products_and_nesting() {
    assert_eq!(decode("«{a,b}{c,d}»").unwrap(), "acadbcbd");
    assert_eq!(decode("«x{a{1,2},b}»").unwrap(), "xa1xa2xb");
    assert_eq!(
        decode("«_{a,b}{_c,_d{1…2}}»").unwrap(),
        "_a_c_a_d1_a_d2_b_c_b_d1_b_d2"
    );
    assert_eq!(decode("«x{y{z{1}}}»").unwrap(), "xyz1");
}

#[test]
fn written_order_and_empty() {
    assert_eq!(decode("«{b,a}»").unwrap(), "ba");
    assert_eq!(decode("a«»b").unwrap(), "ab");
    assert_eq!(decode("«x{,y}»").unwrap(), "xxy");
    assert_eq!(decode("«{,}»").unwrap(), "");
    assert_eq!(decode("«_s{}»").unwrap(), "_s");
    assert_eq!(decode("no block").unwrap(), "no block");
}

/// `…` is the range; three dots are only dots.
#[test]
fn dots_are_literal() {
    assert_eq!(decode("«{1...3}»").unwrap(), "1...3");
    assert_eq!(decode("«a.b{1…2}».c").unwrap(), "a.b1a.b2.c");
    assert_eq!(decode("«{1..3}»").unwrap(), "1..3");
}

#[test]
fn malformed() {
    for (bad, e) in [
        ("a«{1}»b«{2}»", FoldError::Blocks),
        ("a»", FoldError::Blocks),
        ("»a«{1}»", FoldError::Blocks),
        ("a«_s{1}»«", FoldError::Blocks),
        ("a«{1»", FoldError::Syntax),
        ("a«1}»", FoldError::Syntax),
        ("a«{a,b»", FoldError::Syntax),
        ("a«{a}", FoldError::Syntax),
        ("a«x…y»", FoldError::Syntax),
        ("a«x«y»", FoldError::Syntax),
    ] {
        assert_eq!(decode(bad), Err(e), "{bad}");
    }
}

#[test]
fn hostile() {
    let deep = format!("a«{}y{}»", "x{".repeat(50), "}".repeat(50));
    let wide = format!("a«_s{{{}}}»", ["0…999"; 60].join(","));
    let many = format!("a«_s{{0…99}}{}»", "{a}".repeat(100));
    for (s, e) in [
        ("a«_s{0…999999999}»".to_owned(), Some(FoldError::TooLarge)),
        ("a«_s{0…9999999999}»".into(), Some(FoldError::BadRange)),
        ("a«{0…9}{0…9}{0…9}{0…9}»".into(), None),
        (
            "a«{0…9}{0…9}{0…9}{0…9}{0…9}»".into(),
            Some(FoldError::TooLarge),
        ),
        (deep, Some(FoldError::TooDeep)),
        (
            format!("a«{}»", "{a,b}".repeat(13)),
            Some(FoldError::TooLarge),
        ),
        (wide, Some(FoldError::TooLarge)),
        (many, None),
        (
            format!("a«{}»", "{,}".repeat(17)),
            Some(FoldError::TooLarge),
        ),
    ] {
        let r = decode(&s);
        match e {
            Some(e) => assert_eq!(r, Err(e), "{s}"),
            None => assert!(r.unwrap().len() as u64 <= MAX_DECODED, "{s}"),
        }
    }
    // Eight groups deep is the most.
    let over = "x{".repeat(9) + &"}".repeat(9);
    assert_eq!(decode(&format!("«{over}»")), Err(FoldError::TooDeep));
    let ok = "x{".repeat(8) + "y" + &"}".repeat(8);
    assert_eq!(decode(&format!("«{ok}»")).unwrap(), "xxxxxxxxy");
}

/// Random names from the grammar's characters never panic, and never come out longer
/// than the bound.
#[test]
fn fuzz() {
    let alphabet: Vec<char> = "ab_.0129{},«»…".chars().collect();
    let mut x = 0x2545_f491_4f6c_dd1du64;
    let mut next = |n: usize| {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        (x % n as u64) as usize
    };
    for _ in 0..200_000 {
        let len = next(41);
        let mut s: String = (0..len).map(|_| alphabet[next(alphabet.len())]).collect();
        if next(10) < 7 {
            s = format!("x«{s}».bin");
        }
        if let Ok(d) = decode(&s) {
            assert!(d.len() as u64 <= MAX_DECODED + s.len() as u64, "{s}");
        }
        if let Some(e) = encode(&s) {
            assert_eq!(decode(&e).unwrap(), s);
        }
    }
}

#[test]
fn encode_refuses() {
    // Nothing repeats.
    assert_eq!(encode("a_b_c_d.bin"), None);
    // Reserved characters inside what would fold.
    assert_eq!(encode("a_x_b{_x_c"), None);
    assert_eq!(encode("a_x_b…_x_c"), None);
    assert_eq!(encode("a«_x_b_x_c"), None);
    assert!(encode("a{_x_b_x_c").is_some());
    assert_eq!(decode(&encode("a_x_b_x_c").unwrap()).unwrap(), "a_x_b_x_c");
}

#[test]
fn splits() {
    let name = format!(
        "pettftavatar{}.bin",
        (0..70).map(|k| format!("_s{k}")).collect::<String>()
    );
    // Nothing folds, so the whole stem is the preamble, cut back to fit.
    assert_eq!(encode(&name), None);
    let s = split(&name, 220);
    assert_eq!(&name[s.ext..], ".bin");
    assert!(s.pre <= 216 && name[s.pre..].starts_with('_'));
    assert_eq!(split("abc.bin", 220), Split { pre: 3, ext: 3 });
    // The preamble is what encode keeps outside its block.
    let ahri = format!("ahri_multi{}.bin", skins(0..=84));
    assert_eq!(
        split(&ahri, 220),
        Split {
            pre: 10,
            ext: ahri.len() - 4
        }
    );
    let long_ext = format!("a_b.{}", "e".repeat(300));
    assert_eq!(
        split(&long_ext, 220),
        Split {
            pre: 3,
            ext: long_ext.len()
        }
    );
    assert_eq!(split(&"x".repeat(300), 220), Split { pre: 0, ext: 300 });
}

/// Every name component over 255 units in the tables: `target/` isn't in git, so this
/// only runs where the census was taken.
#[test]
fn corpus() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../target/test-long/long-component.tsv"
    );
    let Ok(text) = std::fs::read_to_string(path) else {
        return;
    };
    let (mut rows, mut fit) = (0, 0);
    for line in text.lines() {
        let (_, path) = line.split_once('\t').unwrap();
        let name = path.rsplit('/').next().unwrap();
        let e = encode(name).unwrap_or_else(|| panic!("{name}"));
        assert_eq!(decode(&e).unwrap(), name);
        let block = &e[e.find('«').unwrap()..e.find('»').unwrap()];
        assert!(block.contains('{'), "{e}");
        let stem = e.split('.').next().unwrap();
        assert!(!(stem.len() == 16 && stem.bytes().all(|b| b.is_ascii_hexdigit())));
        rows += 1;
        fit += usize::from(units(&e) <= 255);
    }
    println!("{rows} over-long names, {fit} fit in 255 folded");
    assert_eq!((rows, fit), (4379, 4361));
}
