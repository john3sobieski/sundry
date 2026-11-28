//! Splits JPEG, PNG and WebP files into pieces (metadata blocks and everything
//! else) and puts them back together without the blocks that were selected.

use sundry_core::{Error, Result};

use super::{Selection, tiffedit};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Fmt {
    Jpeg,
    Png,
    WebP,
    Gif,
    Tiff,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    /// Picture data and structure: always copied.
    Keep,
    Exif,
    Xmp,
    Iptc,
    Icc,
    Comment,
    /// PNG `tIME` (last modification time).
    Time,
}

#[derive(Debug, Clone)]
pub(crate) struct Piece {
    pub kind: Kind,
    pub start: usize,
    pub end: usize,
    /// Start of the chunk/segment contents (before any prefix such as `Exif\0\0`).
    pub head: usize,
    /// The block itself: the TIFF data of EXIF, the XML of XMP, and so on.
    pub data: usize,
    pub data_end: usize,
    pub label: String,
}

impl Piece {
    fn keep(start: usize, end: usize, label: impl Into<String>) -> Self {
        Self { kind: Kind::Keep, start, end, head: start, data: start, data_end: end, label: label.into() }
    }
}

#[derive(Debug)]
pub(crate) struct Doc {
    pub fmt: Fmt,
    pub pieces: Vec<Piece>,
    /// Bytes after the end of the picture data (JPEG after EOI, PNG after IEND, ...).
    pub trailer: usize,
}

pub(crate) struct Stripped {
    pub bytes: Vec<u8>,
    pub removed: Vec<String>,
}

fn damaged(what: &str) -> Error {
    Error::InvalidInput(format!("damaged {what} file"))
}

pub(crate) fn parse(b: &[u8]) -> Result<Doc> {
    if b.starts_with(&[0xFF, 0xD8, 0xFF]) {
        jpeg(b)
    } else if b.starts_with(b"\x89PNG\r\n\x1a\n") {
        png(b)
    } else if b.len() >= 12 && &b[..4] == b"RIFF" && &b[8..12] == b"WEBP" {
        webp(b)
    } else if b.starts_with(b"GIF8") {
        Ok(whole(b, Fmt::Gif))
    } else if b.starts_with(b"II*\0") || b.starts_with(b"MM\0*") {
        Ok(whole(b, Fmt::Tiff))
    } else if b.len() > 12 && &b[4..8] == b"ftyp" {
        Err(Error::Unsupported("metadata of AVIF/HEIF files is not supported yet".to_owned()))
    } else if crate::codec::probe(b).is_some() {
        Ok(whole(b, Fmt::Other))
    } else {
        Err(Error::InvalidInput("not a supported image file".to_owned()))
    }
}

fn whole(b: &[u8], fmt: Fmt) -> Doc {
    Doc { fmt, pieces: vec![Piece::keep(0, b.len(), "whole file")], trailer: 0 }
}

fn be16(b: &[u8], p: usize) -> usize {
    usize::from(u16::from_be_bytes([b[p], b[p + 1]]))
}

fn jpeg_marker_label(marker: u8) -> String {
    match marker {
        0xC0..=0xC2 => format!("SOF{}", marker - 0xC0),
        0xC4 => "DHT".to_owned(),
        0xDB => "DQT".to_owned(),
        0xDD => "DRI".to_owned(),
        0xE0..=0xEF => format!("APP{}", marker - 0xE0),
        0xFE => "COM".to_owned(),
        _ => format!("marker 0xFF{marker:02X}"),
    }
}

/// Kind, length of the prefix before the block itself, and label of a JPEG segment.
fn jpeg_block(marker: u8, body: &[u8]) -> (Kind, usize, String) {
    let plain = || (Kind::Keep, 0, jpeg_marker_label(marker));
    match marker {
        0xE0 if body.starts_with(b"JFIF\0") => (Kind::Keep, 0, "APP0 JFIF".to_owned()),
        0xE1 if body.starts_with(b"Exif\0\0") => (Kind::Exif, 6, "APP1 Exif".to_owned()),
        0xE1 if body.starts_with(b"http://ns.adobe.com/xap/1.0/\0") => {
            (Kind::Xmp, 29, "APP1 XMP".to_owned())
        }
        0xE1 if body.starts_with(b"http://ns.adobe.com/xmp/extension/\0") => {
            (Kind::Xmp, 35, "APP1 extended XMP".to_owned())
        }
        0xE2 if body.starts_with(b"ICC_PROFILE\0") => (Kind::Icc, 14, "APP2 ICC profile".to_owned()),
        0xE2 if body.starts_with(b"MPF\0") => (Kind::Keep, 0, "APP2 MPF (multi-picture)".to_owned()),
        0xED if body.starts_with(b"Photoshop 3.0\0") => {
            (Kind::Iptc, 14, "APP13 Photoshop/IPTC".to_owned())
        }
        0xEE if body.starts_with(b"Adobe") => (Kind::Keep, 0, "APP14 Adobe".to_owned()),
        0xFE => (Kind::Comment, 0, "COM comment".to_owned()),
        _ => plain(),
    }
}

fn jpeg(b: &[u8]) -> Result<Doc> {
    let mut pieces = vec![Piece::keep(0, 2, "SOI")];
    let mut pos = 2;
    let mut trailer = 0;
    loop {
        if pos + 4 > b.len() || b[pos] != 0xFF {
            return Err(damaged("JPEG"));
        }
        let marker = b[pos + 1];
        if marker == 0xFF || marker == 0x00 || marker == 0x01 || (0xD0..=0xD9).contains(&marker) {
            return Err(damaged("JPEG"));
        }
        let len = be16(b, pos + 2);
        let end = pos + 2 + len;
        if len < 2 || end > b.len() {
            return Err(damaged("JPEG"));
        }
        if marker == 0xDA {
            // Start of scan: the rest is picture data.
            pieces.push(Piece::keep(pos, b.len(), "SOS + picture data"));
            trailer = jpeg_trailer(b, end);
            break;
        }
        let body = &b[pos + 4..end];
        let head = pos + 4;
        let (kind, skip, label) = jpeg_block(marker, body);
        let mut piece = Piece::keep(pos, end, label);
        if kind != Kind::Keep {
            piece.kind = kind;
            piece.head = head;
            piece.data = head + skip;
            if piece.data > end {
                return Err(damaged("JPEG"));
            }
        }
        piece.data_end = end;
        pieces.push(piece);
        pos = end;
    }
    Ok(Doc { fmt: Fmt::Jpeg, pieces, trailer })
}

/// Number of bytes after EOI (0 when the end cannot be found).
fn jpeg_trailer(b: &[u8], mut i: usize) -> usize {
    while i + 1 < b.len() {
        if b[i] != 0xFF {
            i += 1;
            continue;
        }
        let m = b[i + 1];
        if m == 0xD9 {
            return b.len() - (i + 2);
        }
        if m == 0xFF {
            i += 1;
        } else if m == 0x00 || (0xD0..=0xD7).contains(&m) {
            i += 2;
        } else if i + 3 < b.len() {
            i += 2 + be16(b, i + 2).max(2);
        } else {
            return 0;
        }
    }
    0
}

/// `(keyword, text)` of a PNG text chunk.
pub(crate) fn png_text(kind: &[u8], data: &[u8]) -> (String, String) {
    let latin1 = |s: &[u8]| s.iter().map(|&c| c as char).collect::<String>();
    let Some(nul) = data.iter().position(|&c| c == 0) else {
        return (String::new(), latin1(data));
    };
    let key = latin1(&data[..nul]);
    let rest = &data[nul + 1..];
    match kind {
        b"tEXt" => (key, latin1(rest)),
        b"zTXt" => (key, format!("(compressed text, {} bytes)", rest.len())),
        _ => {
            // iTXt: flag, method, language\0, translated keyword\0, text
            if rest.len() < 2 {
                return (key, String::new());
            }
            let compressed = rest[0] != 0;
            let mut tail = &rest[2..];
            for _ in 0..2 {
                match tail.iter().position(|&c| c == 0) {
                    Some(n) => tail = &tail[n + 1..],
                    None => return (key, String::new()),
                }
            }
            if compressed {
                (key, format!("(compressed text, {} bytes)", tail.len()))
            } else {
                (key, String::from_utf8_lossy(tail).into_owned())
            }
        }
    }
}

fn png(b: &[u8]) -> Result<Doc> {
    let mut pieces = vec![Piece::keep(0, 8, "PNG signature")];
    let mut pos = 8;
    let mut trailer = 0;
    while pos < b.len() {
        if pos + 12 > b.len() {
            return Err(damaged("PNG"));
        }
        let len = u32::from_be_bytes([b[pos], b[pos + 1], b[pos + 2], b[pos + 3]]) as usize;
        let kind = &b[pos + 4..pos + 8];
        let data = pos + 8;
        let end = match data.checked_add(len).and_then(|e| e.checked_add(4)) {
            Some(end) if end <= b.len() => end,
            _ => return Err(damaged("PNG")),
        };
        let data_end = data + len;
        let name = String::from_utf8_lossy(kind).into_owned();
        let mut piece = Piece { kind: Kind::Keep, start: pos, end, head: data, data, data_end, label: name.clone() };
        match kind {
            b"eXIf" => {
                piece.kind = Kind::Exif;
                piece.label = "eXIf".to_owned();
            }
            b"tEXt" | b"zTXt" | b"iTXt" => {
                let (key, _) = png_text(kind, &b[data..data_end]);
                piece.kind =
                    if key == "XML:com.adobe.xmp" && kind == b"iTXt" { Kind::Xmp } else { Kind::Comment };
                piece.label = format!("{name} {key}");
            }
            b"tIME" => piece.kind = Kind::Time,
            b"iCCP" => piece.kind = Kind::Icc,
            _ => {}
        }
        pieces.push(piece);
        pos = end;
        if kind == b"IEND" {
            if end < b.len() {
                trailer = b.len() - end;
                pieces.push(Piece::keep(end, b.len(), "data after IEND"));
            }
            break;
        }
    }
    Ok(Doc { fmt: Fmt::Png, pieces, trailer })
}

fn webp(b: &[u8]) -> Result<Doc> {
    let declared = u32::from_le_bytes([b[4], b[5], b[6], b[7]]) as usize;
    let limit = declared.saturating_add(8).min(b.len());
    if limit < 12 {
        return Err(damaged("WebP"));
    }
    let mut pieces = vec![Piece::keep(0, 12, "RIFF header")];
    let mut pos = 12;
    while pos + 8 <= limit {
        let kind = &b[pos..pos + 4];
        let len = u32::from_le_bytes([b[pos + 4], b[pos + 5], b[pos + 6], b[pos + 7]]) as usize;
        let data = pos + 8;
        let data_end = data.saturating_add(len);
        let end = data_end.saturating_add(len & 1);
        if end > limit {
            return Err(damaged("WebP"));
        }
        let name = String::from_utf8_lossy(kind).trim().to_owned();
        let mut piece = Piece { kind: Kind::Keep, start: pos, end, head: data, data, data_end, label: name };
        match kind {
            b"EXIF" => {
                piece.kind = Kind::Exif;
                if b[data..data_end].starts_with(b"Exif\0\0") {
                    piece.data = data + 6;
                }
            }
            b"XMP " => piece.kind = Kind::Xmp,
            b"ICCP" => piece.kind = Kind::Icc,
            _ => {}
        }
        pieces.push(piece);
        pos = end;
    }
    if pos < limit {
        pieces.push(Piece::keep(pos, limit, "padding"));
    }
    let mut trailer = 0;
    if limit < b.len() {
        trailer = b.len() - limit;
        pieces.push(Piece::keep(limit, b.len(), "data after RIFF"));
    }
    Ok(Doc { fmt: Fmt::WebP, pieces, trailer })
}

fn crc32(parts: &[&[u8]]) -> u32 {
    let mut crc = !0u32;
    for part in parts {
        for &byte in *part {
            crc ^= u32::from(byte);
            for _ in 0..8 {
                crc = if crc & 1 != 0 { 0xEDB8_8320 ^ (crc >> 1) } else { crc >> 1 };
            }
        }
    }
    !crc
}

/// Writes an edited EXIF block back in the form the container expects.
fn write_exif(fmt: Fmt, b: &[u8], p: &Piece, tiff: &[u8], out: &mut Vec<u8>) {
    let prefix = &b[p.head..p.data];
    match fmt {
        Fmt::Jpeg => {
            let len = (2 + prefix.len() + tiff.len()) as u16;
            out.extend_from_slice(&[0xFF, 0xE1]);
            out.extend_from_slice(&len.to_be_bytes());
            out.extend_from_slice(prefix);
            out.extend_from_slice(tiff);
        }
        Fmt::Png => {
            out.extend_from_slice(&(tiff.len() as u32).to_be_bytes());
            out.extend_from_slice(b"eXIf");
            out.extend_from_slice(tiff);
            out.extend_from_slice(&crc32(&[b"eXIf", tiff]).to_be_bytes());
        }
        _ => {
            let len = prefix.len() + tiff.len();
            out.extend_from_slice(b"EXIF");
            out.extend_from_slice(&(len as u32).to_le_bytes());
            out.extend_from_slice(prefix);
            out.extend_from_slice(tiff);
            if len % 2 == 1 {
                out.push(0);
            }
        }
    }
}

/// What disappeared completely (matters for the WebP feature flags).
#[derive(Default)]
struct Gone {
    exif: bool,
    xmp: bool,
    icc: bool,
}

pub(crate) fn strip(b: &[u8], doc: &Doc, sel: &Selection) -> Result<Stripped> {
    match doc.fmt {
        Fmt::Tiff | Fmt::Gif => {
            return Err(Error::Unsupported(
                "removing metadata from TIFF and GIF files is not supported yet".to_owned(),
            ));
        }
        Fmt::Other => return Ok(Stripped { bytes: b.to_vec(), removed: Vec::new() }),
        _ => {}
    }
    let mut out = Vec::with_capacity(b.len());
    let mut removed = Vec::new();
    let mut gone = Gone::default();
    for p in &doc.pieces {
        let raw = &b[p.start..p.end];
        let drop_it = |selected: bool, removed: &mut Vec<String>| {
            if selected {
                removed.push(p.label.clone());
            }
            selected
        };
        match p.kind {
            Kind::Keep => out.extend_from_slice(raw),
            Kind::Xmp => {
                if drop_it(sel.xmp, &mut removed) {
                    gone.xmp = true;
                } else {
                    out.extend_from_slice(raw);
                }
            }
            Kind::Iptc => {
                if !drop_it(sel.iptc, &mut removed) {
                    out.extend_from_slice(raw);
                }
            }
            Kind::Icc => {
                if drop_it(sel.icc, &mut removed) {
                    gone.icc = true;
                } else {
                    out.extend_from_slice(raw);
                }
            }
            Kind::Comment => {
                if !drop_it(sel.comments, &mut removed) {
                    out.extend_from_slice(raw);
                }
            }
            Kind::Time => {
                if !drop_it(sel.datetime, &mut removed) {
                    out.extend_from_slice(raw);
                }
            }
            Kind::Exif => match tiffedit::edit(&b[p.data..p.data_end], sel) {
                Ok(edit) if edit.removed.is_empty() => out.extend_from_slice(raw),
                Ok(edit) => {
                    removed.extend(edit.removed);
                    match edit.tiff {
                        Some(tiff) => write_exif(doc.fmt, b, p, &tiff, &mut out),
                        None => gone.exif = true,
                    }
                }
                Err(_) if sel.touches_exif() => {
                    removed.push("EXIF block (unreadable, removed completely)".to_owned());
                    gone.exif = true;
                }
                Err(_) => out.extend_from_slice(raw),
            },
        }
    }
    if doc.fmt == Fmt::WebP {
        finish_webp(&mut out, &gone);
    }
    Ok(Stripped { bytes: out, removed })
}

/// Fixes the RIFF size and the feature flags of the `VP8X` chunk.
fn finish_webp(out: &mut [u8], gone: &Gone) {
    let size = (out.len() - 8) as u32;
    out[4..8].copy_from_slice(&size.to_le_bytes());
    if out.len() > 20 && &out[12..16] == b"VP8X" {
        if gone.icc {
            out[20] &= !0x20;
        }
        if gone.exif {
            out[20] &= !0x08;
        }
        if gone.xmp {
            out[20] &= !0x04;
        }
    }
}

#[cfg(test)]
pub(crate) fn crc32_for_tests(parts: &[&[u8]]) -> u32 {
    crc32(parts)
}
