use lol7z_api::{InStream, OutStream};
use std::cell::RefCell;
use std::io::{self, Cursor, Read, Seek, SeekFrom, Write};
use std::rc::Rc;

#[test]
fn in_stream_round_trip() {
    let s = InStream::new(Cursor::new(b"0123456789".to_vec()));
    assert_eq!(s.size().unwrap(), 10);
    let mut buf = [0; 3];
    s.read_exact_at(4, &mut buf).unwrap();
    assert_eq!(&buf, b"456");

    let mut a = s.slice(2, 3);
    let mut b = s.slice(7, 10);
    let mut out = Vec::new();
    b.read_to_end(&mut out).unwrap();
    assert_eq!(out, b"789", "slice stops at the end of the stream");
    out.clear();
    a.read_to_end(&mut out).unwrap();
    assert_eq!(out, b"234");
    a.seek(SeekFrom::End(-1)).unwrap();
    out.clear();
    a.read_to_end(&mut out).unwrap();
    assert_eq!(out, b"4");
}

#[test]
fn out_stream_round_trip() {
    struct Shared(Rc<RefCell<Vec<u8>>>);
    impl Write for Shared {
        fn write(&mut self, b: &[u8]) -> io::Result<usize> {
            self.0.borrow_mut().extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let buf = Rc::new(RefCell::new(Vec::new()));
    let mut s = OutStream::new(Shared(buf.clone()));
    assert!(!s.is_seekable());
    s.write_all(b"hello").unwrap();
    assert_eq!(&*buf.borrow(), b"hello");
    let err = s.seek(SeekFrom::Start(0)).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::Unsupported);

    let mut s = OutStream::seekable(Cursor::new(Vec::new()));
    assert!(s.is_seekable());
    s.write_all(b"abc").unwrap();
    assert_eq!(s.seek(SeekFrom::Current(-1)).unwrap(), 2);
}
