extern crate crossbeam_utils;
extern crate doublet;
extern crate rand;

use crossbeam_utils::thread;

use doublet::{OwnedDoublet, Reader, Writer};

use rand::Rng;

use std::collections::HashSet;
use std::mem;
use std::slice;
use std::time::{Duration, Instant};

fn rand_sleep() {
    let mut rng = rand::thread_rng();
    let dur = rng.gen_range(Duration::from_millis(0), Duration::from_millis(1));
    ::std::thread::sleep(dur);
}

fn as_bytes(x: &usize) -> &[u8] {
    unsafe {
        let ptr = x as *const usize;
        let buf = ptr as *const u8;
        slice::from_raw_parts(buf, mem::size_of::<usize>())
    }
}

fn from_bytes(x: &[u8]) -> usize {
    assert_eq!(x.len(), mem::size_of::<usize>());

    unsafe {
        let buf = x.as_ptr();
        let ptr = buf as *const usize;
        *ptr
    }
}

fn write<'a>(mut writer: Writer<'a>, end: Instant) -> usize {
    let mut count = 0;

    while Instant::now() < end {
        let mut guard = match writer.try_lock() {
            Ok(x) => x,
            Err(_) => {
                rand_sleep();
                continue;
            }
        };

        count += 1;

        guard.copy_from_slice(as_bytes(&count));
        guard.activate();
    }

    assert!(100 < count);

    count
}

fn read<'a>(mut reader: Reader<'a>, end: Instant) -> usize {
    let end = end + Duration::from_secs(1);
    let mut last = 0;

    while Instant::now() < end {
        {
            let guard = match reader.try_lock() {
                Ok(x) => x,
                Err(_) => continue,
            };

            rand_sleep();

            let value = from_bytes(&*guard);

            assert!(value >= last);

            last = value;
        }

        rand_sleep();
    }

    last
}

#[test]
fn soak() {
    let end = Instant::now() + Duration::from_secs(58);

    let owned = OwnedDoublet::new(mem::size_of::<usize>());

    let writer = owned.take_writer().unwrap();

    thread::scope(|scope| {
        let mut handles = vec![];

        // Create writer thread.
        handles.push(scope.spawn(move || write(writer, end)));

        // Create reader threads.
        for _ in 0..16 {
            let reader = owned.reader();
            handles.push(scope.spawn(move || read(reader, end)));
        }

        let counts = handles
            .into_iter()
            .map(|x| x.join())
            .map(|x| x.unwrap())
            .collect::<HashSet<usize>>();

        for v in counts.iter() {
            println!("Highest Value: {}", v);
        }

        assert_eq!(1, counts.len());
    });
}
