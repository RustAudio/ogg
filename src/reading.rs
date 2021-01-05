// Ogg decoder and encoder written in Rust
//
// Copyright (c) 2016-2017 est31 <MTest31@outlook.com>
// and contributors. All rights reserved.
// Redistribution or use only under the terms
// specified in the LICENSE file attached to this
// source distribution.

/*!
Reading logic
*/

use std::error;
use std::io;
use std::io::{Cursor, Read, Write, SeekFrom, Error, ErrorKind};
use byteorder::{ReadBytesExt, LittleEndian};
use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::fmt::{Display, Formatter, Error as FmtError};
use std::mem::replace;
use crc::vorbis_crc32_update;
use Packet;
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

impl OggReadError {
	fn description_str(&self) -> &str {
		match *self {
			OggReadError::NoCapturePatternFound => "No Ogg capture pattern found",
			OggReadError::InvalidStreamStructVer(_) =>
				"A non zero stream structure version was passed",
			OggReadError::HashMismatch(_, _) => "CRC32 hash mismatch",
			OggReadError::ReadError(_) => "I/O error",
			OggReadError::InvalidData => "Constraint violated",
		}
	}
}

impl error::Error for OggReadError {
	fn description(&self) -> &str {
		self.description_str()
	}

	fn cause(&self) -> Option<&dyn error::Error> {
		match *self {
			OggReadError::ReadError(ref err) => Some(err as &dyn error::Error),
			_ => None
		}
	}
}

impl Display for OggReadError {
	fn fmt(&self, fmt :&mut Formatter) -> Result<(), FmtError> {
		write!(fmt, "{}", Self::description_str(self))
	}
}

impl From<io::Error> for OggReadError {
	fn from(err :io::Error) -> OggReadError {
		return OggReadError::ReadError(err);
	}
}

/// Containing information about an OGG page that is shared between multiple places
struct PageBaseInfo {
	/// `true`: the first packet is continued from the page before. `false`: if it's a "fresh" one
	starts_with_continued :bool,
	/// `true` if this page is the first one in the logical bitstream
	first_page :bool,
	/// `true` if this page is the last one in the logical bitstream
	last_page :bool,
	/// Absolute granule position. The codec defines further meaning.
	absgp :u64,
	/// Page counter
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
	fn is_first_pck_in_pg(&self) -> bool {
		return self.packet_idx == 0;
	}
	/// Returns `true` if the first "unread" packet is the last one
	/// in the page, `false` otherwise.
	/// If the first "unread" packet isn't completed in this page
	/// (spans page borders), this returns `false`.
	fn is_last_pck_in_pg(&self) -> bool {
		return (self.packet_idx + 1 + (self.bi.ends_with_continued as u8)) as usize
			== self.bi.packet_positions.len();
	}
}

/// Contains a fully parsed OGG page.
pub struct OggPage(PageParser);

impl OggPage {
	/// Returns whether there is an ending packet in the page
	fn has_packet_end(&self) -> bool {
		(self.0.bi.packet_positions.len() -
			self.0.bi.ends_with_continued as usize) > 0
	}
	/// Returns whether there is a packet that both
	/// starts and ends inside the page
	fn has_whole_packet(&self) -> bool {
		self.0.bi.packet_positions.len().saturating_sub(
			self.0.bi.ends_with_continued as usize +
			self.0.bi.starts_with_continued as usize) > 0
	}
	/// Returns whether there is a starting packet in the page
	fn has_packet_start(&self) -> bool {
		(self.0.bi.packet_positions.len() -
			self.0.bi.starts_with_continued as usize) > 0
	}
}

/**
Helper struct for parsing pages

It's created using the `new` function and then it's fed more data via the `parse_segments`
and `parse_packet_data` functions, each called exactly once and in that precise order.

Then later code uses the `OggPage` returned by the `parse_packet_data` function.
*/
pub struct PageParser {
	// Members packet_positions, ends_with_continued and packet_count
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
		let stream_structure_version = tri!(header_rdr.read_u8());
		if stream_structure_version != 0 {
			tri!(Err(OggReadError::InvalidStreamStructVer(stream_structure_version)));
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
			header_buf,
			packet_count : 0,
			segments_or_packets_buf :Vec::new(),
		},
			// Number of page segments
			header_rdr.read_u8().unwrap() as usize
		))
	}

	/// Parses the segments buffer, and returns the requested size
	/// of the packets content array.
	///
	/// You should allocate and fill such an array, in order to pass it to the `parse_packet_data`
	/// function.
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
		// Not that we need it right now, but it's much more fun this way, am I right
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
	/// Returns an `OggPage` to be used by later code.
	pub fn parse_packet_data(mut self, packet_data :Vec<u8>) ->
			Result<OggPage, OggReadError> {
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
			// Do not verify checksum when the decoder is being fuzzed.
			// This allows random input from fuzzers reach decoding code that's actually interesting,
			// instead of being rejected early due to checksum mismatch.
			if !cfg!(fuzzing) {
				tri!(Err(OggReadError::HashMismatch(self.checksum, hash_calculated)));
			}
		}
		self.segments_or_packets_buf = packet_data;
		Ok(OggPage(self))
	}
}

/**
Low level struct for reading from an Ogg stream.

Note that most times you'll want the higher level `PacketReader` struct.

It takes care of most of the internal parsing and logic, you
will only have to take care of handing over your data.

Essentially, it manages a cache of package data for each logical
bitstream, and when the cache of every logical bistream is empty,
it asks for a fresh page. You will then need to feed the struct
one via the `push_page` function.

All functions on this struct are async ready.
They get their data fed, instead of calling and blocking
in order to get it.
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
	/// Constructs a new blank `BasePacketReader`.
	///
	/// You can feed it data using the `push_page` function, and
	/// obtain data using the `read_packet` function.
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
		let pg_info = self.page_infos.get_mut(&str_serial).unwrap();
		let (offs, len) = pg_info.bi.packet_positions[pg_info.packet_idx as usize];
		// If there is a continued packet, and we are at the start right now,
		// and we actually have its end in the current page, glue it together.
		let need_to_glue = pg_info.packet_idx == 0 &&
				pg_info.bi.starts_with_continued &&
				!(pg_info.bi.ends_with_continued && pg_info.bi.packet_positions.len() == 1);
		let packet_content :Vec<u8> = if need_to_glue {
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
			first_packet_pg: first_pck_in_pg,
			first_packet_stream: first_pck_overall,
			last_packet_pg: last_pck_in_pg,
			last_packet_stream: last_pck_overall,
			absgp_page: pg_info.bi.absgp,
			stream_serial: str_serial,
		});
	}

	/// Pushes a given Ogg page, updating the internal structures
	/// with its contents.
	///
	/// If you want the code to function properly, you should first call
	/// `parse_segments`, then `parse_packet_data` on a `PageParser`
	/// before passing the resulting `OggPage` to this function.
	pub fn push_page(&mut self, page :OggPage) -> Result<(), OggReadError> {
		let mut pg_prs = page.0;
		match self.page_infos.entry(pg_prs.stream_serial) {
			Entry::Occupied(mut o) => {
				let inf = o.get_mut();
				if pg_prs.bi.first_page {
					tri!(Err(OggReadError::InvalidData));
				}
				if pg_prs.bi.starts_with_continued != inf.bi.ends_with_continued {
					if !self.has_seeked {
						tri!(Err(OggReadError::InvalidData));
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
					if !pg_prs.bi.first_page || pg_prs.bi.starts_with_continued {
						// If we haven't seeked, this is an error.
						tri!(Err(OggReadError::InvalidData));
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
						// Not actually needed, but good for consistency
						pg_prs.bi.starts_with_continued = false;
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

#[derive(Clone, Copy)]
enum UntilPageHeaderReaderMode {
	Searching,
	FoundWithNeeded(u8),
	SeekNeeded(i32),
	Found,
}

enum UntilPageHeaderResult {
	Eof,
	Found,
	ReadNeeded,
	SeekNeeded,
}

struct UntilPageHeaderReader {
	mode :UntilPageHeaderReaderMode,
	/// Capture pattern offset. Needed so that if we only partially
	/// recognized the capture pattern, we later on only check the
	/// remaining part.
	cpt_of :u8,
	/// The return buffer.
	ret_buf :[u8; 27],
	read_amount :usize,
}

impl UntilPageHeaderReader {
	pub fn new() -> Self {
		UntilPageHeaderReader {
			mode : UntilPageHeaderReaderMode::Searching,
			cpt_of : 0,
			ret_buf : [0; 27],
			read_amount : 0,
		}
	}
	/// Returns Some(off), where off is the offset of the last byte
	/// of the capture pattern if it's found, None if the capture pattern
	/// is not inside the passed slice.
	///
	/// Changes the capture pattern offset accordingly
	fn check_arr(&mut self, arr :&[u8]) -> Option<usize> {
		for (i, ch) in arr.iter().enumerate() {
			match *ch {
				b'O' => self.cpt_of = 1,
				b'g' if self.cpt_of == 1 || self.cpt_of == 2 => self.cpt_of += 1,
				b'S' if self.cpt_of == 3 => return Some(i),
				_ => self.cpt_of = 0,
			}
		}
		return None;
	}
	/// Do one read exactly, and if it was successful,
	/// return Ok(true) if the full header has been read and can be extracted with
	///
	/// or return Ok(false) if the
	pub fn do_read<R :Read>(&mut self, mut rdr :R)
			-> Result<UntilPageHeaderResult, OggReadError> {
		use self::UntilPageHeaderReaderMode::*;
		use self::UntilPageHeaderResult as Res;
		// The array's size is freely choseable, but must be > 27,
		// and must well fit into an i32 (needs to be stored in SeekNeeded)
		let mut buf :[u8; 1024] = [0; 1024];

		let rd_len = tri!(rdr.read(if self.read_amount < 27 {
			// This is an optimisation for the most likely case:
			// the next page directly follows the current read position.
			// Then it would be a waste to read more than the needed amount.
			&mut buf[0 .. 27 - self.read_amount]
		} else {
			match self.mode {
				Searching => &mut buf,
				FoundWithNeeded(amount) => &mut buf[0 .. amount as usize],
				SeekNeeded(_) => return Ok(Res::SeekNeeded),
				Found => return Ok(Res::Found),
			}
		}));

		if rd_len == 0 {
			// Reached EOF.
			if self.read_amount == 0 {
				// If we have read nothing yet, there is no data
				// but ogg data, meaning the stream ends legally
				// and without corruption.
				return Ok(Res::Eof);
			} else {
				// There is most likely a corruption here.
				// I'm not sure, but the ogg spec doesn't say that
				// random data past the last ogg page is allowed,
				// so we just assume it's not allowed.
				tri!(Err(OggReadError::NoCapturePatternFound));
			}
		}
		self.read_amount += rd_len;

		// 150 kb gives us a bit of safety: we can survive
		// up to one page with a corrupted capture pattern
		// after having seeked right after a capture pattern
		// of an earlier page.
		let read_amount_max = 150 * 1024;
		if self.read_amount > read_amount_max {
			// Exhaustive searching for the capture pattern
			// has returned no ogg capture pattern.
			tri!(Err(OggReadError::NoCapturePatternFound));
		}

		let rd_buf = &buf[0 .. rd_len];

		use std::cmp::min;
		let (off, needed) = match self.mode {
			Searching => match self.check_arr(rd_buf) {
				// Capture pattern found
				Some(off) => {
					self.ret_buf[0] = b'O';
					self.ret_buf[1] = b'g';
					self.ret_buf[2] = b'g';
					self.ret_buf[3] = b'S'; // (Not actually needed)
					(off, 24)
				},
				// Nothing found
				None => return Ok(Res::ReadNeeded),
			},
			FoundWithNeeded(needed) => {
				(0, needed as usize)
			},
			_ => unimplemented!(),
		};

		let fnd_buf = &rd_buf[off..];

		let copy_amount = min(needed, fnd_buf.len());
		let start_fill = 27 - needed;
		(&mut self.ret_buf[start_fill .. copy_amount + start_fill])
				.copy_from_slice(&fnd_buf[0 .. copy_amount]);
		if fnd_buf.len() == needed {
			// Capture pattern found!
			self.mode = Found;
			return Ok(Res::Found);
		} else if fnd_buf.len() < needed {
			// We still have to read some content.
			let needed_new = needed - copy_amount;
			self.mode = FoundWithNeeded(needed_new as u8);
			return Ok(Res::ReadNeeded);
		} else {
			// We have read too much content (exceeding the header).
			// Seek back so that we are at the position
			// right after the header.

			self.mode = SeekNeeded(needed as i32 - fnd_buf.len() as i32);
			return Ok(Res::SeekNeeded);
		}
	}
	pub fn do_seek<S :Seek>(&mut self, mut skr :S)
			-> Result<UntilPageHeaderResult, OggReadError> {
		use self::UntilPageHeaderReaderMode::*;
		use self::UntilPageHeaderResult as Res;
		match self.mode {
			Searching | FoundWithNeeded(_) => Ok(Res::ReadNeeded),
			SeekNeeded(offs) => {
				tri!(skr.seek(SeekFrom::Current(offs as i64)));
				self.mode = Found;
				Ok(Res::Found)
			},
			Found => Ok(Res::Found),
		}
	}
	pub fn into_header(self) -> [u8; 27] {
		use self::UntilPageHeaderReaderMode::*;
		match self.mode {
			Found => self.ret_buf,
			_ => panic!("wrong mode"),
		}
	}
}

/**
Reader for packets from an Ogg stream.

This reads codec packets belonging to several different logical streams from one physical Ogg container stream.

This reader is not async ready. It does not keep its internal state
consistent when it encounters the `WouldBlock` error kind.
If you desire async functionality, consider enabling the `async` feature
and look into the async module.
*/
pub struct PacketReader<T :io::Read + io::Seek> {
	rdr :T,

	base_pck_rdr :BasePacketReader,
}

impl<T :io::Read + io::Seek> PacketReader<T> {
	/// Constructs a new `PacketReader` with a given `Read`.
	pub fn new(rdr :T) -> PacketReader<T> {
		PacketReader { rdr, base_pck_rdr : BasePacketReader::new() }
	}
	/// Returns the wrapped reader, consuming the `PacketReader`.
	pub fn into_inner(self) -> T {
		self.rdr
	}
	/// Reads a packet, and returns it on success.
	///
	/// Ok(None) is returned if the physical stream has ended.
	pub fn read_packet(&mut self) -> Result<Option<Packet>, OggReadError> {
		// Read pages until we got a valid entire packet
		// (packets may span multiple pages, so reading one page
		// doesn't always suffice to give us a valid packet)
		loop {
			if let Some(pck) = self.base_pck_rdr.read_packet() {
				return Ok(Some(pck));
			}
			let page = tri!(self.read_ogg_page());
			match page {
				Some(page) => tri!(self.base_pck_rdr.push_page(page)),
				None => return Ok(None),
			}
		}
	}
	/// Reads a packet, and returns it on success.
	///
	/// The difference to the `read_packet` function is that this function
	/// returns an Err(_) if the physical stream has ended.
	/// This function is useful if you expect a new packet to come.
	pub fn read_packet_expected(&mut self) -> Result<Packet, OggReadError> {
		match tri!(self.read_packet()) {
			Some(p) => Ok(p),
			None => tri!(Err(Error::new(ErrorKind::UnexpectedEof,
				"Expected ogg packet but found end of physical stream"))),
		}
	}

	/// Reads until the new page header, and then returns the page header array.
	///
	/// If no new page header is immediately found, it performs a "recapture",
	/// meaning it searches for the capture pattern, and if it finds it, it
	/// reads the complete first 27 bytes of the header, and returns them.
	///
	/// Ok(None) is returned if the stream has ended without an uncompleted page
	/// or non page data after the last page (if any) present.
	fn read_until_pg_header(&mut self) -> Result<Option<[u8; 27]>, OggReadError> {
		let mut r = UntilPageHeaderReader::new();
		use self::UntilPageHeaderResult::*;
		let mut res = tri!(r.do_read(&mut self.rdr));
		loop {
			res = match res {
				Eof => return Ok(None),
				Found => break,
				ReadNeeded => tri!(r.do_read(&mut self.rdr)),
				SeekNeeded => tri!(r.do_seek(&mut self.rdr))
			}
		}
		Ok(Some(r.into_header()))
	}

	/// Parses and reads a new OGG page
	///
	/// To support seeking this does not assume that the capture pattern
	/// is at the current reader position.
	/// Instead it searches until it finds the capture pattern.
	fn read_ogg_page(&mut self) -> Result<Option<OggPage>, OggReadError> {
		let header_buf :[u8; 27] = match tri!(self.read_until_pg_header()) {
			Some(s) => s,
			None => return Ok(None)
		};
		let (mut pg_prs, page_segments) = tri!(PageParser::new(header_buf));

		let mut segments_buf = vec![0; page_segments]; // TODO fix this, we initialize memory for NOTHING!!! Out of some reason, this is seen as "unsafe" by rustc.
		tri!(self.rdr.read_exact(&mut segments_buf));

		let page_siz = pg_prs.parse_segments(segments_buf);

		let mut packet_data = vec![0; page_siz as usize];
		tri!(self.rdr.read_exact(&mut packet_data));

		Ok(Some(tri!(pg_prs.parse_packet_data(packet_data))))
	}

	/// Seeks the underlying reader
	///
	/// Seeks the reader that this PacketReader bases on by the specified
	/// number of bytes. All new pages will be read from the new position.
	///
	/// This also flushes all the unread packets in the queue.
	pub fn seek_bytes(&mut self, pos :SeekFrom) -> Result<u64, Error> {
		let r = tri!(self.rdr.seek(pos));
		// Reset the internal state
		self.base_pck_rdr.update_after_seek();
		return Ok(r);
	}

	/// Seeks to absolute granule pos
	///
	/// More specifically, it seeks to the first Ogg page
	/// that has an `absgp` greater or equal to the specified one.
	/// In the case of continued packets, the seek operation may also end up
	/// at the last page that comes before such a page and has a packet start.
	///
	/// The passed `stream_serial` parameter controls the stream
	/// serial number to filter our search for. If it's `None`, no
	/// filtering is applied, but if it is `Some(n)`, we filter for
	/// streams with the serial number `n`.
	/// Note that the `None` case is only intended for streams
	/// where only one logical stream exists, the seek may misbehave
	/// if `Ǹone` gets passed when multiple streams exist.
	///
	/// The returned bool indicates whether the seek was successful.
	pub fn seek_absgp(&mut self, stream_serial :Option<u32>,
			pos_goal :u64) -> Result<bool, OggReadError> {
		macro_rules! found {
			($pos:expr) => {{
				// println!("found: {}", $pos);
				tri!(self.rdr.seek(SeekFrom::Start($pos)));
				self.base_pck_rdr.update_after_seek();
				return Ok(true);
			}};
		}
		macro_rules! bt {
			($e:expr) => {{
				match tri!($e) {
					Some(s) => s,
					None => return Ok(false),
				}
			}};
		}
		// The task of this macro is to read to the
		// end of the logical stream. For optimisation reasons,
		// it returns early if we found our goal
		// or any page past it.
		macro_rules! pg_read_until_end_or_goal {
			{$goal:expr} => {{
				let mut pos;
				let mut pg;
				loop {
					let (n_pos, n_pg) = pg_read_match_serial!();
					pos = n_pos;
					pg = n_pg;
					// If the absgp matches our goal, the seek process is done.
					// This is a nice shortcut as we don't need to perform
					// the remainder of the seek process any more.
					// Of course, an exact match only happens in the fewest
					// of cases
					if pg.0.bi.absgp == $goal {
						found!(pos);
					}
					// If we found a page past our goal, we already
					// found a position that can serve as end post of the search.
					if pg.0.bi.absgp > $goal {
						break;
					}
					// Stop the search if the stream has ended.
					if pg.0.bi.last_page {
						return Ok(false)
					}
					// If the page is not interesting, seek over it.
				}
				(pos, pg)
			}};
		}
		macro_rules! pg_read_match_serial {
			{} => {{
				let mut pos;
				let mut pg;
				let mut continued_pck_start = None;
				loop {
					pos = tri!(self.rdr.seek(SeekFrom::Current(0)));
					pg = bt!(self.read_ogg_page());
					/*println!("absgp {} serial {} wh {} pe {} @ {}",
						pg.0.bi.absgp, pg.0.bi.sequence_num,
						pg.has_whole_packet(), pg.has_packet_end(), pos);// */

					match stream_serial {
						// Continue the search if we encounter a
						// page with a different stream serial
						Some(s) if pg.0.stream_serial != s => (),
						_ => match continued_pck_start {
								None if pg.has_whole_packet() => break,
								None if pg.has_packet_start() => {
									continued_pck_start = Some(pos);
								},
								Some(s) if pg.has_packet_end() => {
									// We have remembered a packet start,
									// and have just encountered a packet end.
									// Return the position of the start with the
									// info from the end (for the absgp).
									pos = s;
									break;
								},
								_ => (),
						},
					}
				}
				(pos, pg)
			}};
		}

		// Bisect seeking algo.
		// Start by finding boundaries, e.g. at the start and
		// end of the file, then bisect those boundaries successively
		// until a page is found.

		//println!("seek start. goal = {}", pos_goal);
		let ab_of = |pg :&OggPage| { pg.0.bi.absgp };
		let seq_of = |pg :&OggPage| { pg.0.bi.sequence_num };

		// First, find initial "boundaries"
		// Seek to the start of the file to get the starting boundary
		tri!(self.rdr.seek(SeekFrom::Start(0)));
		let (mut begin_pos, mut begin_pg) = pg_read_match_serial!();

		// If the goal is the beginning, we are done.
		if pos_goal == 0 {
			//println!("Seeking to the beginning of the stream - skipping bisect.");
			found!(begin_pos);
		}

		// Seek to the end of the file to get the ending boundary
		// TODO the 200 KB is just a guessed number, any ideas
		// to improve it?
		tri!(seek_before_end(&mut self.rdr, 200 * 1024));
		let (mut end_pos, mut end_pg) = pg_read_until_end_or_goal!(pos_goal);

		// Then perform the bisection
		loop {
			// Search is done if the two limits are the same page,
			// or consecutive pages.
			if seq_of(&end_pg) - seq_of(&begin_pg) <= 1 {
				found!(end_pos);
			}
			// Perform the bisection step
			let pos_to_seek = begin_pos + (end_pos - begin_pos) / 2;
			tri!(self.rdr.seek(SeekFrom::Start(pos_to_seek)));
			let (pos, pg) = pg_read_match_serial!();
			/*println!("seek {} {} . {} @ {} {} . {}",
				ab_of(&begin_pg), ab_of(&end_pg), ab_of(&pg),
				begin_pos, end_pos, pos);// */

			if seq_of(&end_pg) == seq_of(&pg) ||
					seq_of(&begin_pg) == seq_of(&pg) {
				//println!("switching to linear.");
				// The bisection seek doesn't bring us any further.
				// Switch to a linear seek to get the last details.
				let mut pos;
				let mut pg;
				let mut last_packet_end_pos = begin_pos;
				tri!(self.rdr.seek(SeekFrom::Start(begin_pos)));
				loop {
					pos = tri!(self.rdr.seek(SeekFrom::Current(0)));
					pg = bt!(self.read_ogg_page());
					/*println!("absgp {} pck_start {} whole_pck {} pck_end {} @ {} {}",
						ab_of(&pg), pg.has_packet_start(), pg.has_whole_packet(),
						pg.has_packet_end(),
						pos, last_packet_end_pos);// */
					match stream_serial {
						// Continue the search if we encounter a
						// page with a different stream serial,
						// or one with an absgp of -1.
						Some(s) if pg.0.stream_serial != s => (),
						_ if ab_of(&pg) == -1i64 as u64 => (),
						// The page is found if the absgp is >= our goal
						_ if ab_of(&pg) >= pos_goal => found!(last_packet_end_pos),
						// If we encounter a page with a packet start,
						// update accordingly.
						_ => if pg.has_packet_end() {
							last_packet_end_pos = pos;
						},
					}
				}
			}
			if ab_of(&pg) >= pos_goal {
				end_pos = pos;
				end_pg = pg;
			} else {
				begin_pos = pos;
				begin_pg = pg;
			}
		}
	}
	/// Resets the internal state by deleting all
	/// unread packets.
	pub fn delete_unread_packets(&mut self) {
		self.base_pck_rdr.update_after_seek();
	}
}

// util function
fn seek_before_end<T :io::Read + io::Seek>(mut rdr :T,
		offs :u64) -> Result<u64, OggReadError> {
	let end_pos = tri!(rdr.seek(SeekFrom::End(0)));
	let end_pos_to_seek = ::std::cmp::min(end_pos, offs);
	return Ok(tri!(rdr.seek(SeekFrom::End(-(end_pos_to_seek as i64)))));
}

#[cfg(feature = "async")]
/**
Asyncronous ogg decoding
*/
pub mod async_api {
	#![allow(deprecated)]

	use super::*;
	use tokio_io::AsyncRead;
	use tokio_io::codec::{Decoder, FramedRead};
	use futures::stream::Stream;
	use futures::{Async, Poll};
	use bytes::BytesMut;

	enum PageDecodeState {
		Head,
		Segments(PageParser, usize),
		PacketData(PageParser, usize),
		InUpdate,
	}

	impl PageDecodeState {
		fn needed_size(&self) -> usize {
			match self {
				&PageDecodeState::Head => 27,
				&PageDecodeState::Segments(_, s) => s,
				&PageDecodeState::PacketData(_, s) => s,
				&PageDecodeState::InUpdate => panic!("invalid state"),
			}
		}
	}

	/**
	Async page reading functionality.
	*/
	struct PageDecoder {
		state : PageDecodeState,
	}

	impl PageDecoder {
		fn new() -> Self {
			PageDecoder {
				state : PageDecodeState::Head,
			}
		}
	}

	impl Decoder for PageDecoder {
		type Item = OggPage;
		type Error = OggReadError;

		fn decode(&mut self, buf :&mut BytesMut) ->
				Result<Option<OggPage>, OggReadError> {
			use self::PageDecodeState::*;
			loop {
				let needed_size = self.state.needed_size();
				if buf.len() < needed_size {
					return Ok(None);
				}
				let mut ret = None;
				let consumed_buf = buf.split_to(needed_size).to_vec();

				self.state = match ::std::mem::replace(&mut self.state, InUpdate) {
					Head => {
						let mut hdr_buf = [0; 27];
						// TODO once we have const generics, the copy below can be done
						// much nicer, maybe with a new into_array fn on Vec's
						hdr_buf.copy_from_slice(&consumed_buf);
						let tup = tri!(PageParser::new(hdr_buf));
						Segments(tup.0, tup.1)
					},
					Segments(mut pg_prs, _) => {
						let new_needed_len = pg_prs.parse_segments(consumed_buf);
						PacketData(pg_prs, new_needed_len)
					},
					PacketData(pg_prs, _) => {
						ret = Some(tri!(pg_prs.parse_packet_data(consumed_buf)));
						Head
					},
					InUpdate => panic!("invalid state"),
				};
				if ret.is_some() {
					return Ok(ret);
				}
			}
		}

		fn decode_eof(&mut self, buf :&mut BytesMut) ->
				Result<Option<OggPage>, OggReadError> {
			// Ugly hack for "bytes remaining on stream" error
			return self.decode(buf);
		}
	}

	/**
	Async packet reading functionality.
	*/
	pub struct PacketReader<T> where T :AsyncRead {
		base_pck_rdr :BasePacketReader,
		pg_rd :FramedRead<T, PageDecoder>,
	}

	impl<T :AsyncRead> PacketReader<T> {
		pub fn new(inner :T) -> Self {
			PacketReader {
				base_pck_rdr : BasePacketReader::new(),
				pg_rd : FramedRead::new(inner, PageDecoder::new()),
			}
		}
	}

	impl<T :AsyncRead> Stream for PacketReader<T> {
		type Item = Packet;
		type Error = OggReadError;

		fn poll(&mut self) -> Poll<Option<Packet>, OggReadError> {
			// Read pages until we got a valid entire packet
			// (packets may span multiple pages, so reading one page
			// doesn't always suffice to give us a valid packet)
			loop {
				if let Some(pck) = self.base_pck_rdr.read_packet() {
					return Ok(Async::Ready(Some(pck)));
				}
				let page = try_ready!(self.pg_rd.poll());
				match page {
					Some(page) => tri!(self.base_pck_rdr.push_page(page)),
					None => return Ok(Async::Ready(None)),
				}
			}
		}
	}

}
