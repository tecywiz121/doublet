mod toggle;

use toggle::{Side, State, ToggleCount};

use std::ops::{Deref, DerefMut};
use std::slice;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::sync::Mutex;

#[derive(Debug)]
pub struct OwnedDoublet {
    header: Header,

    left_buffer: Vec<u8>,
    right_buffer: Vec<u8>,

    has_writer: Mutex<bool>,
}

impl OwnedDoublet {
    pub fn new(size: usize) -> OwnedDoublet {
        Self {
            header: Header {
            toggle: ToggleCount::default(),
            remaining_readers: AtomicUsize::new(0),
            },

            left_buffer: vec![0; size],
            right_buffer: vec![0; size],

            has_writer: Mutex::new(true),
        }
    }

    fn make_doublet(&self) -> Doublet {
        Doublet {
            header: &self.header,

            left_buffer: self.left_buffer.as_ptr() as *mut _,
            right_buffer: self.right_buffer.as_ptr() as *mut _,

            size: self.left_buffer.len(),
        }
    }

    pub fn take_writer(&self) -> Option<Writer> {
        {
            let mut has_writer = self.has_writer.lock().expect("lock writer mutex");

            if !*has_writer {
                return None;
            }

            *has_writer = false;
        }

        let doublet = self.make_doublet();

        let writer = Writer(doublet);

        Some(writer)
    }

    pub fn reader(&self) -> Reader {
        let doublet = self.make_doublet();
        Reader(doublet)
    }
}

pub fn raw_size(buffer_size: usize) -> usize {
    Doublet::header_size() + (buffer_size * 2)
}

#[derive(Debug)]
#[repr(C)]
struct Header {
    toggle: ToggleCount,
    remaining_readers: AtomicUsize,
}

#[derive(Debug, Clone)]
struct Doublet<'a> {
    header: &'a Header,

    size: usize,
    left_buffer: *mut u8,
    right_buffer: *mut u8,
}

impl<'a> Doublet<'a> {
    fn buffer_ptr(&self, side: Side) -> *mut u8 {
        match side {
            Side::Left => self.left_buffer,
            Side::Right => self.right_buffer,
        }
    }

    fn toggle(&self) -> &'a ToggleCount {
        &self.header.toggle
    }

    fn remaining_readers(&self) -> &'a AtomicUsize {
        &self.header.remaining_readers
    }

    fn header_size() -> usize {
        std::mem::size_of::<Header>()
    }

    unsafe fn from_raw_parts(buf: *mut u8, size: usize) -> Result<Self, ()> {
        let hdrsz = Self::header_size();

        if size <= hdrsz {
            return Err(());
        }

        let rest = size - hdrsz;

        if 0 != rest % 2 {
            return Err(());
        }

        let buffer_size = rest / 2;

        let hdr_ptr = buf as *mut Header;

        let left_buffer = buf.add(hdrsz);
        let right_buffer = left_buffer.add(buffer_size);

        Ok(Self {
            header: &*hdr_ptr,

            size: buffer_size,
            left_buffer,
            right_buffer,
        })
    }
}

unsafe impl<'a> Send for Doublet<'a> {}

#[derive(Debug, Clone)]
pub struct Reader<'b>(Doublet<'b>);

impl<'b> Reader<'b> {
    pub unsafe fn from_raw_parts(buf: *mut u8, size: usize) -> Result<Self, ()> {
        let doublet = Doublet::from_raw_parts(buf, size)?;
        Ok(Reader(doublet))
    }

    pub fn try_lock<'a>(&'a mut self) -> Result<ReadGuard<'a, 'b>, ()> {
        let original = self.0.toggle().load(Ordering::SeqCst);

        if original.count == usize::max_value() - 1 {
            return Err(());
        }

        let mut new = original.clone();
        new.count += 1;

        let old = self
            .0
            .toggle()
            .compare_and_swap(original, new, Ordering::SeqCst);

        if old != original {
            return Err(());
        }

        let buffer_ptr = self.0.buffer_ptr(new.side);
        let buffer = unsafe { slice::from_raw_parts(buffer_ptr, self.0.size) };

        let guard = ReadGuard {
            reader: self,
            reading_from: new.side,
            buffer,
        };

        Ok(guard)
    }
}

#[derive(Debug)]
pub struct ReadGuard<'a, 'b>
where
    'b: 'a,
{
    reading_from: Side,
    reader: &'a mut Reader<'b>,
    buffer: &'b [u8],
}

impl<'a, 'b> Deref for ReadGuard<'a, 'b> {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        self.buffer
    }
}

impl<'a, 'b> Drop for ReadGuard<'a, 'b> {
    fn drop(&mut self) {
        loop {
            let original = self.reader.0.toggle().load(Ordering::SeqCst);

            if original.side != self.reading_from {
                break;
            }

            let mut new = original.clone();
            new.count -= 1;

            let old = self
                .reader
                .0
                .toggle()
                .compare_and_swap(original, new, Ordering::SeqCst);

            if old == original {
                return;
            }
        }

        self.reader
            .0
            .remaining_readers()
            .fetch_sub(1, Ordering::SeqCst);
    }
}

#[derive(Debug)]
pub struct Writer<'b>(Doublet<'b>);

impl<'b> Writer<'b> {
    pub unsafe fn from_raw_parts(buf: *mut u8, size: usize) -> Result<Self, ()> {
        let doublet = Doublet::from_raw_parts(buf, size)?;
        Ok(Writer(doublet))
    }

    pub fn try_lock<'a>(&'a mut self) -> Result<WriteGuard<'a, 'b>, ()> {
        let old =
            self.0
                .remaining_readers()
                .compare_and_swap(0, usize::max_value(), Ordering::SeqCst);

        if 0 == old {
            let toggle = self.0.toggle().load(Ordering::SeqCst);
            let writing_to = !toggle.side;

            let buffer_ptr = self.0.buffer_ptr(writing_to);
            let buffer = unsafe { slice::from_raw_parts_mut(buffer_ptr, self.0.size) };

            let guard = WriteGuard {
                buffer,
                writing_to,
                writer: Some(self),
            };

            Ok(guard)
        } else {
            Err(())
        }
    }
}

#[derive(Debug)]
pub struct WriteGuard<'a, 'b>
where
    'b: 'a,
{
    writer: Option<&'a mut Writer<'b>>,
    writing_to: Side,
    buffer: &'b mut [u8],
}

impl<'a, 'b> Deref for WriteGuard<'a, 'b> {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        self.buffer
    }
}

impl<'a, 'b> DerefMut for WriteGuard<'a, 'b> {
    fn deref_mut(&mut self) -> &mut [u8] {
        self.buffer
    }
}

impl<'a, 'b> WriteGuard<'a, 'b> {
    pub fn activate(mut self) {
        let writer = match self.writer.take() {
            Some(x) => x,
            None => unreachable!(),
        };

        let new = State {
            side: self.writing_to,
            count: 0,
        };

        let original = writer.0.toggle().swap(new, Ordering::SeqCst);

        let mut prev_readers = usize::max_value();

        loop {
            let remaining_readers = original.count - (usize::max_value() - prev_readers);

            let old = writer.0.remaining_readers().compare_and_swap(
                prev_readers,
                remaining_readers,
                Ordering::SeqCst,
            );

            if old == prev_readers {
                break;
            } else {
                prev_readers = old;
            }
        }
    }
}

impl<'a, 'b> Drop for WriteGuard<'a, 'b> {
    fn drop(&mut self) {
        let writer = match self.writer.take() {
            Some(x) => x,
            None => return,
        };

        writer.0.remaining_readers().store(0, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lkwr_unwr_lkrd_unrd() {
        let owned = OwnedDoublet::new(1);

        let mut writer = owned.take_writer().unwrap();
        let mut reader = owned.reader();

        {
            let mut guard = writer.try_lock().unwrap();

            assert_eq!(guard.writing_to, Side::Right);
            assert_eq!(usize::max_value(), owned.header.remaining_readers.load(Ordering::SeqCst));

            guard[0] = 55;

            guard.activate();
        }

        {
            let guard = reader.try_lock().unwrap();

            assert_eq!(guard.reading_from, Side::Right);

            let state = State {
                side: Side::Right,
                count: 1,
            };
            assert_eq!(state, owned.header.toggle.load(Ordering::SeqCst));

            assert_eq!(55, guard[0]);
        }

        let state = State {
            side: Side::Right,
            count: 0,
        };
        assert_eq!(state, owned.header.toggle.load(Ordering::SeqCst));
    }

    #[test]
    fn lkrd_lkwr_unwr_unrd() {
        let owned = OwnedDoublet::new(1);

        let mut writer = owned.take_writer().unwrap();
        let mut reader = owned.reader();

        // Lock the reader
        let rd_guard = reader.try_lock().unwrap();

        assert_eq!(rd_guard.reading_from, Side::Left);

        let state = State {
            side: Side::Left,
            count: 1,
        };
        assert_eq!(state, owned.header.toggle.load(Ordering::SeqCst));

        assert_eq!(0, rd_guard[0]);

        // Write and Activate Right buffer
        {
            let mut guard = writer.try_lock().unwrap();

            assert_eq!(guard.writing_to, Side::Right);
            assert_eq!(usize::max_value(), owned.header.remaining_readers.load(Ordering::SeqCst));

            guard[0] = 55;

            guard.activate();
        }

        let state = State {
            side: Side::Right,
            count: 0,
        };

        assert_eq!(state, owned.header.toggle.load(Ordering::SeqCst));
        assert_eq!(1, owned.header.remaining_readers.load(Ordering::SeqCst));

        ::std::mem::drop(rd_guard);

        assert_eq!(0, owned.header.remaining_readers.load(Ordering::SeqCst));
    }

    #[test]
    fn lkrd_lkwr_unrd_unwr() {
        let owned = OwnedDoublet::new(1);

        let mut writer = owned.take_writer().unwrap();
        let mut reader = owned.reader();

        // Lock the reader
        let rd_guard = reader.try_lock().unwrap();

        assert_eq!(rd_guard.reading_from, Side::Left);

        let state = State {
            side: Side::Left,
            count: 1,
        };
        assert_eq!(state, owned.header.toggle.load(Ordering::SeqCst));

        assert_eq!(0, rd_guard[0]);

        // Write and Activate Right buffer
        let mut wr_guard = writer.try_lock().unwrap();

        assert_eq!(wr_guard.writing_to, Side::Right);
        assert_eq!(usize::max_value(), owned.header.remaining_readers.load(Ordering::SeqCst));

        wr_guard[0] = 55;

        // Drop the read guard
        ::std::mem::drop(rd_guard);

        let state = State {
            side: Side::Left,
            count: 0,
        };
        assert_eq!(state, owned.header.toggle.load(Ordering::SeqCst));
        assert_eq!(usize::max_value(), owned.header.remaining_readers.load(Ordering::SeqCst));

        // Flip the active buffer
        wr_guard.activate();

        let state = State {
            side: Side::Right,
            count: 0,
        };

        assert_eq!(state, owned.header.toggle.load(Ordering::SeqCst));
        assert_eq!(0, owned.header.remaining_readers.load(Ordering::SeqCst));

    }
}
