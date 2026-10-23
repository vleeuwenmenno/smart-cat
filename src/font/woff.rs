//! WOFF and WOFF2 to plain sfnt (TrueType/OpenType) conversion.
//!
//! WOFF 1.0 stores each table zlib-compressed; WOFF2 stores all tables in one brotli
//! stream and may transform `glyf`/`loca` (and `hmtx`) into a denser layout, which is
//! reconstructed here following the W3C WOFF2 specification, section 5. The result is
//! only parsed (never written anywhere), so table checksums and the `head` checksum
//! adjustment are not recomputed beyond what parsers look at.
//!
//! The input is untrusted: every read is bounds-checked and the output size is capped.

use std::io::Read;

/// Largest sfnt a WOFF/WOFF2 file may expand to.
pub const MAX_SFNT_BYTES: usize = 256 << 20;

/// Which web font wrapper a file uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wrapper {
    Woff,
    Woff2,
}

pub fn wrapper(data: &[u8]) -> Option<Wrapper> {
    match data.get(..4)? {
        b"wOFF" => Some(Wrapper::Woff),
        b"wOF2" => Some(Wrapper::Woff2),
        _ => None,
    }
}

/// Big-endian reader over a byte slice; every read fails cleanly at the end.
struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Reader { data, pos: 0 }
    }

    fn bytes(&mut self, n: usize) -> Result<&'a [u8], String> {
        let end = self.pos.checked_add(n).ok_or("truncated data")?;
        let out = self.data.get(self.pos..end).ok_or("truncated data")?;
        self.pos = end;
        Ok(out)
    }

    fn u8(&mut self) -> Result<u8, String> {
        Ok(self.bytes(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, String> {
        let b = self.bytes(2)?;
        Ok(u16::from_be_bytes([b[0], b[1]]))
    }

    fn i16(&mut self) -> Result<i16, String> {
        Ok(self.u16()? as i16)
    }

    fn u32(&mut self) -> Result<u32, String> {
        let b = self.bytes(4)?;
        Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }

    /// WOFF2 `UIntBase128`: up to five bytes, seven bits each, no leading zeros.
    fn base128(&mut self) -> Result<u32, String> {
        let mut value: u32 = 0;
        for i in 0..5 {
            let b = self.u8()?;
            if i == 0 && b == 0x80 {
                return Err("bad UIntBase128".into());
            }
            if value & 0xfe00_0000 != 0 {
                return Err("UIntBase128 overflow".into());
            }
            value = (value << 7) | u32::from(b & 0x7f);
            if b & 0x80 == 0 {
                return Ok(value);
            }
        }
        Err("UIntBase128 too long".into())
    }

    /// WOFF2 `255UInt16`.
    fn u255(&mut self) -> Result<u16, String> {
        Ok(match self.u8()? {
            253 => self.u16()?,
            254 => 506 + u16::from(self.u8()?),
            255 => 253 + u16::from(self.u8()?),
            code => u16::from(code),
        })
    }
}

/// One table of the rebuilt font.
struct Table {
    tag: [u8; 4],
    data: Vec<u8>,
}

/// Converts a WOFF or WOFF2 file to an sfnt (or a TrueType collection).
pub fn to_sfnt(data: &[u8]) -> Result<Vec<u8>, String> {
    match wrapper(data) {
        Some(Wrapper::Woff) => woff1(data),
        Some(Wrapper::Woff2) => woff2(data),
        None => Err("not a WOFF file".into()),
    }
}

fn woff1(data: &[u8]) -> Result<Vec<u8>, String> {
    let mut r = Reader::new(data);
    r.bytes(4)?;
    let flavor = r.u32()?;
    r.u32()?; // length
    let num_tables = r.u16()?;
    r.u16()?;
    let total = r.u32()? as usize;
    if total > MAX_SFNT_BYTES {
        return Err("font too large".into());
    }
    r.bytes(24)?; // versions, metadata and private blocks
    let mut tables = Vec::with_capacity(usize::from(num_tables));
    let mut budget = MAX_SFNT_BYTES;
    for _ in 0..num_tables {
        let tag: [u8; 4] = r.bytes(4)?.try_into().unwrap_or_default();
        let offset = r.u32()? as usize;
        let comp_len = r.u32()? as usize;
        let orig_len = r.u32()? as usize;
        r.u32()?; // checksum
        let stored = data
            .get(offset..offset.saturating_add(comp_len))
            .ok_or("table outside the file")?;
        if orig_len > budget {
            return Err("font too large".into());
        }
        budget -= orig_len;
        let table = if comp_len < orig_len {
            let mut out = Vec::with_capacity(orig_len);
            flate2::read::ZlibDecoder::new(stored)
                .take(orig_len as u64)
                .read_to_end(&mut out)
                .map_err(|e| format!("bad compressed table: {e}"))?;
            if out.len() != orig_len {
                return Err("compressed table has the wrong size".into());
            }
            out
        } else if comp_len == orig_len {
            stored.to_vec()
        } else {
            return Err("bad table length".into());
        };
        tables.push(Table { tag, data: table });
    }
    let all: Vec<usize> = (0..tables.len()).collect();
    Ok(assemble(&[(flavor, all)], &tables))
}

/// The 63 table tags WOFF2 encodes as an index.
const KNOWN_TAGS: [&[u8; 4]; 63] = [
    b"cmap", b"head", b"hhea", b"hmtx", b"maxp", b"name", b"OS/2", b"post", b"cvt ", b"fpgm",
    b"glyf", b"loca", b"prep", b"CFF ", b"VORG", b"EBDT", b"EBLC", b"gasp", b"hdmx", b"kern",
    b"LTSH", b"PCLT", b"VDMX", b"vhea", b"vmtx", b"BASE", b"GDEF", b"GPOS", b"GSUB", b"EBSC",
    b"JSTF", b"MATH", b"CBDT", b"CBLC", b"COLR", b"CPAL", b"SVG ", b"sbix", b"acnt", b"avar",
    b"bdat", b"bloc", b"bsln", b"cvar", b"fdsc", b"feat", b"fmtx", b"fvar", b"gvar", b"hsty",
    b"just", b"lcar", b"mort", b"morx", b"opbd", b"prop", b"trak", b"Zapf", b"Silf", b"Glat",
    b"Gloc", b"Feat", b"Sill",
];

struct Entry {
    tag: [u8; 4],
    transformed: bool,
    /// Length in the decompressed stream.
    stream_len: usize,
    orig_len: usize,
}

fn woff2(data: &[u8]) -> Result<Vec<u8>, String> {
    let mut r = Reader::new(data);
    r.bytes(4)?;
    let flavor = r.u32()?;
    r.u32()?; // length
    let num_tables = r.u16()?;
    r.u16()?;
    let total = r.u32()? as usize;
    if total > MAX_SFNT_BYTES {
        return Err("font too large".into());
    }
    let compressed_len = r.u32()? as usize;
    r.bytes(24)?; // versions, metadata and private blocks

    let mut entries = Vec::with_capacity(usize::from(num_tables));
    let mut stream_total: usize = 0;
    for _ in 0..num_tables {
        let flags = r.u8()?;
        let tag = match flags & 0x3f {
            63 => r.bytes(4)?.try_into().unwrap_or_default(),
            i => *KNOWN_TAGS[usize::from(i)],
        };
        let version = flags >> 6;
        let orig_len = r.base128()? as usize;
        let transformed = match &tag {
            b"glyf" | b"loca" => version == 0,
            _ => version != 0,
        };
        let stream_len = if transformed {
            r.base128()? as usize
        } else {
            orig_len
        };
        if transformed && !matches!(&tag, b"glyf" | b"loca" | b"hmtx") {
            return Err("unknown table transform".into());
        }
        stream_total = stream_total
            .checked_add(stream_len)
            .filter(|&t| t <= MAX_SFNT_BYTES)
            .ok_or("font too large")?;
        entries.push(Entry {
            tag,
            transformed,
            stream_len,
            orig_len,
        });
    }

    let fonts: Vec<(u32, Vec<usize>)> = if flavor == u32::from_be_bytes(*b"ttcf") {
        r.u32()?; // collection version
        let count = r.u255()?;
        let mut fonts = Vec::with_capacity(usize::from(count));
        for _ in 0..count {
            let n = r.u255()?;
            let font_flavor = r.u32()?;
            let mut indices = Vec::with_capacity(usize::from(n));
            for _ in 0..n {
                let i = usize::from(r.u255()?);
                if i >= entries.len() {
                    return Err("bad collection table index".into());
                }
                indices.push(i);
            }
            fonts.push((font_flavor, indices));
        }
        fonts
    } else {
        vec![(flavor, (0..entries.len()).collect())]
    };

    let compressed = r.bytes(compressed_len)?;
    let mut stream = Vec::with_capacity(stream_total.min(64 << 20));
    brotli_decompressor::Decompressor::new(compressed, 4096)
        .take(stream_total as u64)
        .read_to_end(&mut stream)
        .map_err(|e| format!("bad brotli stream: {e}"))?;
    if stream.len() != stream_total {
        return Err("compressed stream has the wrong size".into());
    }

    // Slices of the stream per entry.
    let mut slices = Vec::with_capacity(entries.len());
    let mut pos = 0;
    for e in &entries {
        slices.push(&stream[pos..pos + e.stream_len]);
        pos += e.stream_len;
    }

    let mut tables: Vec<Option<Table>> = (0..entries.len()).map(|_| None).collect();
    // glyf entries rebuilt so far: (glyf index, loca bytes, x minimums).
    let mut rebuilt: Vec<(usize, Vec<u8>, Vec<i16>)> = Vec::new();
    let mut budget = MAX_SFNT_BYTES;
    for (i, e) in entries.iter().enumerate() {
        if e.transformed && &e.tag == b"glyf" {
            let glyf = rebuild_glyf(slices[i], budget)?;
            budget = budget.saturating_sub(glyf.glyf.len() + glyf.loca.len());
            tables[i] = Some(Table {
                tag: e.tag,
                data: glyf.glyf,
            });
            rebuilt.push((i, glyf.loca, glyf.x_min));
        } else if !e.transformed {
            if e.orig_len > budget {
                return Err("font too large".into());
            }
            budget -= e.orig_len;
            tables[i] = Some(Table {
                tag: e.tag,
                data: slices[i].to_vec(),
            });
        }
    }
    for (i, e) in entries.iter().enumerate() {
        if !e.transformed || &e.tag == b"glyf" {
            continue;
        }
        // The glyf table of the font this table belongs to.
        let font = fonts
            .iter()
            .find(|(_, idx)| idx.contains(&i))
            .ok_or("table not in any font")?;
        let glyf = font
            .1
            .iter()
            .find_map(|&g| rebuilt.iter().find(|(gi, _, _)| *gi == g))
            .ok_or("transformed table without glyf")?;
        let data = if &e.tag == b"loca" {
            glyf.1.clone()
        } else {
            let hhea = font
                .1
                .iter()
                .find(|&&t| &entries[t].tag == b"hhea")
                .and_then(|&t| tables[t].as_ref())
                .ok_or("hmtx without hhea")?;
            let num_h = Reader {
                data: &hhea.data,
                pos: 34,
            }
            .u16()?;
            rebuild_hmtx(slices[i], num_h, &glyf.2)?
        };
        tables[i] = Some(Table { tag: e.tag, data });
    }
    let tables: Vec<Table> = tables
        .into_iter()
        .map(|t| t.ok_or_else(|| "missing table".to_string()))
        .collect::<Result<_, _>>()?;
    Ok(assemble(&fonts, &tables))
}

struct Glyf {
    glyf: Vec<u8>,
    loca: Vec<u8>,
    x_min: Vec<i16>,
}

/// Rebuilds `glyf` and `loca` from the transformed glyf table (WOFF2 section 5.1).
fn rebuild_glyf(data: &[u8], budget: usize) -> Result<Glyf, String> {
    let mut r = Reader::new(data);
    r.u16()?; // reserved
    let option_flags = r.u16()?;
    let num_glyphs = usize::from(r.u16()?);
    let index_format = r.u16()?;
    let mut sizes = [0usize; 7];
    for s in &mut sizes {
        *s = r.u32()? as usize;
    }
    let mut streams = Vec::with_capacity(7);
    for s in sizes {
        streams.push(r.bytes(s)?);
    }
    let mut n_contour = Reader::new(streams[0]);
    let mut n_points = Reader::new(streams[1]);
    let mut flags = Reader::new(streams[2]);
    let mut glyphs = Reader::new(streams[3]);
    let mut composite = Reader::new(streams[4]);
    let bbox_all = streams[5];
    let bitmap_len = num_glyphs.div_ceil(32) * 4;
    let bbox_bitmap = bbox_all.get(..bitmap_len).ok_or("truncated data")?;
    let mut bbox = Reader::new(&bbox_all[bitmap_len..]);
    let mut instructions = Reader::new(streams[6]);
    let overlap = if option_flags & 1 != 0 {
        Some(r.bytes(num_glyphs.div_ceil(8))?)
    } else {
        None
    };
    let has_bit = |bits: &[u8], i: usize| bits[i >> 3] & (0x80 >> (i & 7)) != 0;

    let mut glyf = Vec::new();
    let mut offsets = Vec::with_capacity(num_glyphs + 1);
    let mut x_min = Vec::with_capacity(num_glyphs);
    let mut points: Vec<(i32, i32, bool)> = Vec::new();
    for g in 0..num_glyphs {
        offsets.push(glyf.len());
        let contours = n_contour.i16()?;
        let explicit_bbox = has_bit(bbox_bitmap, g);
        if contours == 0 {
            if explicit_bbox {
                return Err("empty glyph with a bounding box".into());
            }
            x_min.push(0);
            continue;
        }
        let start = glyf.len();
        if contours < 0 {
            if !explicit_bbox {
                return Err("composite glyph without a bounding box".into());
            }
            glyf.extend_from_slice(&(-1i16).to_be_bytes());
            let b = bbox.bytes(8)?;
            glyf.extend_from_slice(b);
            x_min.push(i16::from_be_bytes([b[0], b[1]]));
            let comp_start = composite.pos;
            let mut have_instructions = false;
            loop {
                let flag = composite.u16()?;
                composite.u16()?; // glyph index
                let mut len = if flag & 0x0001 != 0 { 4 } else { 2 };
                if flag & 0x0008 != 0 {
                    len += 2;
                } else if flag & 0x0040 != 0 {
                    len += 4;
                } else if flag & 0x0080 != 0 {
                    len += 8;
                }
                composite.bytes(len)?;
                have_instructions |= flag & 0x0100 != 0;
                if flag & 0x0020 == 0 {
                    break;
                }
            }
            glyf.extend_from_slice(&streams[4][comp_start..composite.pos]);
            if have_instructions {
                let n = glyphs.u255()?;
                glyf.extend_from_slice(&n.to_be_bytes());
                glyf.extend_from_slice(instructions.bytes(usize::from(n))?);
            }
        } else {
            let mut end_points = Vec::with_capacity(contours as usize);
            let mut total: usize = 0;
            for _ in 0..contours {
                let n = usize::from(n_points.u255()?);
                total += n;
                if total > 0xffff {
                    return Err("too many points".into());
                }
                end_points.push((total as u16).wrapping_sub(1));
            }
            points.clear();
            let (mut x, mut y) = (0i32, 0i32);
            for _ in 0..total {
                let flag = flags.u8()?;
                let on_curve = flag >> 7 == 0;
                let (dx, dy) = triplet(flag & 0x7f, &mut glyphs)?;
                x = x.wrapping_add(dx);
                y = y.wrapping_add(dy);
                points.push((x, y, on_curve));
            }
            let n_instr = glyphs.u255()?;
            let (xmin, ymin, xmax, ymax) = if explicit_bbox {
                let b = bbox.bytes(8)?;
                let v = |i: usize| i16::from_be_bytes([b[i], b[i + 1]]);
                (v(0), v(2), v(4), v(6))
            } else {
                let clamp = |v: i32| v.clamp(-32768, 32767) as i16;
                let xs = points.iter().map(|p| p.0);
                let ys = points.iter().map(|p| p.1);
                (
                    clamp(xs.clone().min().unwrap_or(0)),
                    clamp(ys.clone().min().unwrap_or(0)),
                    clamp(xs.max().unwrap_or(0)),
                    clamp(ys.max().unwrap_or(0)),
                )
            };
            x_min.push(xmin);
            glyf.extend_from_slice(&contours.to_be_bytes());
            for v in [xmin, ymin, xmax, ymax] {
                glyf.extend_from_slice(&v.to_be_bytes());
            }
            for e in &end_points {
                glyf.extend_from_slice(&e.to_be_bytes());
            }
            glyf.extend_from_slice(&n_instr.to_be_bytes());
            glyf.extend_from_slice(instructions.bytes(usize::from(n_instr))?);
            let overlap_first = overlap.is_some_and(|bits| has_bit(bits, g));
            encode_points(&mut glyf, &points, overlap_first);
        }
        // Pad to four bytes, which both loca formats can address.
        while (glyf.len() - start) % 4 != 0 {
            glyf.push(0);
        }
        if glyf.len() > budget {
            return Err("font too large".into());
        }
    }
    offsets.push(glyf.len());
    let mut loca = Vec::with_capacity(offsets.len() * 4);
    for off in offsets {
        if index_format == 0 {
            let half = u16::try_from(off / 2).map_err(|_| "glyf too large for short loca")?;
            loca.extend_from_slice(&half.to_be_bytes());
        } else {
            loca.extend_from_slice(&(off as u32).to_be_bytes());
        }
    }
    Ok(Glyf { glyf, loca, x_min })
}

/// One point's coordinate deltas from the flag and the glyph stream (WOFF2 5.2).
fn triplet(flag: u8, r: &mut Reader) -> Result<(i32, i32), String> {
    let sign = |f: u8, v: i32| if f & 1 != 0 { v } else { -v };
    let flag32 = i32::from(flag);
    Ok(if flag < 10 {
        let b = i32::from(r.u8()?);
        (0, sign(flag, ((flag32 & 14) << 7) + b))
    } else if flag < 20 {
        let b = i32::from(r.u8()?);
        (sign(flag, (((flag32 - 10) & 14) << 7) + b), 0)
    } else if flag < 84 {
        let b0 = flag32 - 20;
        let b1 = i32::from(r.u8()?);
        (
            sign(flag, 1 + (b0 & 0x30) + (b1 >> 4)),
            sign(flag >> 1, 1 + ((b0 & 0x0c) << 2) + (b1 & 0x0f)),
        )
    } else if flag < 120 {
        let b0 = flag32 - 84;
        let b = r.bytes(2)?;
        (
            sign(flag, 1 + ((b0 / 12) << 8) + i32::from(b[0])),
            sign(flag >> 1, 1 + (((b0 % 12) >> 2) << 8) + i32::from(b[1])),
        )
    } else if flag < 124 {
        let b = r.bytes(3)?;
        let b2 = i32::from(b[1]);
        (
            sign(flag, (i32::from(b[0]) << 4) + (b2 >> 4)),
            sign(flag >> 1, ((b2 & 0x0f) << 8) + i32::from(b[2])),
        )
    } else {
        let b = r.bytes(4)?;
        (
            sign(flag, (i32::from(b[0]) << 8) + i32::from(b[1])),
            sign(flag >> 1, (i32::from(b[2]) << 8) + i32::from(b[3])),
        )
    })
}

/// Writes the flags and coordinates of a simple glyph in the plain glyf encoding.
fn encode_points(out: &mut Vec<u8>, points: &[(i32, i32, bool)], overlap_first: bool) {
    let mut flags = Vec::with_capacity(points.len());
    let mut xs = Vec::new();
    let mut ys = Vec::new();
    let (mut px, mut py) = (0i32, 0i32);
    for (i, &(x, y, on)) in points.iter().enumerate() {
        let mut flag = u8::from(on);
        if i == 0 && overlap_first {
            flag |= 0x40;
        }
        let dx = x.wrapping_sub(px);
        let dy = y.wrapping_sub(py);
        (px, py) = (x, y);
        if dx == 0 {
            flag |= 0x10;
        } else if dx.unsigned_abs() < 256 {
            flag |= 0x02 | if dx > 0 { 0x10 } else { 0 };
            xs.push(dx.unsigned_abs() as u8);
        } else {
            xs.extend_from_slice(&(dx as i16).to_be_bytes());
        }
        if dy == 0 {
            flag |= 0x20;
        } else if dy.unsigned_abs() < 256 {
            flag |= 0x04 | if dy > 0 { 0x20 } else { 0 };
            ys.push(dy.unsigned_abs() as u8);
        } else {
            ys.extend_from_slice(&(dy as i16).to_be_bytes());
        }
        flags.push(flag);
    }
    out.extend_from_slice(&flags);
    out.extend_from_slice(&xs);
    out.extend_from_slice(&ys);
}

/// Rebuilds `hmtx` from its transformed form (WOFF2 5.4); missing left side bearings
/// are the glyphs' `xMin`.
fn rebuild_hmtx(data: &[u8], num_h: u16, x_min: &[i16]) -> Result<Vec<u8>, String> {
    let mut r = Reader::new(data);
    let flags = r.u8()?;
    let num_h = usize::from(num_h);
    let num_glyphs = x_min.len();
    if num_h == 0 || num_h > num_glyphs {
        return Err("bad hmtx".into());
    }
    let mut advances = Vec::with_capacity(num_h);
    for _ in 0..num_h {
        advances.push(r.u16()?);
    }
    let mut out = Vec::with_capacity(num_h * 4 + (num_glyphs - num_h) * 2);
    for (i, adv) in advances.iter().enumerate() {
        let lsb = if flags & 1 == 0 { r.i16()? } else { x_min[i] };
        out.extend_from_slice(&adv.to_be_bytes());
        out.extend_from_slice(&lsb.to_be_bytes());
    }
    for &xm in &x_min[num_h..] {
        let lsb = if flags & 2 == 0 { r.i16()? } else { xm };
        out.extend_from_slice(&lsb.to_be_bytes());
    }
    Ok(out)
}

/// Lays out one sfnt (or a `ttcf` collection when there are several fonts) from the
/// tables; fonts in a collection share table data.
fn assemble(fonts: &[(u32, Vec<usize>)], tables: &[Table]) -> Vec<u8> {
    let collection = fonts.len() > 1;
    let header_len = if collection { 12 + 4 * fonts.len() } else { 0 };
    let dir_len: usize = fonts.iter().map(|(_, t)| 12 + 16 * t.len()).sum();
    let mut out = vec![0u8; header_len + dir_len];
    // Table data, each padded to four bytes.
    let mut offsets = Vec::with_capacity(tables.len());
    for t in tables {
        offsets.push(out.len());
        out.extend_from_slice(&t.data);
        out.resize(out.len().next_multiple_of(4), 0);
    }
    if collection {
        out[..4].copy_from_slice(b"ttcf");
        out[4..8].copy_from_slice(&0x0001_0000u32.to_be_bytes());
        out[8..12].copy_from_slice(&(fonts.len() as u32).to_be_bytes());
    }
    let mut dir = header_len;
    for (n, (flavor, indices)) in fonts.iter().enumerate() {
        if collection {
            out[12 + 4 * n..16 + 4 * n].copy_from_slice(&(dir as u32).to_be_bytes());
        }
        let mut sorted = indices.clone();
        sorted.sort_by_key(|&i| tables[i].tag);
        let count = sorted.len() as u16;
        let entry_selector = if count == 0 {
            0
        } else {
            15 - count.leading_zeros() as u16
        };
        let search_range = (1u16 << entry_selector).wrapping_mul(16);
        let mut head = Vec::with_capacity(12);
        head.extend_from_slice(&flavor.to_be_bytes());
        head.extend_from_slice(&count.to_be_bytes());
        head.extend_from_slice(&search_range.to_be_bytes());
        head.extend_from_slice(&entry_selector.to_be_bytes());
        head.extend_from_slice(&(count.wrapping_mul(16).wrapping_sub(search_range)).to_be_bytes());
        out[dir..dir + 12].copy_from_slice(&head);
        dir += 12;
        for i in sorted {
            let t = &tables[i];
            let mut rec = [0u8; 16];
            rec[..4].copy_from_slice(&t.tag);
            rec[4..8].copy_from_slice(&checksum(&t.data).to_be_bytes());
            rec[8..12].copy_from_slice(&(offsets[i] as u32).to_be_bytes());
            rec[12..16].copy_from_slice(&(t.data.len() as u32).to_be_bytes());
            out[dir..dir + 16].copy_from_slice(&rec);
            dir += 16;
        }
    }
    out
}

fn checksum(data: &[u8]) -> u32 {
    data.chunks(4).fold(0u32, |sum, c| {
        let mut b = [0u8; 4];
        b[..c.len()].copy_from_slice(c);
        sum.wrapping_add(u32::from_be_bytes(b))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn variable_length_integers() {
        assert_eq!(Reader::new(&[0x3f]).base128(), Ok(63));
        assert_eq!(Reader::new(&[0x81, 0x00]).base128(), Ok(128));
        assert!(Reader::new(&[0x80, 0x01]).base128().is_err());
        assert!(Reader::new(&[0xff; 6]).base128().is_err());
        assert_eq!(Reader::new(&[252]).u255(), Ok(252));
        assert_eq!(Reader::new(&[255, 0]).u255(), Ok(253));
        assert_eq!(Reader::new(&[254, 0]).u255(), Ok(506));
        assert_eq!(Reader::new(&[253, 0x12, 0x34]).u255(), Ok(0x1234));
    }

    #[test]
    fn triplets_follow_the_spec_table() {
        // flag 0: dx 0, dy -(0 + b); flag 1: dy +b.
        assert_eq!(triplet(0, &mut Reader::new(&[5])), Ok((0, -5)));
        assert_eq!(triplet(1, &mut Reader::new(&[5])), Ok((0, 5)));
        assert_eq!(triplet(11, &mut Reader::new(&[7])), Ok((7, 0)));
        // flag 20..84: 4-bit pairs.
        assert_eq!(triplet(23, &mut Reader::new(&[0x12])), Ok((2, 3)));
        // flag 124..: two 16-bit values.
        assert_eq!(
            triplet(127, &mut Reader::new(&[0x01, 0x00, 0x00, 0x10])),
            Ok((256, 16))
        );
        assert!(triplet(127, &mut Reader::new(&[0x01])).is_err());
    }

    #[test]
    fn rejects_truncated_files() {
        assert!(to_sfnt(b"wOFF\0\0").is_err());
        assert!(to_sfnt(b"wOF2\0\0\0\0").is_err());
        assert!(to_sfnt(b"OTTO").is_err());
    }
}
