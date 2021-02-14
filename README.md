## Ogg

An Ogg decoder and encoder. Implements the [xiph.org Ogg spec](https://www.xiph.org/vorbis/doc/framing.html) in pure Rust.

[Documentation](https://docs.rs/ogg/0.8.0).

If the `async` feature is disabled, Version 1.27.0 of Rust is the minimum supported one.

Note: `.ogg` files are vorbis encoded audio files embedded into an Ogg transport stream.
There is no extra support for vorbis codec decoding or encoding in this crate,
so you need additional functionality in order to decode them. For example, you can use the
[lewton](https://github.com/RustAudio/lewton) crate.

Also note that the encoder part of the Crate isn't as well tested as the decoder part,
in fact it was only written in order to write compact testing code for the decoder.

## License

Licensed under the three clause BSD license. For details, see the [LICENSE](LICENSE) file.
