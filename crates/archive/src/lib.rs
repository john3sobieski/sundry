//! Writes in-memory files into one ZIP or 7z archive (RAR cannot be written by free software).

use std::collections::HashSet;
use std::io::{Cursor, Seek, Write};
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveFormat {
    Zip,
    SevenZ,
}

impl ArchiveFormat {
    pub fn extension(self) -> &'static str {
        match self {
            ArchiveFormat::Zip => "zip",
            ArchiveFormat::SevenZ => "7z",
        }
    }
}

#[derive(Debug)]
pub enum ArchiveError {
    /// The progress callback asked to stop.
    Cancelled,
    Failed(String),
}

impl std::fmt::Display for ArchiveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ArchiveError::Cancelled => f.write_str("cancelled"),
            ArchiveError::Failed(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for ArchiveError {}

fn failed(err: impl std::fmt::Display) -> ArchiveError {
    ArchiveError::Failed(err.to_string())
}

pub struct Entry<'a> {
    pub name: &'a str,
    pub data: &'a [u8],
}

/// Safe, unique archive names: only the last path component is kept and a
/// repeated name gets ` (2)`, ` (3)`... before its extension (case-insensitive).
pub fn unique_names<'a>(names: impl IntoIterator<Item = Option<&'a str>>) -> Vec<String> {
    let mut seen = HashSet::new();
    names
        .into_iter()
        .enumerate()
        .map(|(i, name)| {
            let base = name
                .map(|n| n.rsplit(['/', '\\']).next().unwrap_or(n))
                .filter(|n| !n.is_empty() && *n != "." && *n != "..")
                .map_or_else(|| format!("file-{}", i + 1), str::to_owned);
            let path = Path::new(&base);
            let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or(&base).to_owned();
            let ext = path.extension().and_then(|e| e.to_str()).map(|e| format!(".{e}"));
            let ext = ext.unwrap_or_default();
            let mut candidate = base.clone();
            let mut n = 2;
            while !seen.insert(candidate.to_lowercase()) {
                candidate = format!("{stem} ({n}){ext}");
                n += 1;
            }
            candidate
        })
        .collect()
}

/// Formats that are already compressed gain nothing from deflate.
fn is_compressed(name: &str) -> bool {
    let ext = name.rsplit('.').next().unwrap_or("").to_lowercase();
    matches!(ext.as_str(), "jpg" | "jpeg" | "png" | "webp" | "avif" | "gif")
}

/// Writes every entry into `out` and returns it. `progress(done)` is called after
/// each entry; returning `false` cancels with `ArchiveError::Cancelled`.
pub fn write_archive<W: Write + Seek>(
    format: ArchiveFormat,
    entries: &[Entry<'_>],
    out: W,
    mut progress: impl FnMut(usize) -> bool,
) -> Result<W, ArchiveError> {
    match format {
        ArchiveFormat::Zip => {
            use zip::CompressionMethod::{Deflated, Stored};
            use zip::write::SimpleFileOptions;
            let mut writer = zip::ZipWriter::new(out);
            for (i, entry) in entries.iter().enumerate() {
                let method = if is_compressed(entry.name) { Stored } else { Deflated };
                writer
                    .start_file(entry.name, SimpleFileOptions::default().compression_method(method))
                    .map_err(failed)?;
                writer.write_all(entry.data).map_err(failed)?;
                if !progress(i + 1) {
                    return Err(ArchiveError::Cancelled);
                }
            }
            writer.finish().map_err(failed)
        }
        ArchiveFormat::SevenZ => {
            let mut writer = sevenz_rust2::ArchiveWriter::new(out).map_err(failed)?;
            for (i, entry) in entries.iter().enumerate() {
                let item = sevenz_rust2::ArchiveEntry::new_file(entry.name);
                writer.push_archive_entry(item, Some(Cursor::new(entry.data))).map_err(failed)?;
                if !progress(i + 1) {
                    return Err(ArchiveError::Cancelled);
                }
            }
            writer.finish().map_err(failed)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_made_safe_and_unique() {
        let names = unique_names([
            Some("a.png"),
            Some("dir/a.png"),
            Some("..\\..\\A.PNG"),
            None,
            Some(".."),
            Some("b"),
            Some("b"),
        ]);
        assert_eq!(names, ["a.png", "a (2).png", "A (3).PNG", "file-4", "file-5", "b", "b (2)"]);
    }

    #[test]
    fn zip_contains_every_file() {
        let entries = [
            Entry { name: "a.png", data: &[1, 2, 3] },
            Entry { name: "\u{444}\u{43e}\u{442}\u{43e}.bmp", data: &[9; 1000] },
        ];
        let mut steps = 0;
        let out = write_archive(ArchiveFormat::Zip, &entries, Cursor::new(Vec::new()), |n| {
            steps = n;
            true
        })
        .unwrap();
        assert_eq!(steps, 2);
        let bytes = out.into_inner();
        assert_eq!(&bytes[..2], b"PK");
        let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
        assert_eq!(zip.len(), 2);
        let mut data = Vec::new();
        std::io::Read::read_to_end(&mut zip.by_name("a.png").unwrap(), &mut data).unwrap();
        assert_eq!(data, [1, 2, 3]);
    }

    #[test]
    fn seven_z_has_its_signature() {
        let entries = [Entry { name: "a.png", data: &[1, 2, 3] }];
        let out = write_archive(ArchiveFormat::SevenZ, &entries, Cursor::new(Vec::new()), |_| true)
            .unwrap();
        assert_eq!(&out.into_inner()[..6], &[b'7', b'z', 0xBC, 0xAF, 0x27, 0x1C]);
    }

    #[test]
    fn progress_can_cancel() {
        let entries = [Entry { name: "a", data: b"x" }, Entry { name: "b", data: b"y" }];
        let result = write_archive(ArchiveFormat::Zip, &entries, Cursor::new(Vec::new()), |_| false);
        assert!(matches!(result, Err(ArchiveError::Cancelled)));
    }
}
