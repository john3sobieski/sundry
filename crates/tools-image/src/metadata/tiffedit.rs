//! In-place EXIF/TIFF editing: entries are deleted without moving anything else
//! (offsets stay valid) and removed values are zeroed.

use ::exif::{Context, Tag};

use super::Selection;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Ctx {
    Main,
    Exif,
    Gps,
    Interop,
}

impl Ctx {
    pub(crate) fn from_exif(context: Context) -> Self {
        match context {
            Context::Exif => Ctx::Exif,
            Context::Gps => Ctx::Gps,
            Context::Interop => Ctx::Interop,
            _ => Ctx::Main,
        }
    }

    fn to_exif(self) -> Context {
        match self {
            Ctx::Main => Context::Tiff,
            Ctx::Exif => Context::Exif,
            Ctx::Gps => Context::Gps,
            Ctx::Interop => Context::Interop,
        }
    }

    fn label(self, ifd: u16) -> &'static str {
        match (self, ifd) {
            (Ctx::Main, 1) => "thumbnail IFD",
            (Ctx::Main, _) => "main",
            (Ctx::Exif, _) => "Exif",
            (Ctx::Gps, _) => "GPS",
            (Ctx::Interop, _) => "Interop",
        }
    }
}

/// What a tag is about; the groups match the switches of the tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Group {
    Gps,
    Datetime,
    Device,
    Author,
    Settings,
    Maker,
    Orientation,
    /// Needed to read the picture or the block itself; never removed by a group.
    Structural,
    Other,
}

pub(crate) fn group(ctx: Ctx, tag: u16) -> Group {
    match (ctx, tag) {
        (Ctx::Gps, _) => Group::Gps,
        (Ctx::Main, 0x0132)
        | (Ctx::Exif, 0x9003 | 0x9004 | 0x9010 | 0x9011 | 0x9012 | 0x9290 | 0x9291 | 0x9292) => {
            Group::Datetime
        }
        (Ctx::Main, 0x010F | 0x0110 | 0x0131 | 0x013C)
        | (Ctx::Exif, 0xA420 | 0xA431..=0xA435) => Group::Device,
        (Ctx::Main, 0x010E | 0x013B | 0x8298 | 0x9C9B..=0x9C9F)
        | (Ctx::Exif, 0x9286 | 0xA430) => Group::Author,
        (Ctx::Exif, 0x927C) => Group::Maker,
        (Ctx::Main, 0x0112) => Group::Orientation,
        (
            Ctx::Exif,
            0x829A | 0x829D | 0x8822 | 0x8824 | 0x8827 | 0x8828 | 0x8830..=0x8832
            | 0x9201..=0x920A | 0x9214 | 0xA20B..=0xA217 | 0xA300..=0xA302 | 0xA401..=0xA40C,
        ) => Group::Settings,
        (
            Ctx::Main,
            0x0100 | 0x0101 | 0x0102 | 0x0103 | 0x0106 | 0x0111 | 0x0115 | 0x0116 | 0x0117
            | 0x011C | 0x0201 | 0x0202,
        )
        | (Ctx::Exif, 0x9000 | 0x9101 | 0xA000 | 0xA001 | 0xA002 | 0xA003)
        | (Ctx::Interop, _) => Group::Structural,
        _ => Group::Other,
    }
}

/// Whether the tag is named by the user (`Software`, `0x0131`, ...).
fn named(sel: &Selection, ctx: Ctx, tag: u16) -> bool {
    if sel.tags.is_empty() {
        return false;
    }
    let name = Tag(ctx.to_exif(), tag).to_string().to_lowercase();
    let hex = format!("0x{tag:04x}");
    sel.tags.iter().any(|t| *t == name || *t == hex)
}

/// Name used in the lists of removed fields.
fn tag_label(ctx: Ctx, ifd: u16, tag: u16) -> String {
    let tag_name = Tag(ctx.to_exif(), tag).to_string();
    let shown = if tag_name.starts_with("Tag(") { format!("0x{tag:04X}") } else { tag_name };
    format!("{shown} ({})", ctx.label(ifd))
}

#[derive(Debug)]
pub(crate) struct Edit {
    /// The block after editing; `None` when nothing is left in it.
    pub tiff: Option<Vec<u8>>,
    pub removed: Vec<String>,
}

fn rd16(buf: &[u8], le: bool, p: usize) -> Option<u16> {
    let s = buf.get(p..p.checked_add(2)?)?;
    Some(if le { u16::from_le_bytes([s[0], s[1]]) } else { u16::from_be_bytes([s[0], s[1]]) })
}

fn rd32(buf: &[u8], le: bool, p: usize) -> Option<u32> {
    let s = buf.get(p..p.checked_add(4)?)?;
    let a = [s[0], s[1], s[2], s[3]];
    Some(if le { u32::from_le_bytes(a) } else { u32::from_be_bytes(a) })
}

fn type_size(kind: u16) -> usize {
    match kind {
        1 | 2 | 6 | 7 => 1,
        3 | 8 => 2,
        4 | 9 | 11 | 13 => 4,
        5 | 10 | 12 => 8,
        _ => 0,
    }
}

#[derive(Clone)]
struct Entry {
    raw: [u8; 12],
    tag: u16,
    kind: u16,
    count: u32,
    /// The four bytes of the value field read as a number (an offset when the value is long).
    value: u32,
    /// Where a value longer than four bytes lives.
    data: Option<(usize, usize)>,
}

struct Walk<'a> {
    buf: Vec<u8>,
    le: bool,
    sel: &'a Selection,
    removed: Vec<String>,
    /// Nothing before this offset may be cut off at the end.
    floor: usize,
    seen: Vec<usize>,
    has_next: bool,
    /// Something besides structural tags stays in the block.
    meaningful: bool,
}

impl Walk<'_> {
    fn read_ifd(&self, off: usize) -> Result<(Vec<Entry>, usize, u32), String> {
        let count = rd16(&self.buf, self.le, off).ok_or("truncated IFD")? as usize;
        let table_end = off + 2 + 12 * count + 4;
        if table_end > self.buf.len() {
            return Err("IFD runs past the end of the block".to_owned());
        }
        let mut entries = Vec::with_capacity(count);
        for i in 0..count {
            let p = off + 2 + 12 * i;
            let mut raw = [0u8; 12];
            raw.copy_from_slice(&self.buf[p..p + 12]);
            let tag = rd16(&self.buf, self.le, p).unwrap_or(0);
            let kind = rd16(&self.buf, self.le, p + 2).unwrap_or(0);
            let count = rd32(&self.buf, self.le, p + 4).unwrap_or(0);
            let value = rd32(&self.buf, self.le, p + 8).unwrap_or(0);
            let size = type_size(kind).saturating_mul(count as usize);
            let data = if size > 4 {
                let start = value as usize;
                match start.checked_add(size) {
                    Some(end) if end <= self.buf.len() => Some((start, end)),
                    _ => return Err("a value lies outside of the block".to_owned()),
                }
            } else {
                None
            };
            entries.push(Entry { raw, tag, kind, count, value, data });
        }
        let next = rd32(&self.buf, self.le, table_end - 4).unwrap_or(0);
        Ok((entries, table_end, next))
    }

    fn zero(&mut self, start: usize, end: usize) {
        if let Some(part) = self.buf.get_mut(start..end) {
            part.fill(0);
        }
    }

    fn decide(&self, ctx: Ctx, ifd: u16, tag: u16) -> bool {
        // The thumbnail location goes away only together with the whole thumbnail.
        if ifd == 1 && matches!(tag, 0x0201 | 0x0202) {
            return false;
        }
        if named(self.sel, ctx, tag) {
            return true;
        }
        let s = self.sel;
        match group(ctx, tag) {
            Group::Gps => s.gps,
            Group::Datetime => s.datetime,
            Group::Device => s.device,
            Group::Author => s.author,
            Group::Settings => s.settings,
            Group::Maker => s.makernote,
            Group::Orientation => s.orientation,
            Group::Other => s.exif_other,
            Group::Structural => false,
        }
    }

    /// Removes the thumbnail IFD together with the thumbnail picture.
    fn wipe_thumbnail(&mut self, off: usize) -> Result<(), String> {
        let (entries, table_end, _) = self.read_ifd(off)?;
        let mut pos = None;
        let mut len = None;
        for e in &entries {
            match e.tag {
                0x0201 => pos = Some(e.value as usize),
                0x0202 => len = Some(e.value as usize),
                _ => {}
            }
            if let Some((s, en)) = e.data {
                self.zero(s, en);
            }
        }
        if let (Some(p), Some(l)) = (pos, len)
            && let Some(end) = p.checked_add(l)
        {
            self.zero(p, end);
        }
        self.zero(off, table_end);
        Ok(())
    }

    /// Edits one IFD (and the IFDs it points to); returns how many entries stay.
    fn process_ifd(&mut self, off: usize, ctx: Ctx, ifd: u16, depth: u8) -> Result<usize, String> {
        if depth > 4 || self.seen.contains(&off) {
            return Err("IFDs point to each other".to_owned());
        }
        self.seen.push(off);
        let (entries, table_end, next) = self.read_ifd(off)?;
        let mut kept: Vec<Entry> = Vec::new();
        for e in entries {
            let child = match (ctx, e.tag) {
                (Ctx::Main, 0x8769) => Some(Ctx::Exif),
                (Ctx::Main, 0x8825) => Some(Ctx::Gps),
                (Ctx::Exif, 0xA005) => Some(Ctx::Interop),
                _ => None,
            };
            if let Some(child_ctx) = child
                && e.kind == 4
                && e.count == 1
            {
                let child_off = e.value as usize;
                let left = self.process_ifd(child_off, child_ctx, ifd, depth + 1)?;
                if left == 0 {
                    self.zero(child_off, child_off + 6);
                    continue;
                }
                kept.push(e);
                continue;
            }
            if self.decide(ctx, ifd, e.tag) {
                self.removed.push(tag_label(ctx, ifd, e.tag));
                if let Some((s, en)) = e.data {
                    self.zero(s, en);
                }
            } else {
                if let Some((_, en)) = e.data {
                    self.floor = self.floor.max(en);
                }
                if group(ctx, e.tag) != Group::Structural {
                    self.meaningful = true;
                }
                kept.push(e);
            }
        }

        let mut next_value = next;
        if ctx == Ctx::Main && ifd == 0 && next != 0 {
            let next_off = next as usize;
            if self.sel.thumbnail {
                self.wipe_thumbnail(next_off)?;
                self.removed.push("Thumbnail image (thumbnail IFD)".to_owned());
                next_value = 0;
            } else {
                let left = self.process_ifd(next_off, Ctx::Main, 1, depth + 1)?;
                if left == 0 {
                    self.zero(next_off, next_off + 6);
                    next_value = 0;
                }
            }
            self.has_next = next_value != 0;
        }
        if ifd == 1 {
            let at = kept.iter().find(|e| e.tag == 0x0201).map(|e| e.value as usize);
            let len = kept.iter().find(|e| e.tag == 0x0202).map(|e| e.value as usize);
            if let (Some(a), Some(l)) = (at, len) {
                self.floor = self.floor.max(a.saturating_add(l).min(self.buf.len()));
            }
        }

        // Rewrite the table: kept entries first, the rest is zeroed.
        let le = self.le;
        let count = kept.len() as u16;
        let mut p = off;
        let count_bytes = if le { count.to_le_bytes() } else { count.to_be_bytes() };
        self.buf[p..p + 2].copy_from_slice(&count_bytes);
        p += 2;
        for e in &kept {
            self.buf[p..p + 12].copy_from_slice(&e.raw);
            p += 12;
        }
        let next_bytes = if le { next_value.to_le_bytes() } else { next_value.to_be_bytes() };
        self.buf[p..p + 4].copy_from_slice(&next_bytes);
        p += 4;
        self.zero(p, table_end);
        self.floor = self.floor.max(p);
        Ok(kept.len())
    }
}

/// Removes what `sel` selects from an EXIF/TIFF block.
pub(crate) fn edit(tiff: &[u8], sel: &Selection) -> Result<Edit, String> {
    let le = match tiff.get(..4) {
        Some(b"II*\0") => true,
        Some(b"MM\0*") => false,
        _ => return Err("no TIFF header".to_owned()),
    };
    let ifd0 = rd32(tiff, le, 4).ok_or("truncated header")? as usize;
    let mut walk = Walk {
        buf: tiff.to_vec(),
        le,
        sel,
        removed: Vec::new(),
        floor: 8,
        seen: Vec::new(),
        has_next: false,
        meaningful: false,
    };
    let left = walk.process_ifd(ifd0, Ctx::Main, 0, 0)?;
    if (left == 0 || !walk.meaningful) && !walk.has_next {
        return Ok(Edit { tiff: None, removed: walk.removed });
    }
    let floor = walk.floor;
    while walk.buf.len() > floor && walk.buf.last() == Some(&0) {
        walk.buf.pop();
    }
    Ok(Edit { tiff: Some(walk.buf), removed: walk.removed })
}
