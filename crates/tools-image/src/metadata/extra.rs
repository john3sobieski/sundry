//! Readers for the smaller metadata blocks: XMP, IPTC (Photoshop resources), ICC.

fn decode_entities(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

/// Properties of an XMP packet as `(name, value)`. A light scanner, not a full XML parser:
/// it reads attributes of `rdf:Description`, simple elements and `rdf:li` lists.
pub(crate) fn xmp_props(xml: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    let mut list: Option<usize> = None;
    for chunk in xml.split('<').skip(1) {
        let Some((tag, rest)) = chunk.split_once('>') else { break };
        if tag.starts_with('?') || tag.starts_with('!') {
            continue;
        }
        if tag.starts_with('/') {
            let name = &tag[1..];
            if !name.starts_with("rdf:") {
                list = None;
            }
            continue;
        }
        let name = tag.split_whitespace().next().unwrap_or("");
        let text = decode_entities(rest.trim());
        if name == "rdf:li" {
            if let Some(i) = list {
                if !text.is_empty() {
                    if !out[i].1.is_empty() {
                        out[i].1.push_str(", ");
                    }
                    out[i].1.push_str(&text);
                }
            }
            continue;
        }
        if name == "rdf:Description" {
            let mut s = &tag[name.len()..];
            while let Some(eq) = s.find("=\"") {
                let key = s[..eq].split_whitespace().last().unwrap_or("");
                let after = &s[eq + 2..];
                let Some(end) = after.find('"') else { break };
                if !key.starts_with("xmlns") && key != "rdf:about" && !key.is_empty() {
                    out.push((key.to_owned(), decode_entities(&after[..end])));
                }
                s = &after[end + 1..];
            }
            continue;
        }
        if name.starts_with("rdf:") || name.starts_with("x:") || name.is_empty() {
            continue;
        }
        out.push((name.to_owned(), text.clone()));
        list = if text.is_empty() { Some(out.len() - 1) } else { None };
    }
    out.retain(|(_, v)| !v.is_empty());
    out
}

fn iptc_name(record: u8, dataset: u8) -> String {
    if record == 2 {
        let name = match dataset {
            5 => "Object name",
            15 => "Category",
            20 => "Supplemental category",
            25 => "Keywords",
            40 => "Special instructions",
            55 => "Date created",
            60 => "Time created",
            80 => "By-line (author)",
            85 => "By-line title",
            90 => "City",
            92 => "Sub-location",
            95 => "Province/State",
            101 => "Country",
            103 => "Original transmission reference",
            105 => "Headline",
            110 => "Credit",
            115 => "Source",
            116 => "Copyright notice",
            120 => "Caption/abstract",
            122 => "Writer/editor",
            _ => "",
        };
        if !name.is_empty() {
            return name.to_owned();
        }
    }
    format!("IPTC {record}:{dataset}")
}

/// Datasets of the IPTC-IIM block.
fn iptc_datasets(d: &[u8], out: &mut Vec<(String, String)>) {
    let mut q = 0;
    while q + 5 <= d.len() && d[q] == 0x1C {
        let len = usize::from(u16::from_be_bytes([d[q + 3], d[q + 4]]));
        if len & 0x8000 != 0 || q + 5 + len > d.len() {
            break;
        }
        let value = String::from_utf8_lossy(&d[q + 5..q + 5 + len]).into_owned();
        out.push((iptc_name(d[q + 1], d[q + 2]), value));
        q += 5 + len;
    }
}

/// Contents of the Photoshop resources block (after `Photoshop 3.0\0`).
pub(crate) fn iptc(irb: &[u8]) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut p = 0;
    while p + 12 <= irb.len() && &irb[p..p + 4] == b"8BIM" {
        let id = u16::from_be_bytes([irb[p + 4], irb[p + 5]]);
        let name_len = usize::from(irb[p + 6]);
        let size_at = p + 6 + ((1 + name_len + 1) & !1);
        if size_at + 4 > irb.len() {
            break;
        }
        let size = u32::from_be_bytes([irb[size_at], irb[size_at + 1], irb[size_at + 2], irb[size_at + 3]])
            as usize;
        let start = size_at + 4;
        let Some(end) = start.checked_add(size).filter(|e| *e <= irb.len()) else { break };
        if id == 0x0404 {
            iptc_datasets(&irb[start..end], &mut out);
        } else {
            out.push((format!("Photoshop resource 0x{id:04X}"), format!("{size} bytes")));
        }
        p = end + (size & 1);
    }
    out
}

fn four(p: &[u8], at: usize) -> String {
    String::from_utf8_lossy(&p[at..at + 4]).trim_matches(|c: char| c == '\0' || c == ' ').to_owned()
}

fn be32(p: &[u8], at: usize) -> Option<usize> {
    let s = p.get(at..at + 4)?;
    Some(u32::from_be_bytes([s[0], s[1], s[2], s[3]]) as usize)
}

/// Text of the profile description tag (`desc` of ICC v2 or `mluc` of v4).
fn icc_description(p: &[u8]) -> Option<String> {
    let count = be32(p, 128)?.min(200);
    for i in 0..count {
        let e = 132 + 12 * i;
        if p.get(e..e + 4)? != b"desc" {
            continue;
        }
        let off = be32(p, e + 4)?;
        let size = be32(p, e + 8)?;
        let data = p.get(off..off.checked_add(size)?)?;
        if data.starts_with(b"desc") {
            let n = be32(data, 8)?.saturating_sub(1);
            return Some(String::from_utf8_lossy(data.get(12..12 + n)?).into_owned());
        }
        if data.starts_with(b"mluc") {
            let len = be32(data, 20)?;
            let at = be32(data, 24)?;
            let raw = data.get(at..at.checked_add(len)?)?;
            let units = raw.chunks_exact(2).map(|c| u16::from_be_bytes([c[0], c[1]]));
            return Some(char::decode_utf16(units).map(|r| r.unwrap_or('?')).collect());
        }
    }
    None
}

/// Header fields and name of an ICC profile.
pub(crate) fn icc(p: &[u8]) -> Vec<(String, String)> {
    if p.len() < 132 || &p[36..40] != b"acsp" {
        return vec![("ICC profile".to_owned(), format!("{} bytes, unreadable header", p.len()))];
    }
    let mut out = vec![
        ("Size".to_owned(), format!("{} bytes", p.len())),
        ("Version".to_owned(), format!("{}.{}", p[8], p[9] >> 4)),
        ("Class".to_owned(), four(p, 12)),
        ("Color space".to_owned(), four(p, 16)),
        ("Connection space".to_owned(), four(p, 20)),
    ];
    let year = u16::from_be_bytes([p[24], p[25]]);
    let part = |i: usize| u16::from_be_bytes([p[26 + 2 * i], p[27 + 2 * i]]);
    out.push((
        "Created".to_owned(),
        format!("{year:04}-{:02}-{:02} {:02}:{:02}:{:02}", part(0), part(1), part(2), part(3), part(4)),
    ));
    for (label, at) in [("Platform", 40), ("Manufacturer", 48), ("Model", 52)] {
        let text = four(p, at);
        if !text.is_empty() {
            out.push((label.to_owned(), text));
        }
    }
    if let Some(desc) = icc_description(p) {
        out.insert(0, ("Description".to_owned(), desc));
    }
    out
}
