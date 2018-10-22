extern crate doublet;

use doublet::{Reader, Writer};

#[test]
fn writer_from_raw() {
    let raw_size = doublet::raw_size(10);

    let mut storage = vec![0; raw_size];
    let ptr = storage.as_mut_slice().as_mut_ptr();

    let mut writer = unsafe { Writer::from_raw_parts(ptr, storage.len()).unwrap() };

    let guard = writer.try_lock().unwrap();

    assert_eq!(10, guard.len());
}

#[test]
fn reader_from_raw() {
    let raw_size = doublet::raw_size(10);

    let mut storage = vec![0; raw_size];
    let ptr = storage.as_mut_slice().as_mut_ptr();

    let mut reader = unsafe { Reader::from_raw_parts(ptr, storage.len()).unwrap() };

    let guard = reader.try_lock().unwrap();

    assert_eq!(10, guard.len());
}
