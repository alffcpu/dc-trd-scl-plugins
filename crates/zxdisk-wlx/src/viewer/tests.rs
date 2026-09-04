//! Tests for the detect string Double Commander asks for before it decides
//! which lister to use.
//!
//! It writes through a raw pointer into a buffer the host owns, at the size the
//! host chose. Its own comment describes the edge - "even for a 1-byte buf" -
//! and nothing checked it.

use super::{ListGetDetectString, DETECT};
use std::os::raw::{c_char, c_int};

const FILL: c_char = 0x7F;

/// Read back a NUL-terminated string the plugin wrote into our buffer.
fn read(buf: &[c_char]) -> Vec<u8> {
    buf.iter()
        .take_while(|&&c| c != 0)
        .map(|&c| c as u8)
        .collect()
}

#[test]
fn a_roomy_buffer_gets_the_whole_string_and_a_terminator() {
    let mut buf = [FILL; 256];
    unsafe { ListGetDetectString(buf.as_mut_ptr(), buf.len() as c_int) };
    assert_eq!(read(&buf), DETECT);
    assert_eq!(buf[DETECT.len()], 0, "no terminator after the string");
}

#[test]
fn a_short_buffer_is_truncated_and_still_terminated() {
    // Every length from 1 up to just past the string: it must never write past
    // what it was given, and must always leave a terminator.
    for cap in 1..=(DETECT.len() + 2) {
        let mut buf = vec![FILL; cap + 8];
        unsafe { ListGetDetectString(buf.as_mut_ptr(), cap as c_int) };

        let got = read(&buf);
        assert!(
            got.len() < cap,
            "cap {cap}: no room left for the terminator"
        );
        assert_eq!(got, &DETECT[..got.len()], "cap {cap}: not a prefix");
        assert_eq!(buf[got.len()], 0, "cap {cap}: not terminated");

        for (i, b) in buf.iter().enumerate().skip(cap) {
            assert_eq!(*b, FILL, "cap {cap}: wrote past the end at {i}");
        }
    }
}

#[test]
fn a_buffer_that_is_no_buffer_is_left_alone() {
    // A null is what a failed allocation looks like, and a zero length is what
    // a host with nothing to spare passes. Either has to be a return, not a
    // write.
    unsafe { ListGetDetectString(std::ptr::null_mut(), 256) };

    let mut buf = [FILL; 8];
    unsafe { ListGetDetectString(buf.as_mut_ptr(), 0) };
    unsafe { ListGetDetectString(buf.as_mut_ptr(), -1) };
    assert!(
        buf.iter().all(|&c| c == FILL),
        "a zero or negative length still wrote something"
    );
}

#[test]
fn the_detect_string_is_the_shape_double_commander_parses() {
    let s = String::from_utf8(DETECT.to_vec()).expect("not ASCII");
    // Parenthesised, single '=', no spaces - the form DC's own plugins use and
    // the form its parser accepts.
    assert!(s.contains("(SIZE=6912)"), "{s}");
    assert!(s.contains("(SIZE=6144)"), "{s}");
    assert!(!s.contains(' '), "a space stops DC's parser: {s}");
    assert!(!s.contains("=="), "{s}");
}
