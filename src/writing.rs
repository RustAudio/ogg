// Ogg decoder and encoder written in Rust
//
// Copyright (c) 2016-2017 est31 <MTest31@outlook.com>
// and contributors. All rights reserved.
// Redistribution or use only under the terms
// specified in the LICENSE file attached to this
// source distribution.

/*!
Writing logic
*/

use std::borrow::Cow;
use std::io::{Write, Seek, SeekFrom, Result};
use std::collections::HashMap;
use crate::crc::vorbis_crc32_update;

/// Returns `false` if the specified expression is false.
macro_rules! bail_out_on_fail {
	($success:expr) => {
		if !$success {
			return false;
		}
	}
}

/**
Writer for packets into an Ogg stream.

Note that the functionality of this struct isn't as well tested as for
the `PacketReader` struct.
*/
pub struct PacketWriter<'writer, T :Write> {
	wtr :T,

	base_pck_wtr :BasePacketWriter<'writer>,
}

/// Internal base packet writer that contains common packet writing logic.
struct BasePacketWriter<'writer> {
	page_vals :HashMap<u32, CurrentPageValues<'writer>>,
}

struct CurrentPageValues<'writer> {
	/// `true` if this page is the first one in the logical bitstream
	first_page :bool,
	/// Page counter of the current page
	/// Increased for every page
	sequence_num :u32,

	/// Points to the first unwritten position in cur_pg_lacing.
	segment_cnt :u8,
	cur_pg_lacing :[u8; 255],
	/// The data and the absgp's of the packets
	cur_pg_data :Vec<(Cow<'writer, [u8]>, u64)>,

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

impl<'writer> BasePacketWriter<'writer> {
	fn new() -> Self {
		Self {
			page_vals : HashMap::new(),
		}
	}

	fn write_packet(&mut self, pck_cont :Cow<'writer, [u8]>, serial :u32,
					inf :PacketWriteEndInfo, absgp :u64,
					mut sink_func :impl FnMut(&[u8]) -> bool) -> bool {
		let is_end_stream = inf == PacketWriteEndInfo::EndStream;
		let pg = self.page_vals.entry(serial).or_insert(
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

		let cont_len = pck_cont.len();
		pg.cur_pg_data.push((pck_cont, absgp));

		let last_data_segment_size = (cont_len % 255) as u8;
		let needed_segments = (cont_len / 255) + 1;
		let mut segment_in_page_i = pg.segment_cnt;
		let mut at_page_end = false;
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
			pg.segment_cnt = segment_in_page_i + 1;
			segment_in_page_i = (segment_in_page_i + 1) % 255;
			if segment_in_page_i == 0 {
				if segment_i + 1 < needed_segments {
					// We have to flush a page, but we know there are more to come...
					pg.pck_this_overflow_idx = Some((segment_i + 1) * 255);
					bail_out_on_fail!(Self::write_page(serial, pg, false, &mut sink_func));
				} else {
					// We have to write a page end, and it's the very last
					// we need to write
					bail_out_on_fail!(Self::write_page(serial, pg, is_end_stream, &mut sink_func));
					// Not actually required
					// (it is always None except if we set it to Some directly
					// before we call write_page)
					pg.pck_this_overflow_idx = None;
					// Required (it could have been Some(offs) before)
					pg.pck_last_overflow_idx = None;
				}
				at_page_end = true;
			}
		}
		if inf != PacketWriteEndInfo::NormalPacket && !at_page_end {
			// Write a page end
			bail_out_on_fail!(Self::write_page(serial, pg, is_end_stream, &mut sink_func));
		}
		// When ending the logical bitstream there is no point in keeping
		// around page data.
		if is_end_stream {
			self.page_vals.remove(&serial);
		}

		// All went fine.
		true
	}
	fn write_page(serial :u32, pg :&mut CurrentPageValues, last_page :bool,
				  mut sink_func :impl FnMut(&[u8]) -> bool) -> bool {
		// The page header with everything but the lacing values:
		let mut hdr = Vec::with_capacity(27);

		// Capture pattern.
		hdr.extend_from_slice(b"OggS");

		// Ogg format version, always zero.
		hdr.push(0);

		let mut flags = 0;
		if pg.pck_last_overflow_idx.is_some() { flags |= 0x01; }
		if pg.first_page { flags |= 0x02; }
		if last_page { flags |= 0x04; }
		hdr.push(flags);

		let pck_data = &pg.cur_pg_data;

		let mut last_finishing_pck_absgp = (-1i64) as u64;
		for (idx, (_, absgp)) in pck_data.iter().enumerate() {
			if !(idx + 1 == pck_data.len() &&
				pg.pck_this_overflow_idx.is_some()) {
				last_finishing_pck_absgp = *absgp;
			}
		}

		macro_rules! write_le {
			($sink:expr, $number:expr) => {
				$sink.extend_from_slice(&$number.to_le_bytes()[..])
			}
		}

		write_le!(hdr, last_finishing_pck_absgp);
		write_le!(hdr, serial);
		write_le!(hdr, pg.sequence_num);

		// checksum, calculated later on :)
		write_le!(hdr, 0_u32);

		write_le!(hdr, pg.segment_cnt);

		let pg_lacing = &pg.cur_pg_lacing[0 .. pg.segment_cnt as usize];


		let mut hash_calculated = vorbis_crc32_update(0, &hdr);
		hash_calculated = vorbis_crc32_update(hash_calculated, pg_lacing);

		for (idx, (pck, _)) in pck_data.iter().enumerate() {
			let mut start = 0;
			if idx == 0 { if let Some(idx) = pg.pck_last_overflow_idx {
				start = idx;
			}}
			let mut end = pck.len();
			if idx + 1 == pck_data.len() {
				if let Some(idx) = pg.pck_this_overflow_idx {
					end = idx;
				}
			}
			hash_calculated = vorbis_crc32_update(hash_calculated,
												  &pck[start .. end]);
		}

		// Go back to enter the checksum
		hdr[22..26].copy_from_slice(&hash_calculated.to_le_bytes()[..]);

		// Now all is done, write the stuff!
		// Bail out the function with a failure if some write error happens.
		// This way the calling code could retry writing the page.
		bail_out_on_fail!(sink_func(&hdr));
		bail_out_on_fail!(sink_func(pg_lacing));
		for (idx, (pck, _)) in pck_data.iter().enumerate() {
			let mut start = 0;
			if idx == 0 { if let Some(idx) = pg.pck_last_overflow_idx {
				start = idx;
			}}
			let mut end = pck.len();
			if idx + 1 == pck_data.len() {
				if let Some(idx) = pg.pck_this_overflow_idx {
					end = idx;
				}
			}
			bail_out_on_fail!(sink_func(&pck[start .. end]));
		}

		// Reset the page.
		pg.first_page = false;
		pg.sequence_num += 1;

		pg.segment_cnt = 0;
		// If we couldn't fully write the last
		// packet, we need to keep it for the next page,
		// otherwise just clear everything.
		if pg.pck_this_overflow_idx.is_some() {
			let d = pg.cur_pg_data.pop().unwrap();
			pg.cur_pg_data.clear();
			pg.cur_pg_data.push(d);
		} else {
			pg.cur_pg_data.clear();
		}

		pg.pck_last_overflow_idx = pg.pck_this_overflow_idx;
		pg.pck_this_overflow_idx = None;

		// All went fine.
		true
	}
}

impl <'writer, T :Write> PacketWriter<'writer, T> {
	/// Constructs a new `PacketWriter` with a given `Write`.
	pub fn new(wtr :T) -> Self {
		Self {
			wtr,
			base_pck_wtr : BasePacketWriter::new()
		}
	}
	/// Returns the wrapped writer, consuming the PacketWriter.
	pub fn into_inner(self) -> T {
		self.wtr
	}
	/// Access the interior writer.
	///
	/// This allows access of the writer contained inside.
	/// No guarantees are given onto the pattern of the writes.
	/// They may change in the future.
	pub fn inner(&self) -> &T {
		&self.wtr
	}
	/// Access the interior writer mutably.
	///
	/// This allows access of the writer contained inside.
	/// No guarantees are given onto the pattern of the writes.
	/// They may change in the future.
	pub fn inner_mut(&mut self) -> &mut T {
		&mut self.wtr
	}
	/// Write a packet.
	pub fn write_packet<P: Into<Cow<'writer, [u8]>>>(&mut self, pck_cont :P,
			serial :u32,
			inf :PacketWriteEndInfo,
			/* TODO find a better way to design the API around
				passing the absgp to the underlying implementation.
				e.g. the caller passes a closure on init which gets
				called when we encounter a new page... with the param
				the index inside the current page, or something.
			*/
			absgp :u64) -> Result<()> {
		let pck_cont = pck_cont.into();

		let mut io_err = None;
		self.base_pck_wtr.write_packet(pck_cont, serial, inf, absgp,
		   |ogg_data| match self.wtr.write_all(ogg_data) {
			   Ok(()) => true,
			   Err(err) => { io_err = Some(err); false }
		   });

		io_err.map_or(Ok(()), Err)
	}
}

impl<T :Seek + Write> PacketWriter<'_, T> {
	pub fn get_current_offs(&mut self) -> Result<u64> {
		self.wtr.seek(SeekFrom::Current(0))
	}
}

// TODO once 1.18 gets released, move this
// to the test module and make wtr pub(crate).
#[test]
fn test_recapture() {
	// Test that we can deal with recapture
	// at varying distances.
	// This is a regression test
	use std::io::{Cursor, Write};
	use super::PacketReader;

	let mut c = Cursor::new(Vec::new());
	let test_arr = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
	let test_arr_2 = [2, 4, 8, 16, 32, 64, 128, 127, 126, 125, 124];
	let test_arr_3 = [3, 5, 9, 17, 33, 65, 129, 129, 127, 126, 125];
	{
		let np = PacketWriteEndInfo::NormalPacket;
		let ep = PacketWriteEndInfo::EndPage;
		{
			let mut w = PacketWriter::new(&mut c);
			w.write_packet(&test_arr[..], 0xdeadb33f, ep, 0).unwrap();

			// Now, after the end of the page, put in some noise.
			w.wtr.write_all(&[0; 38]).unwrap();

			w.write_packet(&test_arr_2[..], 0xdeadb33f, np, 1).unwrap();
			w.write_packet(&test_arr_3[..], 0xdeadb33f, ep, 2).unwrap();
		}
	}
	//print_u8_slice(c.get_ref());
	assert_eq!(c.seek(SeekFrom::Start(0)).unwrap(), 0);
	{
		let mut r = PacketReader::new(c);
		let p1 = r.read_packet().unwrap().unwrap();
		assert_eq!(test_arr, *p1.data);
		let p2 = r.read_packet().unwrap().unwrap();
		assert_eq!(test_arr_2, *p2.data);
		let p3 = r.read_packet().unwrap().unwrap();
		assert_eq!(test_arr_3, *p3.data);
	}
}

/// Asynchronous Ogg encoding.
#[cfg(feature = "async")]
pub mod async_api {
	use std::io;
	use std::pin::Pin;
	use std::task::{Context, Poll};

	use super::*;
	use futures_sink::Sink;
	use futures_io::AsyncWrite as FuturesAsyncWrite;
	use tokio::io::AsyncWrite as TokioAsyncWrite;
	use bytes::BytesMut;
	use pin_project::pin_project;
	use tokio_util::codec::{Encoder, FramedWrite};
	use tokio_util::compat::{Compat, FuturesAsyncWriteCompatExt};

	struct PacketEncoder<'writer> {
		base_pck_wtr :BasePacketWriter<'writer>,
	}

	impl<'writer, 'packet :'writer> Encoder<Packet<'packet>> for PacketEncoder<'writer> {
		type Error = io::Error;

		fn encode(&mut self, item :Packet<'packet>, dst :&mut BytesMut) -> Result<()> {
			// An encoder only cares about encapsulating data in the proper format,
			// which in this case is Ogg packets in Ogg pages, to a memory buffer.
			// Memory operations are assumed to be infallible, so the base packet
			// writer sink function can be a simple passthrough. Flushing and
			// actual I/O is taken care of by FramedWrite. Observe that writing
			// a packet is not guaranteed to generate any page, and thus any data
			// to encode, unless it ends a bitstream or forces the end the page it
			// belongs to.
			self.base_pck_wtr.write_packet(item.data, item.serial, item.inf, item.absgp,
				|ogg_data| { dst.extend_from_slice(ogg_data); true }
			);

			Ok(())
		}
	}

	/// Asynchronous writer for packets into an Ogg stream.
	///
	/// Please read the documentation of the [`Packet::inf`] field for more
	/// information about the not-so-obvious semantics of flushing this sink.
	#[pin_project]
	pub struct PacketWriter<'writer, W :TokioAsyncWrite> {
		#[pin]
		sink :FramedWrite<W, PacketEncoder<'writer>>,
	}

	/// A Ogg packet that may be fed to a [`PacketWriter`].
	pub struct Packet<'packet> {
		/// The data the packet contains.
		pub data :Cow<'packet, [u8]>,
		/// The serial of the stream this packet belongs to.
		pub serial :u32,
		/// Specifies whether to end something with the write of the packet.
		///
		/// Note that flushing a [`PacketWriter`] alone does not guarantee that
		/// every Ogg packet so far has made it to the destination: normally,
		/// packets are stuffed into pages as possible, and then those pages are
		/// written. Flushing will only write pending pages, thus packets that
		/// belong to yet incomplete pages will not immediately generate anything
		/// to flush.
		///
		/// A packet has to forcibly end the page or stream it belongs to in order
		/// to ensure that it is written on a flush. That can be done by setting
		/// this value accordingly.
		pub inf :PacketWriteEndInfo,
		/// The granule position of the packet.
		pub absgp :u64,
	}

	impl<W :TokioAsyncWrite> PacketWriter<'_, W> {
		/// Wraps the specified Tokio runtime `AsyncWrite` into an Ogg packet
		/// writer.
		///
		/// This is the recommended constructor when using the Tokio runtime
		/// types.
		pub fn new(inner :W) -> Self {
			Self {
				sink : FramedWrite::new(inner, PacketEncoder {
					base_pck_wtr : BasePacketWriter::new()
				}),
			}
		}
	}

	impl<W :FuturesAsyncWrite> PacketWriter<'_, Compat<W>> {
		/// Wraps the specified futures_io `AsyncWrite` into an Ogg packet
		/// writer.
		///
		/// This crate uses Tokio internally, so a wrapper that may have
		/// some performance cost will be used. Therefore, this constructor
		/// is to be used only when dealing with `AsyncWrite` implementations
		/// from other runtimes, and implementing a Tokio `AsyncWrite`
		/// compatibility layer oneself is not desired.
		pub fn new_compat(inner :W) -> Self {
			Self::new(inner.compat_write())
		}
	}

	impl<'writer, 'packet :'writer, W :TokioAsyncWrite> Sink<Packet<'packet>> for PacketWriter<'writer, W> {
		type Error = io::Error;

		fn poll_ready(self :Pin<&mut Self>, cx :&mut Context<'_>) -> Poll<Result<()>> {
			self.project().sink.poll_ready(cx)
		}

		fn start_send(self :Pin<&mut Self>, item :Packet<'packet>) -> Result<()> {
			self.project().sink.start_send(item)
		}

		fn poll_flush(self :Pin<&mut Self>, cx :&mut Context<'_>) -> Poll<Result<()>> {
			self.project().sink.poll_flush(cx)
		}

		fn poll_close(self :Pin<&mut Self>, cx :&mut Context<'_>) -> Poll<Result<()>> {
			self.project().sink.poll_close(cx)
		}
	}
}
