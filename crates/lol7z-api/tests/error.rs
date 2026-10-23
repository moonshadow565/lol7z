use lol7z_api::{Error, ErrorFlags, HRESULT, OperationResult};
use std::io;

#[test]
fn hresult_survives_io_round_trip() {
    for e in [Error::ABORT, Error::FAIL, Error::OUT_OF_MEMORY] {
        assert_eq!(Error::from(io::Error::from(e)), e);
    }
    let win32 = io::Error::from_raw_os_error(5); // ERROR_ACCESS_DENIED
    assert_eq!(Error::from(win32).code(), HRESULT::from_win32(5));
    assert_eq!(Error::from(io::Error::other("x")), Error::FAIL);
}

#[test]
fn damage_survives_io_round_trip() {
    for e in [Error::UNEXPECTED_END, Error::CRC_ERROR, Error::NotArchive] {
        assert_eq!(Error::from(io::Error::from(e)), e);
    }
    let eof = io::Error::from(Error::UNEXPECTED_END);
    assert_eq!(eof.kind(), io::ErrorKind::UnexpectedEof);
}

#[test]
fn io_kinds_map_to_damage() {
    let eof = io::Error::from(io::ErrorKind::UnexpectedEof);
    assert_eq!(Error::from(eof), Error::UNEXPECTED_END);
    let bad = io::Error::new(io::ErrorKind::InvalidData, "bad zstd frame");
    assert_eq!(Error::from(bad), Error::DATA_ERROR);
}

#[test]
fn operation_results() {
    let cases = [
        (Error::UNEXPECTED_END, Some(OperationResult::UnexpectedEnd)),
        (Error::CRC_ERROR, Some(OperationResult::CRCError)),
        (Error::DATA_ERROR, Some(OperationResult::DataError)),
        (
            Error::Damaged(ErrorFlags::empty()),
            Some(OperationResult::DataError),
        ),
        (
            Error::Damaged(ErrorFlags::UnsupportedFeature),
            Some(OperationResult::UnsupportedMethod),
        ),
        (Error::NotArchive, Some(OperationResult::IsNotArc)),
        (Error::ABORT, None),
    ];
    for (e, want) in cases {
        assert_eq!(e.operation_result(), want, "{e:?}");
    }
    assert_eq!(Error::UNEXPECTED_END.code(), Error::FAIL.code());
}
