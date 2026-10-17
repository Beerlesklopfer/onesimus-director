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

// ---------------------------------------------------------------------------
// Compression (backup side): lib/compression.cc CompressData() and the
// comp_stream_header of filed/compression.cc SetupCompressionContext().
// ---------------------------------------------------------------------------

/// The compressed payload (without comp_stream_header) of one block.
pub fn compress_data(algo: u32, level: i32, data: &[u8]) -> Result<Vec<u8>, String> {
    match algo {
        COMPRESS_GZIP => gzip_compress(level, data),
        COMPRESS_LZO1X => lzo_compress(data),
        COMPRESS_FZFZ => Ok(fastlzlib_compress(data, false)),
        COMPRESS_FZ4L | COMPRESS_FZ4H => Ok(fastlzlib_compress(data, true)),
        _ => Err(format!("Unknown compression algorithm: {algo}")),
    }
}

/// The record data: comp_stream_header (magic, compressed length, level,
/// version 1; network order) and the compressed payload. The level is 0
/// for LZO (SetupCompressionContext only sets it for GZIP and FastLZ/LZ4).
pub fn with_header(algo: u32, level: i32, payload: &[u8]) -> Vec<u8> {
    let lvl: u16 = match algo {
        COMPRESS_GZIP | COMPRESS_FZFZ | COMPRESS_FZ4L | COMPRESS_FZ4H => level as u16,
        _ => 0,
    };
    let mut v = Vec::with_capacity(payload.len() + COMP_HEADER_SIZE);
    v.extend_from_slice(&algo.to_be_bytes());
    v.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    v.extend_from_slice(&lvl.to_be_bytes());
    v.extend_from_slice(&COMP_HEAD_VERSION.to_be_bytes());
    v.extend_from_slice(payload);
    v
}

/// The compression algorithm can be used (ClearCompressionFlagInFileset).
pub fn compressor_available(algo: u32) -> bool {
    match algo {
        COMPRESS_GZIP | COMPRESS_FZFZ | COMPRESS_FZ4L | COMPRESS_FZ4H => true,
        COMPRESS_LZO1X => lzo_compress_fn().is_some(),
        _ => false,
    }
}

/// CompressorName()
pub fn compressor_name(algo: u32) -> &'static str {
    match algo {
        COMPRESS_GZIP => "GZIP",
        COMPRESS_LZO1X => "LZO",
        COMPRESS_FZFZ => "FASTLZ",
        COMPRESS_FZ4L => "LZ4",
        COMPRESS_FZ4H => "LZ4HC",
        _ => "Unknown",
    }
}

const FLZ_DEFAULT_BLOCK_SIZE: usize = 262144;
const FLZ_MIN_BLOCK_SIZE: usize = 64;

/// fastlzlibCompress(Z_FINISH) over a whole buffer (fastlz_compress_hdr per
/// block of 256 KiB, then the EOF marker).
pub fn fastlzlib_compress(input: &[u8], lz4: bool) -> Vec<u8> {
    let mut out = Vec::with_capacity(input.len() + input.len() / 10 + 64);
    let header = |out: &mut Vec<u8>, t: u8, c: usize, o: usize| {
        out.extend_from_slice(b"FastLZ\0");
        out.push(t + 8); // 2^(10+8) = 256 KiB block size
        out.extend_from_slice(&(c as u32).to_le_bytes());
        out.extend_from_slice(&(o as u32).to_le_bytes());
    };
    for block in input.chunks(FLZ_DEFAULT_BLOCK_SIZE) {
        let compressed = if block.len() > FLZ_MIN_BLOCK_SIZE {
            let c = if lz4 { lz4_compress_block(block) } else { fastlz1_compress_block(block) };
            (c.len() < block.len()).then_some(c)
        } else {
            None
        };
        match compressed {
            Some(c) => {
                header(&mut out, BLOCK_TYPE_COMPRESSED, c.len(), block.len());
                out.extend_from_slice(&c);
            }
            None => {
                header(&mut out, BLOCK_TYPE_RAW, block.len(), block.len());
                out.extend_from_slice(block);
            }
        }
    }
    header(&mut out, BLOCK_TYPE_COMPRESSED, 0, 0);
    out
}

fn hash_bits(v: u32, bits: u32) -> usize {
    (v.wrapping_mul(2654435761) >> (32 - bits)) as usize
}

fn read_u32(b: &[u8], i: usize) -> u32 {
    u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]])
}

fn push_len(out: &mut Vec<u8>, mut r: usize) {
    while r >= 255 {
        out.push(255);
        r -= 255;
    }
    out.push(r as u8);
}

/// A greedy LZ4 block compressor (output for LZ4_decompress_safe: the last
/// 5 bytes are literals, no match starts in the last 12 bytes).
pub fn lz4_compress_block(src: &[u8]) -> Vec<u8> {
    const MIN_MATCH: usize = 4;
    const LAST_LITERALS: usize = 5;
    const MF_LIMIT: usize = 12;
    let n = src.len();
    let mut out = Vec::with_capacity(n + n / 255 + 16);
    let mut table = vec![usize::MAX; 1 << 16];
    let mut anchor = 0;
    let mut i = 0;
    let emit = |out: &mut Vec<u8>, lit: &[u8], mtch: Option<(usize, usize)>| {
        let ll = lit.len();
        let ml = mtch.map(|(m, _)| m - MIN_MATCH).unwrap_or(0);
        out.push(((ll.min(15) as u8) << 4) | (ml.min(15) as u8));
        if ll >= 15 {
            push_len(out, ll - 15);
        }
        out.extend_from_slice(lit);
        if let Some((_, off)) = mtch {
            out.extend_from_slice(&(off as u16).to_le_bytes());
            if ml >= 15 {
                push_len(out, ml - 15);
            }
        }
    };
    if n > MF_LIMIT {
        let match_limit = n - LAST_LITERALS;
        while i + MF_LIMIT < n {
            let h = hash_bits(read_u32(src, i), 16);
            let cand = table[h];
            table[h] = i;
            if cand != usize::MAX && i - cand <= 65535 && read_u32(src, cand) == read_u32(src, i) {
                let mut m = MIN_MATCH;
                while i + m < match_limit && src[cand + m] == src[i + m] {
                    m += 1;
                }
                emit(&mut out, &src[anchor..i], Some((m, i - cand)));
                i += m;
                anchor = i;
            } else {
                i += 1;
            }
        }
    }
    emit(&mut out, &src[anchor..], None);
    out
}

/// fastlz1_compress() of core/src/fastlz/src/fastlz.c, byte for byte.
pub fn fastlz1_compress_block(src: &[u8]) -> Vec<u8> {
    const MAX_COPY: usize = 32;
    const MAX_LEN: usize = 264;
    const MAX_DISTANCE: usize = 8192;
    const HASH_MASK: u32 = (1 << 13) - 1;
    let length = src.len();
    let mut out = Vec::with_capacity(length + length / 16 + 16);
    if length < 4 {
        if length > 0 {
            out.push((length - 1) as u8);
            out.extend_from_slice(src);
        }
        return out;
    }
    let ip_bound = length - 2;
    let ip_limit = length as isize - 12 - 1;
    let r16 = |p: usize| u32::from(u16::from_le_bytes([src[p], src[p + 1]]));
    let hash = |p: usize| {
        let mut v = r16(p);
        v ^= r16(p + 1) ^ (v >> (16 - 13));
        (v & HASH_MASK) as usize
    };
    let mut htab = vec![0usize; 1 << 13];
    let mut copy: usize = 2;
    out.push((MAX_COPY - 1) as u8);
    out.push(src[0]);
    out.push(src[1]);
    let mut ip = 2usize;
    while (ip as isize) < ip_limit {
        let anchor = ip;
        let hval = hash(ip);
        let mut rf = htab[hval];
        let mut distance = anchor - rf;
        htab[hval] = anchor;
        if distance == 0
            || distance >= MAX_DISTANCE
            || src[rf] != src[ip]
            || src[rf + 1] != src[ip + 1]
            || src[rf + 2] != src[ip + 2]
        {
            // literal
            out.push(src[anchor]);
            ip = anchor + 1;
            copy += 1;
            if copy == MAX_COPY {
                copy = 0;
                out.push((MAX_COPY - 1) as u8);
            }
            continue;
        }
        rf += 3;
        ip = anchor + 3;
        distance -= 1;
        if distance == 0 {
            // a run
            let x = src[ip - 1];
            while ip < ip_bound {
                let eq = src[rf] == x;
                rf += 1;
                if !eq {
                    break;
                }
                ip += 1;
            }
        } else {
            let mut all = true;
            for _ in 0..8 {
                let eq = src[rf] == src[ip];
                rf += 1;
                ip += 1;
                if !eq {
                    all = false;
                    break;
                }
            }
            if all {
                while ip < ip_bound {
                    let eq = src[rf] == src[ip];
                    rf += 1;
                    ip += 1;
                    if !eq {
                        break;
                    }
                }
            }
        }
        if copy > 0 {
            let idx = out.len() - copy - 1;
            out[idx] = (copy - 1) as u8;
        } else {
            out.pop();
        }
        copy = 0;
        ip -= 3;
        let mut len = ip - anchor;
        while len > MAX_LEN - 2 {
            out.push(((7 << 5) + (distance >> 8)) as u8);
            out.push((MAX_LEN - 2 - 7 - 2) as u8);
            out.push((distance & 255) as u8);
            len -= MAX_LEN - 2;
        }
        if len < 7 {
            out.push(((len << 5) + (distance >> 8)) as u8);
            out.push((distance & 255) as u8);
        } else {
            out.push(((7 << 5) + (distance >> 8)) as u8);
            out.push((len - 7) as u8);
            out.push((distance & 255) as u8);
        }
        let h = hash(ip);
        htab[h] = ip;
        ip += 1;
        let h = hash(ip);
        htab[h] = ip;
        ip += 1;
        out.push((MAX_COPY - 1) as u8);
    }
    while ip <= ip_bound + 1 {
        out.push(src[ip]);
        ip += 1;
        copy += 1;
        if copy == MAX_COPY {
            copy = 0;
            out.push((MAX_COPY - 1) as u8);
        }
    }
    if copy > 0 {
        let idx = out.len() - copy - 1;
        out[idx] = (copy - 1) as u8;
    } else {
        out.pop();
    }
    out
}

type ZCompress2 = unsafe extern "C" fn(*mut u8, *mut libc::c_ulong, *const u8, libc::c_ulong, libc::c_int) -> libc::c_int;

/// compress2() of the system zlib (what the original's deflate() on a
/// fresh stream produces); None without libz.
fn zlib_compress2() -> Option<ZCompress2> {
    static F: std::sync::OnceLock<Option<usize>> = std::sync::OnceLock::new();
    let addr = *F.get_or_init(|| unsafe {
        for lib in [c"libz.so.1", c"libz.so"] {
            let h = libc::dlopen(lib.as_ptr(), libc::RTLD_NOW);
            if !h.is_null() {
                let s = libc::dlsym(h, c"compress2".as_ptr());
                if !s.is_null() {
                    return Some(s as usize);
                }
            }
        }
        None
    });
    addr.map(|a| unsafe { std::mem::transmute::<usize, ZCompress2>(a) })
}

/// GZIP block: zlib deflate at `level` (the system zlib when present).
fn gzip_compress(level: i32, data: &[u8]) -> Result<Vec<u8>, String> {
    if let Some(f) = zlib_compress2() {
        let mut out = vec![0u8; data.len() + data.len() / 1000 + 64 + 18];
        let mut len = out.len() as libc::c_ulong;
        let r = unsafe { f(out.as_mut_ptr(), &mut len, data.as_ptr(), data.len() as libc::c_ulong, level) };
        if r != 0 {
            return Err(format!("Compression deflate error: {r}\n"));
        }
        out.truncate(len as usize);
        return Ok(out);
    }
    use std::io::Write as _;
    let lvl = level.clamp(0, 9) as u32;
    let mut enc = flate2::write::ZlibEncoder::new(Vec::with_capacity(data.len() / 2 + 64), flate2::Compression::new(lvl));
    enc.write_all(data).map_err(|e| format!("Compression deflate error: {e}\n"))?;
    enc.finish().map_err(|e| format!("Compression deflate error: {e}\n"))
}

type LzoCompress = unsafe extern "C" fn(*const u8, usize, *mut u8, *mut usize, *mut libc::c_void) -> libc::c_int;

fn lzo_compress_fn() -> Option<LzoCompress> {
    static F: std::sync::OnceLock<Option<usize>> = std::sync::OnceLock::new();
    let addr = *F.get_or_init(|| unsafe {
        for lib in [c"liblzo2.so.2", c"liblzo2.so"] {
            let h = libc::dlopen(lib.as_ptr(), libc::RTLD_NOW);
            if !h.is_null() {
                let s = libc::dlsym(h, c"lzo1x_1_compress".as_ptr());
                if !s.is_null() {
                    return Some(s as usize);
                }
            }
        }
        None
    });
    addr.map(|a| unsafe { std::mem::transmute::<usize, LzoCompress>(a) })
}

/// lzo1x_1_compress() of liblzo2.
fn lzo_compress(data: &[u8]) -> Result<Vec<u8>, String> {
    let f = lzo_compress_fn().ok_or_else(|| "Compression LZO error: -1\n".to_string())?;
    // LZO1X_1_MEM_COMPRESS = 16384 * sizeof(lzo_dict_t)
    let mut wrkmem = vec![0u8; 16384 * 8];
    let mut out = vec![0u8; data.len() + data.len() / 16 + 64 + 3];
    let mut len = 0usize;
    let r = unsafe { f(data.as_ptr(), data.len(), out.as_mut_ptr(), &mut len, wrkmem.as_mut_ptr().cast()) };
    if r != 0 || len > out.len() {
        return Err(format!("Compression LZO error: {r}\n"));
    }
    out.truncate(len);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn compressors_roundtrip() {
        let mut data = b"bareos backup data ".repeat(20_000);
        data.extend((0..70_000u32).map(|i| (i.wrapping_mul(2654435761) >> 13) as u8));
        data.extend(std::iter::repeat_n(0u8, 5000));
        for algo in [COMPRESS_GZIP, COMPRESS_FZFZ, COMPRESS_FZ4L, COMPRESS_FZ4H, COMPRESS_LZO1X] {
            if !compressor_available(algo) {
                continue;
            }
            for input in [&data[..], &data[..100], &data[..7]] {
                let c = compress_data(algo, 6, input).unwrap();
                let rec = with_header(algo, 6, &c);
                assert_eq!(decompress_data("f", STREAM_COMPRESSED_DATA, &rec).unwrap(), input, "{}", compressor_name(algo));
            }
        }
    }

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
