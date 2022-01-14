// Ogg decoder and encoder written in Rust
//
// Copyright (c) 2016 est31 <MTest31@outlook.com>
// and contributors. All rights reserved.
// Redistribution or use only under the terms
// specified in the LICENSE file attached to this
// source distribution.

#![cfg(feature = "async")]

use std::io;
use ogg::{PacketWriter, PacketWriteEndInfo};
use ogg::reading::async_api::PacketReader;
use std::io::{Cursor, Seek, SeekFrom};
use std::pin::Pin;
use std::task::{Poll, Context};
use pin_project::pin_project;
use tokio::io::{AsyncRead, AsyncSeek, ReadBuf};
use futures_util::TryStreamExt;

#[pin_project]
struct RandomWouldBlock<T>(#[pin] T);
impl<T :AsyncRead> AsyncRead for RandomWouldBlock<T> {
	fn poll_read(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>)
			-> Poll<io::Result<()>> {
		if rand::random() {
			cx.waker().wake_by_ref();
			return Poll::Pending;
		}

		self.project().0.poll_read(cx, buf)
	}
}

impl<T :AsyncSeek> AsyncSeek for RandomWouldBlock<T> {
	fn start_seek(self: Pin<&mut Self>, position: SeekFrom) -> io::Result<()> {
		self.project().0.start_seek(position)
	}

	fn poll_complete(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<u64>> {
		if rand::random() {
			cx.waker().wake_by_ref();
			return Poll::Pending;
		}

		self.project().0.poll_complete(cx)
	}
}

async fn test_ogg_random_would_block_run() {
	let mut c = Cursor::new(Vec::new());
	let test_arr = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
	let test_arr_2 = [2, 4, 8, 16, 32, 64, 128, 127, 126, 125, 124];
	let test_arr_3 = [3, 5, 9, 17, 33, 65, 129, 129, 127, 126, 125];
	{
		let mut w = PacketWriter::new(&mut c);
		let np = PacketWriteEndInfo::NormalPacket;
		w.write_packet(&test_arr[..], 0xdeadb33f, np, 0).unwrap();
		w.write_packet(&test_arr_2[..], 0xdeadb33f, np, 1).unwrap();
		w.write_packet(&test_arr_3[..], 0xdeadb33f,
			PacketWriteEndInfo::EndPage, 2).unwrap();
	}
	//print_u8_slice(c.get_ref());
	assert_eq!(c.seek(SeekFrom::Start(0)).unwrap(), 0);
	{
		let mut rwd = RandomWouldBlock(&mut c);
		let mut r = PacketReader::new(&mut rwd);
		let p1 = r.try_next().await.unwrap().unwrap();
		assert_eq!(test_arr, *p1.data);
		let p2 = r.try_next().await.unwrap().unwrap();
		assert_eq!(test_arr_2, *p2.data);
		let p3 = r.try_next().await.unwrap().unwrap();
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
		w.write_packet(&test_arr[..], 0xdeadb33f, np, 0).unwrap();
		w.write_packet(&test_arr_2[..], 0xdeadb33f, np, 1).unwrap();
		w.write_packet(&test_arr_3[..], 0xdeadb33f,
			PacketWriteEndInfo::EndPage, 2).unwrap();
	}
	//print_u8_slice(c.get_ref());
	assert_eq!(c.seek(SeekFrom::Start(0)).unwrap(), 0);
	{
		let mut rwd = RandomWouldBlock(&mut c);
		let mut r = PacketReader::new(&mut rwd);
		let p1 = r.try_next().await.unwrap().unwrap();
		assert_eq!(test_arr, p1.data.as_slice());
		let p2 = r.try_next().await.unwrap().unwrap();
		assert_eq!(test_arr_2, p2.data.as_slice());
		let p3 = r.try_next().await.unwrap().unwrap();
		assert_eq!(test_arr_3, p3.data.as_slice());
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
		w.write_packet(&test_arr_2[..], 0xdeadb33f, np, 1).unwrap();
		w.write_packet(&test_arr_3[..], 0xdeadb33f,
			PacketWriteEndInfo::EndPage, 2).unwrap();
	}
	//print_u8_slice(c.get_ref());
	assert_eq!(c.seek(SeekFrom::Start(0)).unwrap(), 0);
	{
		let mut rwd = RandomWouldBlock(&mut c);
		let mut r = PacketReader::new(&mut rwd);
		let p2 = r.try_next().await.unwrap().unwrap();
		assert_eq!(test_arr_2, p2.data.as_slice());
		let p3 = r.try_next().await.unwrap().unwrap();
		assert_eq!(test_arr_3, p3.data.as_slice());
	}
}

#[tokio::test]
async fn test_ogg_random_would_block() {
	for i in 0 .. 100 {
		println!("Run {}", i);
		test_ogg_random_would_block_run().await;
	}
}
