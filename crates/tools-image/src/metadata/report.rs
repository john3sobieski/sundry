//! The text report: what a file carries, with a list of privacy-relevant findings on top.

use std::fmt::Write as _;

use ::exif::{Exif, Field, In, Reader, Tag, Value};

use super::containers::{Doc, Fmt, Kind, Piece, png_text};
use super::tiffedit::{self, Ctx, Group};
use super::extra;

fn section(out: &mut String, title: &str) {
    let _ = write!(out, "\n== {title} ==\n");
}

fn line(out: &mut String, key: &str, value: &str) {
    let _ = writeln!(out, "{key:<30} {value}");
}

fn clip(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_owned()
    } else {
        let mut t: String = s.chars().take(max).collect();
        t.push_str("...");
        t
    }
}

fn one_line(s: &str) -> String {
    s.replace("\r\n", " / ").replace(['\n', '\r'], " / ")
}

fn fmt_name(doc: &Doc, b: &[u8]) -> &'static str {
    match doc.fmt {
        Fmt::Jpeg => "JPEG",
        Fmt::Png => "PNG",
        Fmt::WebP => "WebP",
        Fmt::Gif => "GIF",
        Fmt::Tiff => "TIFF",
        Fmt::Other => crate::codec::format_name(crate::codec::probe(b).and_then(|p| p.2)),
    }
}

fn read_exif(b: &[u8], doc: &Doc) -> Option<Exif> {
    let tiff: &[u8] = if doc.fmt == Fmt::Tiff {
        b
    } else {
        let p = doc.pieces.iter().find(|p| p.kind == Kind::Exif)?;
        &b[p.data..p.data_end]
    };
    let mut reader = Reader::new();
    reader.continue_on_error(true);
    match reader.read_raw(tiff.to_vec()) {
        Ok(exif) => Some(exif),
        Err(err) => err.distill_partial_result(|_| {}).ok(),
    }
}

fn gps_position(exif: &Exif) -> Option<(f64, f64)> {
    let coord = |tag: Tag, ref_tag: Tag| -> Option<f64> {
        let f = exif.get_field(tag, In::PRIMARY)?;
        let Value::Rational(v) = &f.value else { return None };
        if v.len() < 3 {
            return None;
        }
        let deg = v[0].to_f64() + v[1].to_f64() / 60.0 + v[2].to_f64() / 3600.0;
        let r = exif.get_field(ref_tag, In::PRIMARY)?;
        let Value::Ascii(a) = &r.value else { return None };
        let c = *a.first()?.first()?;
        Some(if matches!(c, b'S' | b'W') { -deg } else { deg })
    };
    Some((
        coord(Tag::GPSLatitude, Tag::GPSLatitudeRef)?,
        coord(Tag::GPSLongitude, Tag::GPSLongitudeRef)?,
    ))
}

fn field_value(exif: &Exif, f: &Field) -> String {
    match &f.value {
        Value::Undefined(v, _) if f.tag == Tag::MakerNote => {
            let preview: String = v
                .iter()
                .take_while(|c| (0x20..0x7F).contains(*c))
                .take(24)
                .map(|&c| c as char)
                .collect();
            if preview.len() >= 3 {
                format!("{} bytes of vendor data (starts with \"{preview}\")", v.len())
            } else {
                format!("{} bytes of vendor data", v.len())
            }
        }
        Value::Undefined(v, _) if v.len() > 64 && f.tag != Tag::UserComment => {
            format!("{} bytes of binary data", v.len())
        }
        _ => one_line(&clip(&f.display_value().with_unit(exif).to_string(), 300)),
    }
}

fn exif_title(ifd: u16, ctx: Ctx) -> String {
    match (ifd, ctx) {
        (0, Ctx::Main) => "EXIF: main image (IFD0)".to_owned(),
        (0, Ctx::Exif) => "EXIF: Exif IFD (shooting data)".to_owned(),
        (0, Ctx::Gps) => "EXIF: GPS".to_owned(),
        (0, Ctx::Interop) => "EXIF: interoperability".to_owned(),
        (1, _) => "EXIF: thumbnail (IFD1)".to_owned(),
        (n, _) => format!("EXIF: IFD{n}"),
    }
}

/// Prints the EXIF fields and collects privacy findings.
fn exif_part(out: &mut String, exif: &Exif, findings: &mut Vec<String>) {
    let mut fields: Vec<&Field> = exif.fields().collect();
    fields.sort_by_key(|f| (f.ifd_num, f.tag.context(), f.tag.number()));

    let mut current: Option<(u16, Ctx)> = None;
    let (mut device, mut author, mut dates) = (Vec::new(), Vec::new(), Vec::new());
    let (mut gps, mut maker, mut serial) = (false, false, false);
    for f in &fields {
        let ctx = Ctx::from_exif(f.tag.context());
        let ifd = f.ifd_num.index();
        if current != Some((ifd, ctx)) {
            section(out, &exif_title(ifd, ctx));
            current = Some((ifd, ctx));
        }
        let name = f.tag.to_string();
        let shown = if name.starts_with("Tag(") { "(unknown tag)".to_owned() } else { name.clone() };
        let key = format!("{shown} [0x{:04X}]", f.tag.number());
        line(out, &key, &field_value(exif, f));
        if ifd == 1 {
            continue;
        }
        match tiffedit::group(ctx, f.tag.number()) {
            Group::Gps => gps = true,
            Group::Datetime => dates.push(shown),
            Group::Device => {
                serial |= matches!(f.tag, Tag::BodySerialNumber | Tag::LensSerialNumber);
                device.push(shown);
            }
            Group::Author => author.push(shown),
            Group::Maker => maker = true,
            _ => {}
        }
    }

    if gps {
        match gps_position(exif) {
            Some((lat, lon)) => findings.push(format!(
                "GPS location: {lat:.6}, {lon:.6} (https://www.openstreetmap.org/?mlat={lat:.6}&mlon={lon:.6}#map=16/{lat:.5}/{lon:.5})"
            )),
            None => findings.push("GPS tags are present (no complete position)".to_owned()),
        }
    }
    if !device.is_empty() {
        let extra = if serial { " - includes serial numbers" } else { "" };
        findings.push(format!("Camera/lens/software info: {}{extra}", device.join(", ")));
    }
    if !author.is_empty() {
        findings.push(format!("Author, owner or description fields: {}", author.join(", ")));
    }
    if !dates.is_empty() {
        findings.push(format!("Dates and times: {}", dates.join(", ")));
    }
    if maker {
        findings.push("MakerNote: vendor-specific data (can hold serial numbers and settings)".to_owned());
    }
    if let Some(len) = exif.get_field(Tag::JPEGInterchangeFormatLength, In::THUMBNAIL).and_then(|f| f.value.get_uint(0)) {
        findings.push(format!(
            "Embedded thumbnail ({len} bytes): it can show the picture as it was before cropping or editing"
        ));
    }
}

fn piece_text(b: &[u8], doc: &Doc, p: &Piece) -> (String, String) {
    let data = &b[p.data..p.data_end];
    if doc.fmt == Fmt::Png {
        png_text(&b[p.start + 4..p.start + 8], data)
    } else {
        ("Comment".to_owned(), String::from_utf8_lossy(data).into_owned())
    }
}

fn png_time(data: &[u8]) -> String {
    if data.len() < 7 {
        return "unreadable".to_owned();
    }
    let year = u16::from_be_bytes([data[0], data[1]]);
    format!("{year:04}-{:02}-{:02} {:02}:{:02}:{:02} UTC", data[2], data[3], data[4], data[5], data[6])
}

/// The report for one file; `removed` is set when the file is the result of a removal.
pub(crate) fn render(name: &str, b: &[u8], doc: &Doc, removed: Option<&[String]>) -> String {
    let mut body = String::new();
    let mut findings: Vec<String> = Vec::new();

    section(&mut body, "File");
    line(&mut body, "Name", name);
    line(&mut body, "Format", fmt_name(doc, b));
    line(&mut body, "Size", &format!("{} bytes", b.len()));
    if let Some((w, h, _)) = crate::codec::probe(b) {
        line(&mut body, "Dimensions", &format!("{w} x {h} px"));
    }

    if let Some(exif) = read_exif(b, doc) {
        exif_part(&mut body, &exif, &mut findings);
    } else if doc.pieces.iter().any(|p| p.kind == Kind::Exif) {
        section(&mut body, "EXIF");
        line(&mut body, "Status", "the EXIF block is present but cannot be read");
        findings.push("An unreadable EXIF block is present".to_owned());
    }

    for p in doc.pieces.iter().filter(|p| p.kind == Kind::Xmp) {
        let text = if doc.fmt == Fmt::Png {
            png_text(b"iTXt", &b[p.data..p.data_end]).1
        } else {
            String::from_utf8_lossy(&b[p.data..p.data_end]).into_owned()
        };
        let props = extra::xmp_props(&text);
        section(&mut body, &format!("XMP ({})", p.label));
        for (k, v) in &props {
            line(&mut body, k, &one_line(&clip(v, 300)));
        }
        let _ = writeln!(body, "-- raw packet ({} bytes, first 6000 characters) --", text.len());
        let _ = writeln!(body, "{}", clip(text.trim(), 6000));
        findings.push(format!("XMP packet with {} properties ({})", props.len(), p.label));
    }

    for p in doc.pieces.iter().filter(|p| p.kind == Kind::Iptc) {
        let items = extra::iptc(&b[p.data..p.data_end]);
        section(&mut body, "IPTC / Photoshop resources");
        for (k, v) in &items {
            line(&mut body, k, &one_line(&clip(v, 300)));
        }
        findings.push("IPTC / Photoshop block (captions, keywords, credits, location)".to_owned());
    }

    let icc: Vec<&Piece> = doc.pieces.iter().filter(|p| p.kind == Kind::Icc).collect();
    if !icc.is_empty() {
        section(&mut body, "ICC colour profile");
        if doc.fmt == Fmt::Png {
            let data = &b[icc[0].data..icc[0].data_end];
            let nul = data.iter().position(|&c| c == 0).unwrap_or(0);
            line(&mut body, "Profile name", &String::from_utf8_lossy(&data[..nul]));
            line(&mut body, "Compressed size", &format!("{} bytes", data.len()));
        } else {
            let profile: Vec<u8> = icc.iter().flat_map(|p| b[p.data..p.data_end].to_vec()).collect();
            for (k, v) in extra::icc(&profile) {
                line(&mut body, &k, &v);
            }
        }
    }

    let texts: Vec<&Piece> =
        doc.pieces.iter().filter(|p| matches!(p.kind, Kind::Comment | Kind::Time)).collect();
    if !texts.is_empty() {
        section(&mut body, "Comments and text chunks");
        let mut keys = Vec::new();
        for p in texts {
            if p.kind == Kind::Time {
                line(&mut body, "tIME (last modified)", &png_time(&b[p.data..p.data_end]));
                findings.push("PNG tIME chunk: modification date".to_owned());
            } else {
                let (key, text) = piece_text(b, doc, p);
                line(&mut body, &key, &one_line(&clip(&text, 600)));
                keys.push(key);
            }
        }
        if !keys.is_empty() {
            findings.push(format!("Text comments: {}", keys.join(", ")));
        }
    }

    section(&mut body, "File structure");
    for p in &doc.pieces {
        let mark = if p.kind == Kind::Keep { "" } else { "  [metadata]" };
        line(&mut body, &p.label, &format!("{} bytes{mark}", p.end - p.start));
    }
    if doc.trailer > 0 {
        let note = format!("{} bytes after the end of the image data (not touched by this tool)", doc.trailer);
        line(&mut body, "Trailing data", &note);
        findings.push(note);
    }
    if doc.pieces.iter().any(|p| p.label.contains("MPF")) {
        findings.push("MPF (multi-picture) segment: the file may hold extra pictures (not touched by this tool)".to_owned());
    }
    if matches!(doc.fmt, Fmt::Other) && doc.pieces.len() == 1 {
        let _ = writeln!(body, "\nThis format has no metadata blocks that Sundry can read.");
    }

    let mut out = String::new();
    if let Some(removed) = removed {
        section(&mut out, "Removed");
        if removed.is_empty() {
            out.push_str("Nothing was removed: no selected metadata was found.\n");
        } else {
            for item in removed {
                let _ = writeln!(out, "- {item}");
            }
        }
        out.push_str("\nEverything below is what the resulting file still contains.\n");
    }
    section(&mut out, if removed.is_some() { "Privacy findings (remaining)" } else { "Privacy findings" });
    if findings.is_empty() {
        out.push_str("Nothing sensitive found.\n");
    } else {
        for f in &findings {
            let _ = writeln!(out, "! {f}");
        }
    }
    out.push_str(&body);
    out.trim_start_matches('\n').to_owned()
}
