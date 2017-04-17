// Ogg decoder and encoder written in Rust
//
// Copyright (c) 2016-2017 est31 <MTest31@outlook.com>
// and contributors. All rights reserved.
// Redistribution or use only under the terms
// specified in the LICENSE file attached to this
// source distribution.

use super::*;

use std::io::{Cursor, Seek, SeekFrom};

macro_rules! test_arr_eq {
	($a_arr:expr, $b_arr:expr) => {
		for i in 0 .. $b_arr.len() {
			if $a_arr[i] != $b_arr[i] {
				panic!("Mismatch of values at index {}: {} {}", i, $a_arr[i], $b_arr[i]);
			}
		}
	}
}

#[test]
fn test_ogg_packet_rw() {
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
		let mut r = PacketReader::new(c);
		let p1 = r.read_packet().unwrap();
		assert_eq!(test_arr, *p1.data);
		let p2 = r.read_packet().unwrap();
		assert_eq!(test_arr_2, *p2.data);
		let p3 = r.read_packet().unwrap();
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
		let mut r = PacketReader::new(&mut c);
		let p1 = r.read_packet().unwrap();
		assert_eq!(test_arr, *p1.data);
		let p2 = r.read_packet().unwrap();
		test_arr_eq!(test_arr_2, *p2.data);
		let p3 = r.read_packet().unwrap();
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
		let mut r = PacketReader::new(c);
		let p2 = r.read_packet().unwrap();
		test_arr_eq!(test_arr_2, *p2.data);
		let p3 = r.read_packet().unwrap();
		assert_eq!(test_arr_3, *p3.data);
	}
}

#[test]
fn test_page_end_after_first_packet() {
	// Test that everything works well if we force a page end
	// after the first packet
	let mut c = Cursor::new(Vec::new());
	let test_arr = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
	let test_arr_2 = [2, 4, 8, 16, 32, 64, 128, 127, 126, 125, 124];
	let test_arr_3 = [3, 5, 9, 17, 33, 65, 129, 129, 127, 126, 125];
	{
		let mut w = PacketWriter::new(&mut c);
		let np = PacketWriteEndInfo::NormalPacket;
		w.write_packet(Box::new(test_arr), 0xdeadb33f,
			PacketWriteEndInfo::EndPage, 0).unwrap();
		w.write_packet(Box::new(test_arr_2), 0xdeadb33f, np, 1).unwrap();
		w.write_packet(Box::new(test_arr_3), 0xdeadb33f,
			PacketWriteEndInfo::EndPage, 2).unwrap();
	}
	//print_u8_slice(c.get_ref());
	assert_eq!(c.seek(SeekFrom::Start(0)).unwrap(), 0);
	{
		let mut r = PacketReader::new(c);
		let p1 = r.read_packet().unwrap();
		assert_eq!(test_arr, *p1.data);
		let p2 = r.read_packet().unwrap();
		assert_eq!(test_arr_2, *p2.data);
		let p3 = r.read_packet().unwrap();
		assert_eq!(test_arr_3, *p3.data);
	}
}

#[test]
fn test_ogg_packet_write() {
	let mut c = Cursor::new(Vec::new());

	// Test page taken from real Ogg file
	let test_arr_out = [
	0x4f, 0x67, 0x67, 0x53, 0x00, 0x02, 0x00, 0x00,
	0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x74, 0xa3,
	0x90, 0x5b, 0x00, 0x00, 0x00, 0x00, 0x6d, 0x94,
	0x4e, 0x3d, 0x01, 0x1e, 0x01, 0x76, 0x6f, 0x72,
	0x62, 0x69, 0x73, 0x00, 0x00, 0x00, 0x00, 0x02,
	0x44, 0xac, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
	0x80, 0xb5, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00,
	0xb8, 0x01u8];
	let test_arr_in = [0x01, 0x76, 0x6f, 0x72,
	0x62, 0x69, 0x73, 0x00, 0x00, 0x00, 0x00, 0x02,
	0x44, 0xac, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
	0x80, 0xb5, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00,
	0xb8, 0x01u8];

	{
		let mut w = PacketWriter::new(&mut c);
		w.write_packet(Box::new(test_arr_in), 0x5b90a374,
			PacketWriteEndInfo::EndPage, 0).unwrap();
	}
	//print_u8_slice(c.get_ref());
	assert_eq!(c.get_ref().len(), test_arr_out.len());

	let cr = c.get_ref();
	test_arr_eq!(cr, test_arr_out);
}

struct XorShift {
	state :(u32, u32, u32, u32),
}
impl XorShift {
	fn from_two(seed :(u32, u32)) -> Self {
		let mut xs = XorShift {
			state : (seed.0 ^ 0x2a24a930, seed.1 ^ 0xa9f60227,
				!seed.0 ^ 0x68c44d2d, !seed.1 ^ 0xa1f9794a)
		};
		xs.next();
		xs.next();
		xs.next();
		xs
	}

	fn next(&mut self) -> u32 {
		let mut r = self.state.3;
		r ^= r << 11;
		r ^= r >> 8;
		self.state.3 = self.state.2;
		self.state.2 = self.state.1;
		self.state.1 = self.state.0;
		r ^= self.state.0;
		r ^= self.state.0 >> 19;
		self.state.0 = r;
		r
	}
}

fn gen_pck(seed :u32, len_d_four :usize) -> Box<[u8]> {
	let mut ret = Vec::with_capacity(len_d_four * 4);
	let mut xs = XorShift::from_two((seed, len_d_four as u32));
	for _ in 0..len_d_four {
		let v = xs.next();
		ret.push(v as u8);
		ret.push((v >> 8) as u8);
		ret.push((v >> 16) as u8);
		ret.push((v >> 24) as u8);
	}
	ret.into_boxed_slice()
}

#[test]
fn test_seeking() {
	let pck_count = 402;
	let mut rng = XorShift::from_two((0x9899eb03, 0x54138143));

	let mut c = Cursor::new(Vec::new());

	{
		let mut w = PacketWriter::new(&mut c);
		let np = PacketWriteEndInfo::NormalPacket;
		let ep = PacketWriteEndInfo::EndPage;

		for ctr in 0..pck_count {
			w.write_packet(gen_pck(ctr, rng.next() as usize & 127), 0xdeadb33f,
				if (ctr + 1) % 3 == 0 { ep } else { np }, ctr as u64).unwrap();
		}
	}
	assert_eq!(c.seek(SeekFrom::Start(0)).unwrap(), 0);
	{
		let mut r = PacketReader::new(c);
		macro_rules! test_seek {
			($absgp:expr) => {
				// First, perform the seek
				r.seek_absgp(None, $absgp).unwrap();
				// Then go to the searched packet inside the page
				// We know that all groups of three packets form one.
				for _ in 0 .. $absgp % 3 {
					r.read_packet().unwrap();
				}
				// Now read the actual packet we are interested in and
				let pck = r.read_packet().unwrap();
				// a) ensure we have a correct absolute granule pos
				// for the page and
				assert_eq!($absgp - ($absgp % 3), pck.absgp_page - 2);
				// b) ensure the packet's content matches with the one we
				// have put in. This is another insurance.
				test_arr_eq!(pck.data, gen_pck($absgp, &pck.data.len() / 4));
			};
		}
		macro_rules! ensure_continues {
			($absgp:expr) => {
				// Ensure the stream continues normally
				let pck = r.read_packet().unwrap();
				test_arr_eq!(pck.data, gen_pck($absgp, &pck.data.len() / 4));
				let pck = r.read_packet().unwrap();
				test_arr_eq!(pck.data, gen_pck($absgp + 1, &pck.data.len() / 4));
				let pck = r.read_packet().unwrap();
				test_arr_eq!(pck.data, gen_pck($absgp + 2, &pck.data.len() / 4));
				let pck = r.read_packet().unwrap();
				test_arr_eq!(pck.data, gen_pck($absgp + 3, &pck.data.len() / 4));
			};
		}
		test_seek!(32);
		test_seek!(300);
		test_seek!(314);
		test_seek!(100);
		ensure_continues!(101);
		test_seek!(10);
		ensure_continues!(11);
		// Ensure that if we seek to the same place multiple times, it doesn't
		// fill data needlessly.
		r.seek_absgp(None, 377).unwrap();
		r.seek_absgp(None, 377).unwrap();
		test_seek!(377);
		ensure_continues!(378);
		// Ensure that if we seek to the same place multiple times, it doesn't
		// fill data needlessly.
		r.seek_absgp(None, 200).unwrap();
		r.seek_absgp(None, 200).unwrap();
		test_seek!(200);
		ensure_continues!(201);
		// Ensure the final page can be sought to
		test_seek!(401);
		// TODO after we sought to the final page, we should be able to seek
		// before it again. Right now this doesn't work.
		// test_seek!(250);
	}
}

// TODO add seeking tests for more cases:
//     * -1 absgp pages (continued pages)
//     * multiple logical streams
//     * seeking to unavailable positions
