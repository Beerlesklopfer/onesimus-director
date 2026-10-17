//! Decompression of restore data: lib/compression.cc DecompressData().
//!
//! - GZIP (legacy STREAM_*_GZIP_DATA without header, and COMPRESS_GZIP in a
//!   comp_stream_header): zlib `uncompress()`, here flate2.
//! - COMPRESS_FZFZ / COMPRESS_FZ4L / COMPRESS_FZ4H: the fastlzlib block
//!   stream (core/src/fastlz/src/fastlzlib.c) with the FastLZ resp. LZ4
//!   block decoders, ported to Rust.
//! - COMPRESS_LZO1X: lzo1x_decompress_safe() of liblzo2, loaded at run time
//!   (the original links liblzo2; without the library the stream is
//!   reported like a build without HAVE_LZO).

use std::io::Read;

use crate::streams::*;

// include/ch.h
pub const COMPRESS_GZIP: u32 = 0x475a_4950;
pub const COMPRESS_LZO1X: u32 = 0x4c5a_4f58;
pub const COMPRESS_FZFZ: u32 = 0x465A_465A;
pub const COMPRESS_FZ4L: u32 = 0x465A_344C;
pub const COMPRESS_FZ4H: u32 = 0x465A_3448;
const COMP_HEAD_VERSION: u16 = 1;
const COMP_HEADER_SIZE: usize = 12;

/// DecompressData() with want_data_stream=false. Err: the M_ERROR text.
pub fn decompress_data(last_fname: &str, stream: i32, data: &[u8]) -> Result<Vec<u8>, String> {
    match stream {
        STREAM_COMPRESSED_DATA
        | STREAM_SPARSE_COMPRESSED_DATA
        | STREAM_WIN32_COMPRESSED_DATA
        | STREAM_ENCRYPTED_FILE_COMPRESSED_DATA
        | STREAM_ENCRYPTED_WIN32_COMPRESSED_DATA => {
            if data.len() < COMP_HEADER_SIZE {
                return Err(format!("Compressed header size error. comp_len=0, message_length={}\n", data.len()));
            }
            // unser order: magic, comp_len, level, version (network order)
            let magic = u32::from_be_bytes(data[0..4].try_into().unwrap());
            let comp_len = u32::from_be_bytes(data[4..8].try_into().unwrap());
            let _level = u16::from_be_bytes(data[8..10].try_into().unwrap());
            let version = u16::from_be_bytes(data[10..12].try_into().unwrap());
            if version != COMP_HEAD_VERSION {
                return Err(format!("Compressed header version error. version=0x{version:x}\n"));
            }
            if comp_len as usize + COMP_HEADER_SIZE != data.len() {
                return Err(format!("Compressed header size error. comp_len={comp_len}, message_length={}\n", data.len()));
            }
            let payload = &data[COMP_HEADER_SIZE..];
            match magic {
                COMPRESS_GZIP => zlib(last_fname, payload),
                COMPRESS_LZO1X => lzo(last_fname, payload),
                COMPRESS_FZFZ | COMPRESS_FZ4L | COMPRESS_FZ4H => fastlzlib(payload, magic != COMPRESS_FZFZ)
                    .map_err(|e| format!("Uncompression error on file {last_fname}. ERR={e}\n")),
                _ => Err(format!("Compression algorithm 0x{magic:x} found, but not supported!\n")),
            }
        }
        _ => zlib(last_fname, data),
    }
}

fn zlib(last_fname: &str, data: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = Vec::with_capacity(data.len() * 4);
    flate2::read::ZlibDecoder::new(data)
        .read_to_end(&mut out)
        .map_err(|e| format!("Uncompression error on file {last_fname}. ERR={}\n", zlib_strerror(&e)))?;
    Ok(out)
}

/// zlib_strerror() for the errors flate2 can report.
fn zlib_strerror(e: &std::io::Error) -> &'static str {
    match e.kind() {
        std::io::ErrorKind::UnexpectedEof => "Buffer error",
        _ => "Data error",
    }
}

// ---------------------------------------------------------------------------
// fastlzlib block stream
// ---------------------------------------------------------------------------

const FLZ_HEADER_SIZE: usize = 16;
const BLOCK_TYPE_RAW: u8 = 0x10;
const BLOCK_TYPE_COMPRESSED: u8 = 0xc0;

/// fastlzlibDecompress() over a whole buffer: blocks of a 16 byte header
/// ("FastLZ\0", type|block size power, compressed and original size LE)
/// and the payload; a header with both sizes 0 ends the stream.
pub fn fastlzlib(mut input: &[u8], lz4: bool) -> Result<Vec<u8>, &'static str> {
    let mut out = Vec::new();
    while !input.is_empty() {
        if input.len() < FLZ_HEADER_SIZE {
            return Err("Buffer error");
        }
        if &input[..7] != b"FastLZ\0" {
            return Err("Data error");
        }
        let packed = input[7];
        let block_type = packed & 0xf0;
        let str_size = u32::from_le_bytes(input[8..12].try_into().unwrap()) as usize;
        let dec_size = u32::from_le_bytes(input[12..16].try_into().unwrap()) as usize;
        input = &input[FLZ_HEADER_SIZE..];
        if str_size == 0 && dec_size == 0 {
            break;
        }
        if input.len() < str_size {
            return Err("Buffer error");
        }
        let block = &input[..str_size];
        input = &input[str_size..];
        match block_type {
            BLOCK_TYPE_RAW => out.extend_from_slice(block),
            BLOCK_TYPE_COMPRESSED => {
                let start = out.len();
                let ok = if lz4 { lz4_block(block, &mut out, dec_size) } else { fastlz_block(block, &mut out, dec_size) };
                if !ok || out.len() - start != dec_size {
                    return Err("Data error");
                }
            }
            _ => return Err("Incompatible version"),
        }
    }
    Ok(out)
}

/// LZ4_decompress_safe() of one block, appended to `out`.
fn lz4_block(src: &[u8], out: &mut Vec<u8>, maxout: usize) -> bool {
    let base = out.len();
    let limit = base + maxout;
    let mut i = 0;
    loop {
        let Some(&token) = src.get(i) else { return false };
        i += 1;
        let mut lit = usize::from(token >> 4);
        if lit == 15 {
            loop {
                let Some(&b) = src.get(i) else { return false };
                i += 1;
                lit += usize::from(b);
                if b != 255 {
                    break;
                }
            }
        }
        if i + lit > src.len() || out.len() + lit > limit {
            return false;
        }
        out.extend_from_slice(&src[i..i + lit]);
        i += lit;
        if i == src.len() {
            return true;
        }
        if i + 2 > src.len() {
            return false;
        }
        let offset = usize::from(u16::from_le_bytes([src[i], src[i + 1]]));
        i += 2;
        if offset == 0 || offset > out.len() - base {
            return false;
        }
        let mut mlen = usize::from(token & 15);
        if mlen == 15 {
            loop {
                let Some(&b) = src.get(i) else { return false };
                i += 1;
                mlen += usize::from(b);
                if b != 255 {
                    break;
                }
            }
        }
        mlen += 4;
        if out.len() + mlen > limit {
            return false;
        }
        let from = out.len() - offset;
        for k in 0..mlen {
            let b = out[from + k];
            out.push(b);
        }
    }
}

/// fastlz_decompress() (levels 1 and 2) of one block, appended to `out`.
fn fastlz_block(src: &[u8], out: &mut Vec<u8>, maxout: usize) -> bool {
    const MAX_DISTANCE: usize = 8191;
    let Some(&first) = src.first() else { return false };
    let level = (first >> 5) + 1;
    if level != 1 && level != 2 {
        return false;
    }
    let base = out.len();
    let limit = base + maxout;
    let mut ip = 1usize;
    let mut ctrl = usize::from(first & 31);
    let at = |i: usize| src.get(i).copied();
    loop {
        if ctrl >= 32 {
            let mut len = (ctrl >> 5) - 1;
            let ofs = (ctrl & 31) << 8;
            // ref = op - ofs (then minus the next code byte, then 1)
            let mut back = ofs;
            if len == 7 - 1 {
                if level == 1 {
                    let Some(b) = at(ip) else { return false };
                    ip += 1;
                    len += usize::from(b);
                } else {
                    loop {
                        let Some(code) = at(ip) else { return false };
                        ip += 1;
                        len += usize::from(code);
                        if code != 255 {
                            break;
                        }
                    }
                }
            }
            let Some(code) = at(ip) else { return false };
            ip += 1;
            back += usize::from(code);
            if level == 2 && code == 255 && ofs == (31 << 8) {
                let (Some(h), Some(l)) = (at(ip), at(ip + 1)) else { return false };
                ip += 2;
                back = (usize::from(h) << 8) + usize::from(l) + MAX_DISTANCE;
            }
            // distance from op to the first byte copied = back + 1
            let produced = out.len() - base;
            if out.len() + len + 3 > limit || back + 1 > produced {
                return false;
            }
            let from = out.len() - back - 1;
            for k in 0..len + 3 {
                let b = out[from + k];
                out.push(b);
            }
            match at(ip) {
                Some(c) => {
                    ctrl = usize::from(c);
                    ip += 1;
                }
                None => return true,
            }
        } else {
            let n = ctrl + 1;
            if out.len() + n > limit || ip + n > src.len() {
                return false;
            }
            out.extend_from_slice(&src[ip..ip + n]);
            ip += n;
            match at(ip) {
                Some(c) => {
                    ctrl = usize::from(c);
                    ip += 1;
                }
                None => return true,
            }
        }
    }
}

// ---------------------------------------------------------------------------
// LZO via liblzo2 (dlopen)
// ---------------------------------------------------------------------------

type LzoDecompress = unsafe extern "C" fn(*const u8, usize, *mut u8, *mut usize, *mut libc::c_void) -> libc::c_int;

fn lzo_fn() -> Option<LzoDecompress> {
    static F: std::sync::OnceLock<Option<usize>> = std::sync::OnceLock::new();
    let addr = *F.get_or_init(|| unsafe {
        for lib in [c"liblzo2.so.2", c"liblzo2.so"] {
            let h = libc::dlopen(lib.as_ptr(), libc::RTLD_NOW);
            if !h.is_null() {
                let s = libc::dlsym(h, c"lzo1x_decompress_safe".as_ptr());
                if !s.is_null() {
                    return Some(s as usize);
                }
            }
        }
        None
    });
    addr.map(|a| unsafe { std::mem::transmute::<usize, LzoDecompress>(a) })
}

fn lzo(last_fname: &str, data: &[u8]) -> Result<Vec<u8>, String> {
    let Some(f) = lzo_fn() else {
        return Err(format!("Compression algorithm 0x{COMPRESS_LZO1X:x} found, but not supported!\n"));
    };
    const LZO_E_OUTPUT_OVERRUN: libc::c_int = -5;
    let mut size = (data.len() * 4).max(65536);
    loop {
        let mut out = vec![0u8; size];
        let mut out_len = size;
        let status = unsafe { f(data.as_ptr(), data.len(), out.as_mut_ptr(), &mut out_len, std::ptr::null_mut()) };
        if status == LZO_E_OUTPUT_OVERRUN && size < (1 << 30) {
            size += size >> 1;
            continue;
        }
        if status != 0 {
            return Err(format!("LZO uncompression error on file {last_fname}. ERR={status}\n"));
        }
        out.truncate(out_len);
        return Ok(out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn header(magic: u32, payload: &[u8]) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(&magic.to_be_bytes());
        v.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        v.extend_from_slice(&6u16.to_be_bytes());
        v.extend_from_slice(&1u16.to_be_bytes());
        v.extend_from_slice(payload);
        v
    }

    #[test]
    fn gzip_with_and_without_header() {
        let text = b"hello hello hello hello bareos".repeat(50);
        let mut enc = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        enc.write_all(&text).unwrap();
        let z = enc.finish().unwrap();
        assert_eq!(decompress_data("f", STREAM_GZIP_DATA, &z).unwrap(), text);
        assert_eq!(decompress_data("f", STREAM_COMPRESSED_DATA, &header(COMPRESS_GZIP, &z)).unwrap(), text);
        let mut bad = header(COMPRESS_GZIP, &z);
        bad[11] = 2;
        assert!(decompress_data("f", STREAM_COMPRESSED_DATA, &bad).unwrap_err().contains("version error"));
        assert!(decompress_data("f", STREAM_COMPRESSED_DATA, &header(0x1234, b"x")).unwrap_err().contains("not supported"));
    }

    #[test]
    fn lz4_and_fastlz_blocks() {
        // LZ4: literal "abc", match offset 3 length 6, final literals "xyz12"
        // token 0x32 (3 literals, match 2+4=6)
        let mut lz = vec![0x32, b'a', b'b', b'c', 3, 0];
        lz.extend_from_slice(&[0x50, b'x', b'y', b'z', b'1', b'2']);
        let mut out = Vec::new();
        assert!(lz4_block(&lz, &mut out, 14));
        assert_eq!(out, b"abcabcabcxyz12");

        // FastLZ level 1: literal run of 3 ("abc", ctrl=2), then a match
        // len code 1 (+2 = 3 bytes) distance 3 (ofs 0, code 2) -> "abc"
        let fl = vec![2, b'a', b'b', b'c', (1 << 5), 2];
        let mut out = Vec::new();
        assert!(fastlz_block(&fl, &mut out, 6));
        assert_eq!(out, b"abcabc");

        // Framed stream: one compressed (LZ4) and one raw block, EOF marker
        let mut s = Vec::new();
        let hdr = |s: &mut Vec<u8>, t: u8, c: u32, o: u32| {
            s.extend_from_slice(b"FastLZ\0");
            s.push(t | 8);
            s.extend_from_slice(&c.to_le_bytes());
            s.extend_from_slice(&o.to_le_bytes());
        };
        hdr(&mut s, BLOCK_TYPE_COMPRESSED, lz.len() as u32, 14);
        s.extend_from_slice(&lz);
        hdr(&mut s, BLOCK_TYPE_RAW, 2, 2);
        s.extend_from_slice(b"!!");
        hdr(&mut s, BLOCK_TYPE_COMPRESSED, 0, 0);
        assert_eq!(decompress_data("f", STREAM_COMPRESSED_DATA, &header(COMPRESS_FZ4L, &s)).unwrap(), b"abcabcabcxyz12!!");
    }
}
