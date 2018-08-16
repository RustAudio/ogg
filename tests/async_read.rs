// Ogg decoder and encoder written in Rust
//
// Copyright (c) 2016 est31 <MTest31@outlook.com>
// and contributors. All rights reserved.
// Redistribution or use only under the terms
// specified in the LICENSE file attached to this
// source distribution.

#![cfg(feature = "async")]

extern crate ogg;
extern crate rand;
extern crate tokio_io;
extern crate futures;

use std::io;
use ogg::{PacketWriter, PacketWriteEndInfo};
use ogg::reading::async_api::PacketReader;
use std::boxed::Box;
use std::io::{Cursor, Seek, SeekFrom};
use tokio_io::AsyncRead;
use futures::Stream;

struct RandomWouldBlock<T>(T);
impl <T: io::Read> io::Read for RandomWouldBlock<T> {
	fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
		if rand::random() {
			return Err(io::Error::new(io::ErrorKind::WouldBlock, "would block"));
		}
		self.0.read(buf)
	}
}

impl <T :io::Read> AsyncRead for RandomWouldBlock<T> {}

impl <T: io::Seek> io::Seek for RandomWouldBlock<T> {
	fn seek(&mut self, pos: io::SeekFrom) -> io::Result<u64> {
		if rand::random() {
			return Err(io::Error::new(io::ErrorKind::WouldBlock, "would block"));
		}
		self.0.seek(pos)
	}
}

macro_rules! test_arr_eq {
	($a_arr:expr, $b_arr:expr) => {
		for i in 0 .. $b_arr.len() {
			if $a_arr[i] != $b_arr[i] {
				panic!("Mismatch of values at index {}: {} {}", i, $a_arr[i], $b_arr[i]);
			}
		}
	}
}

macro_rules! cont_try {
	($e:expr) => {
		(|| {
			loop {
				match $e {
					Ok(futures::Async::Ready(v)) => return Ok(v),
					Ok(_) => (),
					Err(e) => return Err(e),
				}
			}
		}) ()
	}
}

fn test_ogg_random_would_block_run() {
	let mut c = Cursor::new(Vec::new());
	let test_arr = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
	let test_arr_2 = [2, 4, 8, 16, 32, 64, 128, 127, 126, 125, 124];
	let test_arr_3 = [3, 5, 9, 17, 33, 65, 129, 129, 127, 126, 125];
	{
		let mut w = PacketWriter::new(&mut c);
		let np = PacketWriteEndInfo::NormalPacket;
		w.write_packet(Box::new(test_arr), 0xdeadb33f, np, 0).unwrap();
		w.write_packet(Box::new(test_arr_2), 0xdeadb33f, np, 1).unwrap();
		w.write_packet(Box::new(test_arr_3), 0xdeadb33f,
			PacketWriteEndInfo::EndPage, 2).unwrap();
	}
	//print_u8_slice(c.get_ref());
	assert_eq!(c.seek(SeekFrom::Start(0)).unwrap(), 0);
	{
		let mut rwd = RandomWouldBlock(&mut c);
		let mut r = PacketReader::new(&mut rwd);
		let p1 = cont_try!(r.poll()).unwrap().unwrap();
		assert_eq!(test_arr, *p1.data);
		let p2 = cont_try!(r.poll()).unwrap().unwrap();
		assert_eq!(test_arr_2, *p2.data);
		let p3 = cont_try!(r.poll()).unwrap().unwrap();
		assert_eq!(test_arr_3, *p3.data);
	}

	// Now test packets spanning multiple segments
	let mut c = Cursor::new(Vec::new());
	let test_arr = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
	let mut test_arr_2 = [0; 700];
	let test_arr_3 = [3, 5, 9, 17, 33, 65, 129, 129, 127, 126, 125];
	for (idx, a) in test_arr_2.iter_mut().enumerate() {
		*a = (idx as u8) / 4;
	}
	{
		let mut w = PacketWriter::new(&mut c);
		let np = PacketWriteEndInfo::NormalPacket;
		w.write_packet(Box::new(test_arr), 0xdeadb33f, np, 0).unwrap();
		w.write_packet(Box::new(test_arr_2), 0xdeadb33f, np, 1).unwrap();
		w.write_packet(Box::new(test_arr_3), 0xdeadb33f,
			PacketWriteEndInfo::EndPage, 2).unwrap();
	}
	//print_u8_slice(c.get_ref());
	assert_eq!(c.seek(SeekFrom::Start(0)).unwrap(), 0);
	{
		let mut rwd = RandomWouldBlock(&mut c);
		let mut r = PacketReader::new(&mut rwd);
		let p1 = cont_try!(r.poll()).unwrap().unwrap();
		assert_eq!(test_arr, *p1.data);
		let p2 = cont_try!(r.poll()).unwrap().unwrap();
		test_arr_eq!(test_arr_2, *p2.data);
		let p3 = cont_try!(r.poll()).unwrap().unwrap();
		assert_eq!(test_arr_3, *p3.data);
	}

	// Now test packets spanning multiple pages
	let mut c = Cursor::new(Vec::new());
	let mut test_arr_2 = [0; 14_000];
	let test_arr_3 = [3, 5, 9, 17, 33, 65, 129, 129, 127, 126, 125];
	for (idx, a) in test_arr_2.iter_mut().enumerate() {
		*a = (idx as u8) / 4;
	}
	{
		let mut w = PacketWriter::new(&mut c);
		let np = PacketWriteEndInfo::NormalPacket;
		w.write_packet(Box::new(test_arr_2), 0xdeadb33f, np, 1).unwrap();
		w.write_packet(Box::new(test_arr_3), 0xdeadb33f,
			PacketWriteEndInfo::EndPage, 2).unwrap();
	}
	//print_u8_slice(c.get_ref());
	assert_eq!(c.seek(SeekFrom::Start(0)).unwrap(), 0);
	{
		let mut rwd = RandomWouldBlock(&mut c);
		let mut r = PacketReader::new(&mut rwd);
		let p2 = cont_try!(r.poll()).unwrap().unwrap();
		test_arr_eq!(test_arr_2, *p2.data);
		let p3 = cont_try!(r.poll()).unwrap().unwrap();
		assert_eq!(test_arr_3, *p3.data);
	}
}

#[test]
fn test_ogg_random_would_block() {
	for i in 0 .. 100 {
		println!("Run {}", i);
		test_ogg_random_would_block_run();
	}
}
