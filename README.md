## Ogg

An Ogg decoder and encoder. Implements the [xiph.org Ogg spec](https://www.xiph.org/vorbis/doc/framing.html) in pure Rust.

Note: `.ogg` files are vorbis encoded audio files embedded into an Ogg transport stream.
There is no extra support for vorbis codec decoding or encoding in this crate,
so you need additional functionality in order to decode them.

## License

Licensed under Apache 2 or MIT (at your option). For details, see the [LICENSE](LICENSE) file.
