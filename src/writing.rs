// Ogg decoder and encoder written in Rust
//
// Copyright (c) 2017 est31 <MTest31@outlook.com>
// and contributors. All rights reserved.
// Redistribution or use only under the terms
// specified in the LICENSE file attached to this
// source distribution.

/*!
Writing logic
*/

use std::result;
use std::rc::Rc;
use std::io::{self, Cursor, Write, Seek, SeekFrom};
use byteorder::{WriteBytesExt, LittleEndian};
use std::collections::HashMap;
use crc::vorbis_crc32_update;


/// Ogg version of the `std::io::Result` type.
///
/// We need `std::result::Result` at other points
/// too, so we can't use `Result` as the name.
type IoResult<T> = result::Result<T, io::Error>;

/**
Writer for packets into an Ogg stream.

Note that the functionality of this struct isn't as well tested as for
the `PacketReader` struct.
*/
pub struct PacketWriter<T :io::Write> {
	wtr :T,

	page_vals :HashMap<u32, CurrentPageValues>,
}

struct CurrentPageValues {
	/// `true` if this page is the first one in the logical bitstream
	first_page :bool,
	/// Page counter of the current page
	/// Increased for every page
	sequence_num :u32,

	/// Points to the first unwritten position in cur_pg_lacing.
	segment_cnt :u8,
	cur_pg_lacing :[u8; 255],
	cur_pg_data :Vec<Rc<[u8]>>,

	/// Some(offs), if the last packet
	/// couldn't make it fully into this page, and
	/// has to be continued in the next page.
	///
	/// `offs` should point to the first idx in
	/// cur_pg_data[last] that should NOT be written
	/// in this page anymore.
	///
	/// None if all packets can be written nicely.
	pck_this_overflow_idx :Option<usize>,

	/// Some(offs), if the first packet
	/// couldn't make it fully into the last page, and
	/// has to be continued in this page.
	///
	/// `offs` should point to the first idx in cur_pg_data[0]
	/// that hasn't been written.
	///
	/// None if all packets can be written nicely.
	pck_last_overflow_idx :Option<usize>,
}

/// Specifies whether to end something with the write of the packet.
///
/// If you want to end a stream you need to inform the Ogg `PacketWriter`
/// about this. This is the enum to do so.
///
/// Also, Codecs sometimes have special requirements to put
/// the first packet of the whole stream into its own page.
/// The `EndPage` variant can be used for this.
#[derive(PartialEq)]
#[derive(Clone, Copy)]
pub enum PacketWriteEndInfo {
	/// No ends here, just a normal packet
	NormalPacket,
	/// Force-end the current page
	EndPage,
	/// End the whole logical stream.
	EndStream,
}

impl <T :io::Write> PacketWriter<T> {
	pub fn new(wtr :T) -> Self {
		return PacketWriter {
			wtr : wtr,
			page_vals : HashMap::new(),
		};
	}
	pub fn into_inner(self) -> T {
		self.wtr
	}
	/// Write a packet
	///
	///
	pub fn write_packet(&mut self, pck_cont :Rc<[u8]>, serial :u32,
			inf :PacketWriteEndInfo,
			/* TODO find a better way to design the API around
				passing the absgp to the underlying implementation.
				e.g. the caller passes a closure on init which gets
				called when we encounter a new page... with the param
				the index inside the current page, or something.
			*/
			absgp :u64) -> IoResult<()> {
		let is_end_stream :bool = inf == PacketWriteEndInfo::EndStream;
		let mut pg = self.page_vals.entry(serial).or_insert(
			CurrentPageValues {
				first_page : true,
				sequence_num : 0,
				segment_cnt : 0,
				cur_pg_lacing :[0; 255],
				cur_pg_data :Vec::with_capacity(255),
				pck_this_overflow_idx : None,
				pck_last_overflow_idx : None,
			}
		);

		pg.cur_pg_data.push(pck_cont.clone());

		let last_data_segment_size = (pck_cont.len() % 255) as u8;
		let needed_segments :usize = (pck_cont.len() / 255) + 1;
		let mut segment_in_page_i :u8 = pg.segment_cnt;
		let mut at_page_end :bool = false;
		for segment_i in 0 .. needed_segments {
			at_page_end = false;
			if segment_i + 1 < needed_segments {
				// For all segments containing 255 pieces of data
				pg.cur_pg_lacing[segment_in_page_i as usize] = 255;
			} else {
				// For the last segment, must contain < 255 pieces of data
				// (including 0)
				pg.cur_pg_lacing[segment_in_page_i as usize] = last_data_segment_size;
			}
			segment_in_page_i = segment_in_page_i.wrapping_add(1);
			if segment_in_page_i == 0 {
				if segment_i + 1 < needed_segments {
					// We have to flush a page, but we know there are more to come...
					pg.pck_this_overflow_idx = Some((segment_i + 1) * 255);
					try!(PacketWriter::write_page(&mut self.wtr, serial, pg,
						false, absgp));
				} else {
					// We have to write a page end, and its the very last in the stream
					try!(PacketWriter::write_page(&mut self.wtr,
						serial, pg, is_end_stream, absgp));
					// Not actually required either
					// (it is always None except if we set it to Some directly
					// before we call write_page)
					pg.pck_this_overflow_idx = None;
					// Required (it could have been Some(offs) before)
					pg.pck_last_overflow_idx = None;
				}
				at_page_end = true;
			}
			pg.segment_cnt = segment_in_page_i;
		}
		if (inf != PacketWriteEndInfo::NormalPacket) && !at_page_end {
			// Write a page end
			try!(PacketWriter::write_page(&mut self.wtr, serial, pg,
				is_end_stream, absgp));

			pg.pck_last_overflow_idx = None;

			// TODO if inf was PacketWriteEndInfo::EndStream, we have to
			// somehow erase pg from the hashmap...
			// any ideas? perhaps needs external scope...
		}
		// All went fine.
		Ok(())
	}
	fn write_page(wtr :&mut T, serial :u32, pg :&mut CurrentPageValues,
			last_page :bool, absgp :u64)  -> IoResult<()> {
		{
			// The page header with everything but the lacing values:
			let mut hdr_cur = Cursor::new(Vec::with_capacity(27));
			try!(hdr_cur.write_all(&[0x4f, 0x67, 0x67, 0x53, 0x00]));
			let mut flags :u8 = 0;
			if pg.pck_last_overflow_idx.is_some() { flags |= 0x01; }
			if pg.first_page { flags |= 0x02; }
			if last_page { flags |= 0x04; }
			try!(hdr_cur.write_u8(flags));
			try!(hdr_cur.write_u64::<LittleEndian>(absgp));
			try!(hdr_cur.write_u32::<LittleEndian>(serial));
			try!(hdr_cur.write_u32::<LittleEndian>(pg.sequence_num));

			// checksum, calculated later on :)
			// Don't do excessive checking here (that the seek
			// succeeded & we are at the right pos now).
			// Its hopefully not required.
			try!(hdr_cur.seek(SeekFrom::Current(4)));

			try!(hdr_cur.write_u8(pg.segment_cnt));

			let mut hash_calculated :u32;

			let pg_lacing = &pg.cur_pg_lacing[0 .. pg.segment_cnt as usize];
			let pck_data = &pg.cur_pg_data;

			hash_calculated = vorbis_crc32_update(0, hdr_cur.get_ref());
			hash_calculated = vorbis_crc32_update(hash_calculated, pg_lacing);
			for (idx, pck) in pck_data.iter().enumerate() {
				let mut start :usize = 0;
				if idx == 0 { if let Some(idx) = pg.pck_last_overflow_idx {
					start = idx;
				}}
				let mut end :usize = pck.len();
				if idx + 1 == pck_data.len() {
					if let Some(idx) = pg.pck_this_overflow_idx {
						end = idx;
					}
				}
				hash_calculated = vorbis_crc32_update(hash_calculated,
					&pck[start .. end]);
			}

			// Go back to enter the checksum
			// Don't do excessive checking here (that the seek
			// succeeded & we are at the right pos now).
			// Its hopefully not required.
			try!(hdr_cur.seek(SeekFrom::Start(22)));
			try!(hdr_cur.write_u32::<LittleEndian>(hash_calculated));

			// Now all is done, write the stuff!
			try!(wtr.write_all(hdr_cur.get_ref()));
			try!(wtr.write_all(pg_lacing));
			for (idx, pck) in pck_data.iter().enumerate() {
				let mut start :usize = 0;
				if idx == 0 { if let Some(idx) = pg.pck_last_overflow_idx {
					start = idx;
				}}
				let mut end :usize = pck.len();
				if idx + 1 == pck_data.len() {
					if let Some(idx) = pg.pck_this_overflow_idx {
						end = idx;
					}
				}
				try!(wtr.write_all(&pck[start .. end]));
			}
		}

		// Reset the page.
		pg.first_page = false;
		pg.sequence_num += 1;

		pg.segment_cnt = 0;
		pg.cur_pg_data.clear();

		pg.pck_last_overflow_idx = pg.pck_this_overflow_idx;
		pg.pck_this_overflow_idx = None;

		return Ok(());
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
		w.write_packet(Rc::new(test_arr_in), 0x5b90a374,
			PacketWriteEndInfo::EndPage, 0).unwrap();
	}
	//print_u8_slice(c.get_ref());
	assert_eq!(c.get_ref().len(), test_arr_out.len());

	let cr = c.get_ref();
	test_arr_eq!(cr, test_arr_out);
}
