// Ogg decoder and encoder written in Rust
//
// Copyright (c) 2017 est31 <MTest31@outlook.com>
// and contributors. All rights reserved.
// Redistribution or use only under the terms
// specified in the LICENSE file attached to this
// source distribution.

/*!
Reading logic
*/

use std::error;
use std::io;
use std::io::{Cursor, Write, SeekFrom, Error};
use byteorder::{ReadBytesExt, LittleEndian};
use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::fmt::{Display, Formatter, Error as FmtError};
use std::mem::replace;
use crc::vorbis_crc32_update;
use Packet;
#[cfg(feature = "async")]
use {AdvanceAndSeekBack};
#[cfg(feature = "async")]
use std::io::Seek;

/// Error that can be raised when decoding an Ogg transport.
#[derive(Debug)]
pub enum OggReadError {
	/// The capture pattern for a new page was not found
	/// where one was expected.
	NoCapturePatternFound,
	/// Invalid stream structure version, with the given one
	/// attached.
	InvalidStreamStructVer(u8),
	/// Mismatch of the hash value with (expected, calculated) value.
	HashMismatch(u32, u32),
	/// I/O error occured.
	ReadError(io::Error),
	/// Some constraint required by the spec was not met.
	InvalidData,
}

impl error::Error for OggReadError {
	fn description(&self) -> &str {
		match self {
			&OggReadError::NoCapturePatternFound => "No Ogg capture pattern found",
			&OggReadError::InvalidStreamStructVer(_) =>
				"A non zero stream structure version was passed",
			&OggReadError::HashMismatch(_, _) => "CRC32 hash mismatch",
			&OggReadError::ReadError(_) => "I/O error",
			&OggReadError::InvalidData => "Constraint violated",
		}
	}

	fn cause(&self) -> Option<&error::Error> {
		match self {
			&OggReadError::ReadError(ref err) => Some(err as &error::Error),
			_ => None
		}
	}
}

impl Display for OggReadError {
	fn fmt(&self, fmt :&mut Formatter) -> Result<(), FmtError> {
		write!(fmt, "{}", error::Error::description(self))
	}
}

impl From<io::Error> for OggReadError {
	fn from(err :io::Error) -> OggReadError {
		return OggReadError::ReadError(err);
	}
}

/// Containing information about an OGG page that is shared between multiple places
struct PageBaseInfo {
	/// `true`: the first packet is continued from the page before. `false`: if its a "fresh" one
	starts_with_continued :bool,
	/// `true` if this page is the first one in the logical bitstream
	first_page :bool,
	/// `true` if this page is the last one in the logical bitstream
	last_page :bool,
	/// Absolute granule position. The codec defines further meaning.
	absgp :u64,
	/// Page counter
	#[allow(unused)]
	sequence_num :u32,
	/// Packet information:
	/// index is number of packet,
	/// tuple is (offset, length) of packet
	/// if ends_with_continued is true, the last element will contain information
	/// about the continued packet
	packet_positions :Vec<(u16,u16)>,
	/// `true` if the packet is continued in subsequent page(s)
	/// `false` if the packet has a segment of length < 255 inside this page
	ends_with_continued :bool,
}

/// Internal helper struct for PacketReader state
struct PageInfo {
	/// Basic information about the last read page
	bi :PageBaseInfo,
	/// The index of the first "unread" packet
	packet_idx :u8,
	/// Contains the package data
	page_body :Vec<u8>,

	/// If there is a residue from previous pages in terms of a package spanning multiple
	/// pages, this field contains it. Having this Vec<Vec<u8>> and
	/// not Vec<u8> ensures to give us O(n) complexity, not O(n^2)
	/// for `n` as number of pages that the packet is contained in.
	last_overlap_pck :Vec<Vec<u8>>,
}

impl PageInfo {
	/// Returns `true` if the first "unread" packet is the first one
	/// in the page, `false` otherwise.
	fn is_first_pck_in_pg(self :&PageInfo) -> bool {
		return self.packet_idx == 0;
	}
	/// Returns `true` if the first "unread" packet is the last one
	/// in the page, `false` otherwise.
	/// If the first "unread" packet isn't completed in this page
	/// (spans page borders), this returns `false`.
	fn is_last_pck_in_pg(self :&PageInfo) -> bool {
		return ((self.packet_idx + 1 + (self.bi.ends_with_continued as u8)) as usize
			== self.bi.packet_positions.len());
	}
}

/**
Helper struct for parsing pages

Its created using the `new` function and then its fed more data via the `parse_segments`
and `parse_packet_data` functions, each called exactly once and in that precise order.

Then later code uses the struct's contents.
*/
pub struct PageParser {
	// Members packet_positions and packet_count
	// get populated after segments have been parsed
	bi :PageBaseInfo,

	stream_serial :u32,
	checksum :u32,
	header_buf: [u8; 27],
	/// Number of packet ending segments
	packet_count :u16, // Gets populated gafter segments have been parsed
	/// after segments have been parsed, this contains the segments buffer,
	/// after the packet data have been read, this contains the packets buffer.
	segments_or_packets_buf :Vec<u8>,
}

impl PageParser {
	/// Creates a new Page parser
	///
	/// The `header_buf` param contains the first 27 bytes of a new OGG page.
	/// Determining when one begins is your responsibility. Usually they
	/// begin directly after the end of a previous OGG page, but
	/// after you've performed a seek you might end up within the middle of a page
	/// and need to recapture.
	///
	/// Returns a page parser, and the requested size of the segments array.
	/// You should allocate and fill such an array, in order to pass it to the `parse_segments`
	/// function.
	pub fn new(header_buf :[u8; 27]) -> Result<(PageParser, usize), OggReadError> {
		let mut header_rdr = Cursor::new(header_buf);
		header_rdr.set_position(4);
		let stream_structure_version = try!(header_rdr.read_u8());
		if stream_structure_version != 0 {
			try!(Err(OggReadError::InvalidStreamStructVer(stream_structure_version)));
		}
		let header_type_flag = header_rdr.read_u8().unwrap();
		let stream_serial;

		Ok((PageParser {
			bi : PageBaseInfo {
				starts_with_continued : header_type_flag & 0x01u8 != 0,
				first_page : header_type_flag & 0x02u8 != 0,
				last_page : header_type_flag & 0x04u8 != 0,
				absgp : header_rdr.read_u64::<LittleEndian>().unwrap(),
				sequence_num : {
					stream_serial = header_rdr.read_u32::<LittleEndian>().unwrap();
					header_rdr.read_u32::<LittleEndian>().unwrap()
				},
				packet_positions : Vec::new(),
				ends_with_continued : false,
			},
			stream_serial,
			checksum : header_rdr.read_u32::<LittleEndian>().unwrap(),
			header_buf : header_buf,
			packet_count : 0,
			segments_or_packets_buf :Vec::new(),
		},
			// Number of page segments
			header_rdr.read_u8().unwrap() as usize
		))
	}

	/// Parses the segments buffer, and returns the requested size
	/// of the packets content array.
	pub fn parse_segments(&mut self, segments_buf :Vec<u8>) -> usize {
		let mut page_siz :u16 = 0; // Size of the page's body
		// Whether our page ends with a continued packet
		self.bi.ends_with_continued = self.bi.starts_with_continued;

		// First run: get the number of packets,
		// whether the page ends with a continued packet,
		// and the size of the page's body
		for val in &segments_buf {
			page_siz += *val as u16;
			// Increment by 1 if val < 255, otherwise by 0
			self.packet_count += (*val < 255) as u16;
			self.bi.ends_with_continued = !(*val < 255);
		}

		let mut packets = Vec::with_capacity(self.packet_count as usize
			+ self.bi.ends_with_continued as usize);
		let mut cur_packet_siz :u16 = 0;
		let mut cur_packet_offs :u16 = 0;

		// Second run: get the offsets of the packets
		// Not that we need it right now, but its much more fun this way, am I right
		for val in &segments_buf {
			cur_packet_siz += *val as u16;
			if *val < 255 {
				packets.push((cur_packet_offs, cur_packet_siz));
				cur_packet_offs += cur_packet_siz;
				cur_packet_siz = 0;
			}
		}
		if self.bi.ends_with_continued {
			packets.push((cur_packet_offs, cur_packet_siz));
		}

		self.bi.packet_positions = packets;
		self.segments_or_packets_buf = segments_buf;
		page_siz as usize
	}

	/// Parses the packets data and verifies the checksum.
	///
	/// Only after this function has been called (and before it `parse_segments`)
	/// you should pass on the `PageParser` to later code.
	pub fn parse_packet_data(&mut self, packet_data :Vec<u8>) -> Result<(), OggReadError> {
		// Now to hash calculation.
		// 1. Clear the header buffer
		self.header_buf[22] = 0;
		self.header_buf[23] = 0;
		self.header_buf[24] = 0;
		self.header_buf[25] = 0;

		// 2. Calculate the hash
		let mut hash_calculated :u32;
		hash_calculated = vorbis_crc32_update(0, &self.header_buf);
		hash_calculated = vorbis_crc32_update(hash_calculated,
			&self.segments_or_packets_buf);
		hash_calculated = vorbis_crc32_update(hash_calculated, &packet_data);

		// 3. Compare to the extracted one
		if self.checksum != hash_calculated {
			try!(Err(OggReadError::HashMismatch(self.checksum, hash_calculated)));
		}
		self.segments_or_packets_buf = packet_data;
		Ok(())
	}
}

/**
Low level struct for reading from an Ogg stream.

Note that most times you'll want the higher level `PageReader` struct.

It takes care of most of the internal parsing and logic, you
will only have to take care of handing over your data.

Essentially, it manages a cache of package data for each logical
bitstream, and when the cache of every logical bistream is empty,
it asks for a fresh page. You will then need to feed the struct
one via the `push_page` function.
*/
pub struct BasePacketReader {
	// TODO the hashmap plus the set is perhaps smart ass perfect design but could be made more performant I guess...
	// I mean: in > 99% of all cases we'll just have one or two streams.
	// AND: their setup changes only very rarely.

	/// Contains info about all logical streams that
	page_infos :HashMap<u32, PageInfo>,

	/// Contains the stream_serial of the stream that contains some unprocessed packet data.
	/// There is always <= 1, bc if there is one, no new pages will be read, so there is no chance for a second to be added
	/// None if there is no such stream and one has to read a new page.
	stream_with_stuff :Option<u32>,

	// Bool that is set to true when a seek of the stream has occured.
	// This helps validator code to decide whether to accept certain strange data.
	has_seeked :bool,
}

impl BasePacketReader {
	/// Constructs a new `BasePacketReader`.
	pub fn new() -> Self {
		BasePacketReader { page_infos: HashMap::new(),
			stream_with_stuff: None, has_seeked: false }
	}
	/// Extracts a packet from the cache, if the cache contains valid packet data,
	/// otherwise it returns `None`.
	///
	/// If this function returns `None`, you'll need to add a page to the cache
	/// by using the `push_page` function.
	pub fn read_packet(&mut self) -> Option<Packet> {
		if self.stream_with_stuff == None {
			return None;
		}
		let str_serial :u32 = self.stream_with_stuff.unwrap();
		let mut pg_info = self.page_infos.get_mut(&str_serial).unwrap();
		let (offs, len) = pg_info.bi.packet_positions[pg_info.packet_idx as usize];
		// If there is a continued packet, and we are at the start right now,
		// and we actually have its end in the current page, glue it together.
		let packet_content :Vec<u8> = if (
				pg_info.packet_idx == 0 && pg_info.bi.starts_with_continued
				&& !(pg_info.bi.ends_with_continued && pg_info.bi.packet_positions.len() == 1)
		) {
			// First find out the size of our spanning packet
			let mut siz :usize = 0;
			for pck in pg_info.last_overlap_pck.iter() {
				siz += pck.len();
			}
			siz += len as usize;
			let mut cont :Vec<u8> = Vec::with_capacity(siz);

			// Then do the copying
			for pck in pg_info.last_overlap_pck.iter() {
				cont.write_all(pck).unwrap();
			}
			// Now reset the overlap container again
			pg_info.last_overlap_pck = Vec::new();
			cont.write_all(&pg_info.page_body[offs as usize .. (offs + len) as usize]).unwrap();

			cont
		} else {
			let mut cont :Vec<u8> = Vec::with_capacity(len as usize);
			// TODO The copy below is totally unneccessary. It is only needed so that we don't have to carry around the old Vec's.
			// TODO get something like the shared_slice crate for RefCells, so that we can also have mutable data, shared through
			// slices.
			let cont_slice :&[u8] = &pg_info.page_body[offs as usize .. (offs + len) as usize];
			cont.write_all(cont_slice).unwrap();
			cont
		};

		let first_pck_in_pg = pg_info.is_first_pck_in_pg();
		let first_pck_overall = pg_info.bi.first_page && first_pck_in_pg;

		let last_pck_in_pg = pg_info.is_last_pck_in_pg();
		let last_pck_overall = pg_info.bi.last_page && last_pck_in_pg;

		// Update the last read index.
		pg_info.packet_idx += 1;
		// Set stream_with_stuff to None so that future packet reads
		// yield a page read first
		if last_pck_in_pg {
			self.stream_with_stuff = None;
		}

		return Some(Packet {
			data: packet_content,
			first_packet: first_pck_overall,
			last_packet: last_pck_overall,
			absgp_page: pg_info.bi.absgp,
			stream_serial: str_serial,
		});
	}

	/// Pushes a given Ogg page, updating the internal structures
	/// with its contents.
	pub fn push_page(&mut self, mut pg_prs :PageParser) -> Result<(), OggReadError> {
		match self.page_infos.entry(pg_prs.stream_serial) {
			Entry::Occupied(mut o) => {
				let inf = o.get_mut();
				if pg_prs.bi.first_page {
					try!(Err(OggReadError::InvalidData));
				}
				if pg_prs.bi.starts_with_continued != inf.bi.ends_with_continued {
					if !self.has_seeked {
						try!(Err(OggReadError::InvalidData));
					} else {
						// If we have seeked, we are more tolerant here,
						// and just drop the continued packet's content.

						inf.last_overlap_pck.clear();
						if pg_prs.bi.starts_with_continued {
							pg_prs.bi.packet_positions.remove(0);
							if pg_prs.packet_count != 0 {
								// Decrease packet count by one. Normal case.
								pg_prs.packet_count -= 1;
							} else {
								// If the packet count is 0, this means
								// that we start and end with the same continued packet.
								// So now as we ignore that packet, we must clear the
								// ends_with_continued state as well.
								pg_prs.bi.ends_with_continued = false;
							}
						}
					}
				} else if pg_prs.bi.starts_with_continued {
					// Remember the packet at the end so that it can be glued together once
					// we encounter the next segment with length < 255 (doesnt have to be in this page)
					let (offs, len) = inf.bi.packet_positions[inf.packet_idx as usize];
					if len as usize != inf.page_body.len() {
						let mut tmp = Vec::with_capacity(len as usize);
						tmp.write_all(&inf.page_body[offs as usize .. (offs + len) as usize]).unwrap();
						inf.last_overlap_pck.push(tmp);
					} else {
						// Little optimisation: don't copy if not neccessary
						inf.last_overlap_pck.push(replace(&mut inf.page_body, vec![0;0]));
					}

				}
				inf.bi = pg_prs.bi;
				inf.packet_idx = 0;
				inf.page_body = pg_prs.segments_or_packets_buf;
			},
			Entry::Vacant(v) => {
				if !self.has_seeked {
					if (!pg_prs.bi.first_page || pg_prs.bi.starts_with_continued) {
						// If we haven't seeked, this is an error.
						try!(Err(OggReadError::InvalidData));
					}
				} else {
					if !pg_prs.bi.first_page {
						// we can just ignore this.
					}
					if pg_prs.bi.starts_with_continued {
						// Ignore the continued packet's content.
						// This is a normal occurence if we have just seeked.
						pg_prs.bi.packet_positions.remove(0);
						if pg_prs.packet_count != 0 {
							// Decrease packet count by one. Normal case.
							pg_prs.packet_count -= 1;
						} else {
							// If the packet count is 0, this means
							// that we start and end with the same continued packet.
							// So now as we ignore that packet, we must clear the
							// ends_with_continued state as well.
							pg_prs.bi.ends_with_continued = false;
						}
					}
				}
				v.insert(PageInfo {
					bi : pg_prs.bi,
					packet_idx: 0,
					page_body: pg_prs.segments_or_packets_buf,
					last_overlap_pck: Vec::new(),
				});
			},
		}
		let pg_has_stuff :bool = pg_prs.packet_count > 0;

		if pg_has_stuff {
			self.stream_with_stuff = Some(pg_prs.stream_serial);
		} else {
			self.stream_with_stuff = None;
		}

		return Ok(());
	}

	/// Reset the internal state after a seek
	///
	/// It flushes the cache so that no partial data is left inside.
	/// It also tells the parsing logic to expect little inconsistencies
	/// due to the read position not being at the start.
	pub fn update_after_seek(&mut self) {
		self.stream_with_stuff = None;
		self.page_infos = HashMap::new();
		self.has_seeked = true;
	}
}

/**
Reader for packets from an Ogg stream.

This reads codec packets belonging to several different logical streams from one physical Ogg container stream.

If the `async` feature is activated, and you pass as internal reader a valid implementation of the
`AdvanceAndSeekBack` trait, like the `BufReader` wrapper, the PacketReader will support async operation,
meaning that its internal state doesn't get corrupted if from multiple consecutive reads which it performs,
some fail with e.g. the `WouldBlock` error kind.
*/
pub struct PacketReader<T :io::Read + io::Seek> {
	rdr :T,

	base_pck_rdr :BasePacketReader,
}

impl<T :io::Read + io::Seek> PacketReader <T> {
	/// Constructs a new `PacketReader` with a given `Read`.
	pub fn new(rdr :T) -> PacketReader<T> {
		PacketReader { rdr: rdr, base_pck_rdr : BasePacketReader::new() }
	}
	/// Returns the wrapped reader, consuming the `PacketReader`.
	pub fn into_inner(self) -> T {
		self.rdr
	}
	/// Reads a packet, and returns it on success.
	pub fn read_packet(&mut self) -> Result<Packet, OggReadError> {
		// Read pages until we got a valid entire packet
		// (packets may span multiple pages, so reading one page
		// doesn't always suffice to give us a valid packet)
		loop {
			if let Some(pck) = self.base_pck_rdr.read_packet() {
				return Ok(pck);
			}
			let page = try!(self.read_ogg_page());
			try!(self.base_pck_rdr.push_page(page));
		}
	}

	/// Reads until the new page header, and then returns the page header array.
	///
	/// If no new page header is immediately found, it performs a "recapture",
	/// meaning it searches for the capture pattern, and if it finds it, it
	/// reads the complete first 27 bytes of the header, and returns them.
	fn read_until_pg_header(&mut self) -> Result<[u8; 27], OggReadError> {
		let mut cpat_offs = 0;
		// Returns the Some(off), where off is the offset of the last byte
		// of the capture pattern if its found, None if the capture pattern
		// is not inside the passed slice.
		let mut check_arr = |arr :&[u8]| {
			for (i, ch) in arr.iter().enumerate() {
				match *ch {
					0x4f /*'O'*/ if cpat_offs == 0 => cpat_offs = 1,
					0x67 /*'g'*/ if cpat_offs == 1 || cpat_offs == 2 => cpat_offs += 1,
					0x53 /*'S'*/ if cpat_offs == 3 => return Some(i),
					_ => cpat_offs = 0,
				}
			}
			return None;
		};
		// First try the most normal case: The capture
		// pattern is at offset 0. No need for "recapture".
		let mut header_buf :[u8; 27] = [0; 27];
		try!(self.rdr.read_exact(&mut header_buf));
		match check_arr(&header_buf) {
			Some(idx) => if idx == 3 {
				// No need for recapture
				return Ok(header_buf);
			} else {
				// Capture pattern found, but offset by
				// a small amount.
				// Read remaining parts of the header,
				// then reassemble and return
				let mut ret_buf :[u8; 27] = [0; 27];
				let tocopy_len = header_buf.len() - idx;
				(&mut ret_buf[0..tocopy_len]).copy_from_slice(&header_buf[idx..]);
				try!(self.rdr.read_exact(&mut ret_buf[tocopy_len..]));
				return Ok(ret_buf);
			},
			None => (), // Not found, do nothing.
		}
		let mut read_amount = 27;
		// 150 kb gives us a bit of safety: we can survive
		// up to one page with a corrupted capture pattern
		// after having seeked right after a capture pattern
		// of an earlier page.
		let read_amount_max = 150 * 1024;
		let mut rdr_buf :[u8; 1024] = [0; 1024];
		loop {
			let rd_len = try!(self.rdr.read(&mut rdr_buf));
			read_amount += rd_len;
			if rd_len == 0 {
				// Reached EOF.
				try!(Err(OggReadError::NoCapturePatternFound));
			}
			match check_arr(&rdr_buf[0..rd_len]) {
				Some(off) => {
					let mut ret_buf :[u8; 27] = [0; 27];
					ret_buf[0] = 0x4f; // 'O'
					ret_buf[1] = 0x67; // 'g'
					ret_buf[2] = 0x67; // 'g'
					ret_buf[3] = 0x53; // 'S' (Not actually needed)
					let cnt_from_rdr_buf = rdr_buf.len() - off;
					use std::cmp::min;
					let copy_amount = min(24, cnt_from_rdr_buf);
					(&mut ret_buf[3..copy_amount + 3])
						.copy_from_slice(&rdr_buf[off..copy_amount + off]);
					if cnt_from_rdr_buf > 24 {
						// We have read too much content (exceeding the header).
						// Seek back so that we are at the position
						// right after the header.
						try!(self.seek_back(cnt_from_rdr_buf - 24));
					} else if cnt_from_rdr_buf == 24 {
						// All is fine. Nothing has to be done.
					} else {
						// We still have to read some content.
						try!(self.rdr.read_exact(
							&mut ret_buf[cnt_from_rdr_buf + 3..]));
					}
					return Ok(ret_buf);
				},
				None => (), // Nothing found.
			}
			if read_amount > read_amount_max {
				// Exhaustive searching for the capture pattern
				// has returned no ogg capture pattern.
				try!(Err(OggReadError::NoCapturePatternFound));
			}
		}
	}

	/// Parses and reads a new OGG page
	///
	/// Returns a fully complete `PageParser`
	///
	/// To support seeking this does not assume that the capture pattern
	/// is at the current reader position.
	/// Instead it searches until it finds the capture pattern.
	fn read_ogg_page(&mut self) -> Result<PageParser, OggReadError> {
		let header_buf :[u8; 27] = try!(self.read_until_pg_header());
		let (mut pg_prs, page_segments) = try!(PageParser::new(header_buf));

		let mut segments_buf = vec![0; page_segments]; // TODO fix this, we initialize memory for NOTHING!!! Out of some reason, this is seen as "unsafe" by rustc.
		try!(self.rdr.read_exact(&mut segments_buf));

		let page_siz = pg_prs.parse_segments(segments_buf);

		let mut packet_data = vec![0; page_siz as usize];
		try!(self.rdr.read_exact(&mut packet_data));

		self.maybe_advance();

		try!(pg_prs.parse_packet_data(packet_data));
		Ok(pg_prs)
	}

	#[cfg(feature = "async")]
	fn maybe_advance(&mut self) {
		trait MaybeAdvance {
			fn maybe_advance(&mut self);
		}
		impl<T> MaybeAdvance for T {
			default fn maybe_advance(&mut self) { }
		}
		impl<T :AdvanceAndSeekBack> MaybeAdvance for T {
			fn maybe_advance(&mut self) {
				self.advance();
			}
		}
		self.rdr.maybe_advance();
	}

	#[cfg(not(feature = "async"))]
	fn maybe_advance(&mut self) {
		// Do nothing ...
	}

	#[cfg(feature = "async")]
	fn seek_back(&mut self, len :usize) -> io::Result<()> {
		trait MaybeAdvance {
			fn seek_back(&mut self, len :usize) -> io::Result<()>;
		}
		impl<T :Seek> MaybeAdvance for T {
			default fn seek_back(&mut self, len :usize) -> io::Result<()> {
				return match self.seek(SeekFrom::Current(-(len as i64))) {
					Ok(_) => Ok(()),
					Err(e) => Err(e),
				};
			}
		}
		impl<T :Seek + AdvanceAndSeekBack> MaybeAdvance for T {
			fn seek_back(&mut self, len :usize) -> io::Result<()> {
				self.seek_back(len);
				return Ok(());
			}
		}
		return self.rdr.seek_back(len);
	}

	#[cfg(not(feature = "async"))]
	fn seek_back(&mut self, len :usize) -> io::Result<()> {
		return match self.rdr.seek(SeekFrom::Current(-(len as i64))) {
			Ok(_) => Ok(()),
			Err(e) => Err(e),
		};
	}

	/// Seeks the underlying reader
	///
	/// Seeks the reader that this PacketReader bases on by the specified
	/// number of bytes. All new pages will be read from the new position.
	///
	/// This also flushes all the unread packets in the queue.
	pub fn seek_bytes(&mut self, pos :SeekFrom) -> Result<u64, Error> {
		let r = try!(self.rdr.seek(pos));
		// Reset the internal state
		self.base_pck_rdr.update_after_seek();
		return Ok(r);
	}
}
