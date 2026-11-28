//! `image.metadata`: shows everything a picture file carries (EXIF, XMP, IPTC, ICC,
//! comments) and removes the selected parts without touching the picture data.

mod containers;
mod extra;
mod report;
mod tiffedit;

use std::path::Path;

use sundry_core::{
    Category, Data, Error, Input, InputKind, Item, Job, Media, Output, ParamSpec, Params, Result,
    Tool, ToolInfo,
};

pub struct Metadata;

/// What to remove. The switches are groups of tags; `tags` names single EXIF tags.
#[derive(Debug, Clone, Default)]
pub(crate) struct Selection {
    pub gps: bool,
    pub datetime: bool,
    pub device: bool,
    pub author: bool,
    pub settings: bool,
    pub thumbnail: bool,
    pub makernote: bool,
    pub exif_other: bool,
    pub xmp: bool,
    pub iptc: bool,
    pub comments: bool,
    pub icc: bool,
    pub orientation: bool,
    /// Lower-case tag names (`software`) or hex numbers (`0x0131`).
    pub tags: Vec<String>,
}

impl Selection {
    /// Everything that can identify a person, a place or a device.
    pub(crate) fn privacy() -> Self {
        Self {
            gps: true,
            datetime: true,
            device: true,
            author: true,
            thumbnail: true,
            makernote: true,
            xmp: true,
            iptc: true,
            comments: true,
            ..Self::default()
        }
    }

    /// Everything except what changes how the picture looks (colour profile, rotation).
    pub(crate) fn all() -> Self {
        Self { settings: true, exif_other: true, ..Self::privacy() }
    }

    pub(crate) fn touches_exif(&self) -> bool {
        self.gps
            || self.datetime
            || self.device
            || self.author
            || self.settings
            || self.thumbnail
            || self.makernote
            || self.exif_other
            || self.orientation
            || !self.tags.is_empty()
    }
}

enum Mode {
    View,
    Remove(Selection),
}

struct Settings {
    mode: Mode,
}

fn unknown(name: &str) -> Error {
    Error::InvalidParam { name: name.to_owned(), reason: "unknown".to_owned() }
}

fn parse_tags(text: &str) -> Vec<String> {
    text.split([',', ';', '\n'])
        .map(|t| t.trim().to_lowercase())
        .filter(|t| !t.is_empty())
        .collect()
}

fn base_name(name: Option<&str>) -> String {
    name.and_then(|n| Path::new(n).file_name())
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "image".to_owned())
}

impl Settings {
    fn from_params(params: &Params) -> Result<Self> {
        let mode = match params.get_text("mode")? {
            "view" => return Ok(Self { mode: Mode::View }),
            "remove" => params.get_text("preset")?,
            _ => return Err(unknown("mode")),
        };
        let mut sel = match mode {
            "privacy" => Selection::privacy(),
            "all" => Selection::all(),
            "custom" => Selection {
                gps: params.get_bool("gps")?,
                datetime: params.get_bool("datetime")?,
                device: params.get_bool("device")?,
                author: params.get_bool("author")?,
                settings: params.get_bool("settings")?,
                thumbnail: params.get_bool("thumbnail")?,
                makernote: params.get_bool("makernote")?,
                exif_other: params.get_bool("exif_other")?,
                xmp: params.get_bool("xmp")?,
                iptc: params.get_bool("iptc")?,
                comments: params.get_bool("comments")?,
                icc: params.get_bool("icc")?,
                orientation: params.get_bool("orientation")?,
                tags: Vec::new(),
            },
            _ => return Err(unknown("preset")),
        };
        sel.tags = parse_tags(params.get_text("tags")?);
        Ok(Self { mode: Mode::Remove(sel) })
    }

    /// One report item (view) or the cleaned file plus the report about it (remove).
    fn process(&self, item: &Item) -> Result<Vec<Item>> {
        let Data::Bytes(bytes) = &item.data else {
            return Err(Error::InvalidInput("expected an image file".to_owned()));
        };
        let base = base_name(item.name.as_deref());
        let report_name = format!("{base}.metadata.txt");
        let doc = containers::parse(bytes)?;
        match &self.mode {
            Mode::View => {
                let text = report::render(&base, bytes, &doc, None);
                Ok(vec![Item::text(text).named(report_name)])
            }
            Mode::Remove(sel) => {
                let out = containers::strip(bytes, &doc, sel)?;
                let after = containers::parse(&out.bytes)?;
                let text = report::render(&base, &out.bytes, &after, Some(&out.removed));
                Ok(vec![Item::bytes(out.bytes).named(base), Item::text(text).named(report_name)])
            }
        }
    }
}

impl Tool for Metadata {
    fn info(&self) -> ToolInfo {
        ToolInfo::new(
            "image.metadata",
            Category::Image,
            "Image metadata",
            "Shows everything a picture file carries (EXIF, GPS, XMP, IPTC, colour profile, \
             comments) in detail and removes only the selected parts. The picture itself is \
             not re-encoded, so nothing is lost. Removal works for JPEG, PNG and WebP.",
        )
        .with_input(InputKind::Files)
        .with_media(Media::ImageMetadata)
        .per_file()
    }

    fn params(&self) -> Vec<ParamSpec> {
        let custom = |spec: ParamSpec| spec.show_when("mode", &["remove"]).show_when("preset", &["custom"]);
        vec![
            ParamSpec::choice("mode", &["view", "remove"], "view"),
            ParamSpec::choice("preset", &["privacy", "all", "custom"], "privacy")
                .show_when("mode", &["remove"]),
            custom(ParamSpec::boolean("gps", true)),
            custom(ParamSpec::boolean("datetime", true)),
            custom(ParamSpec::boolean("device", true)),
            custom(ParamSpec::boolean("author", true)),
            custom(ParamSpec::boolean("settings", false)),
            custom(ParamSpec::boolean("thumbnail", true)),
            custom(ParamSpec::boolean("makernote", true)),
            custom(ParamSpec::boolean("exif_other", false)),
            custom(ParamSpec::boolean("xmp", true)),
            custom(ParamSpec::boolean("iptc", true)),
            custom(ParamSpec::boolean("comments", true)),
            custom(ParamSpec::boolean("icc", false)),
            custom(ParamSpec::boolean("orientation", false)),
            custom(ParamSpec::text("tags", "")),
        ]
    }

    fn run(&self, input: &Input, params: &Params, job: &Job) -> Result<Output> {
        let settings = Settings::from_params(params)?;
        let total = input.items.len();
        let mut items = Vec::new();
        for (i, item) in input.items.iter().enumerate() {
            job.check_cancelled()?;
            let done = settings.process(item).map_err(|err| match (&item.name, err) {
                (_, Error::Cancelled) => Error::Cancelled,
                (Some(name), err) => Error::Processing(format!("{name}: {err}")),
                (None, err) => err,
            })?;
            items.extend(done);
            if total > 1 {
                job.set_progress((i + 1) as f32 / total as f32);
            }
        }
        Ok(Output { items })
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use ::exif::experimental::Writer;
    use ::exif::{Field, In, Rational, Reader, Tag, Value};
    use image::{DynamicImage, ImageFormat, RgbaImage};
    use sundry_core::registry::Registry;

    use super::*;

    fn ascii(s: &str) -> Value {
        Value::Ascii(vec![s.as_bytes().to_vec()])
    }

    fn rat(n: u32) -> Rational {
        Rational { num: n, denom: 1 }
    }

    /// EXIF block with the camera, the dates, a GPS position, the orientation and a thumbnail.
    fn sample_tiff(le: bool, thumbnail: bool) -> Vec<u8> {
        let fields = vec![
            Field { tag: Tag::Make, ifd_num: In::PRIMARY, value: ascii("TestCam") },
            Field { tag: Tag::Model, ifd_num: In::PRIMARY, value: ascii("T-1000") },
            Field { tag: Tag::Orientation, ifd_num: In::PRIMARY, value: Value::Short(vec![6]) },
            Field { tag: Tag::Artist, ifd_num: In::PRIMARY, value: ascii("Somebody Secret") },
            Field { tag: Tag::DateTimeOriginal, ifd_num: In::PRIMARY, value: ascii("2020:01:02 03:04:05") },
            Field { tag: Tag::ExposureTime, ifd_num: In::PRIMARY, value: Value::Rational(vec![Rational { num: 1, denom: 250 }]) },
            Field { tag: Tag::PixelXDimension, ifd_num: In::PRIMARY, value: Value::Long(vec![8]) },
            Field { tag: Tag::GPSLatitudeRef, ifd_num: In::PRIMARY, value: ascii("N") },
            Field { tag: Tag::GPSLatitude, ifd_num: In::PRIMARY, value: Value::Rational(vec![rat(51), rat(2), rat(3)]) },
            Field { tag: Tag::GPSLongitudeRef, ifd_num: In::PRIMARY, value: ascii("E") },
            Field { tag: Tag::GPSLongitude, ifd_num: In::PRIMARY, value: Value::Rational(vec![rat(7), rat(1), rat(1)]) },
            Field { tag: Tag::Compression, ifd_num: In::THUMBNAIL, value: Value::Short(vec![6]) },
        ];
        let thumb = vec![0xFF, 0xD8, 0xFF, 0xD9, 7, 7, 7, 7];
        let mut writer = Writer::new();
        for f in &fields {
            if thumbnail || f.ifd_num != In::THUMBNAIL {
                writer.push_field(f);
            }
        }
        if thumbnail {
            writer.set_jpeg(&thumb, In::THUMBNAIL);
        }
        let mut cur = Cursor::new(Vec::new());
        writer.write(&mut cur, le).unwrap();
        cur.into_inner()
    }

    fn picture(format: ImageFormat) -> Vec<u8> {
        let img = DynamicImage::ImageRgba8(RgbaImage::from_fn(8, 8, |x, y| image::Rgba([x as u8 * 30, y as u8 * 30, 9, 255])));
        let mut cur = Cursor::new(Vec::new());
        match format {
            ImageFormat::Jpeg => img.to_rgb8().write_to(&mut cur, format).unwrap(),
            _ => img.write_to(&mut cur, format).unwrap(),
        }
        cur.into_inner()
    }

    fn jpeg_with(tiff: &[u8], extra: &[(u8, Vec<u8>)]) -> Vec<u8> {
        let base = picture(ImageFormat::Jpeg);
        let mut out = base[..2].to_vec();
        let mut app = |marker: u8, body: &[u8]| {
            out.extend_from_slice(&[0xFF, marker]);
            out.extend_from_slice(&((body.len() + 2) as u16).to_be_bytes());
            out.extend_from_slice(body);
        };
        let mut exif_body = b"Exif\0\0".to_vec();
        exif_body.extend_from_slice(tiff);
        app(0xE1, &exif_body);
        for (marker, body) in extra {
            app(*marker, body);
        }
        out.extend_from_slice(&base[2..]);
        out
    }

    fn exif_of_jpeg(bytes: &[u8]) -> Option<::exif::Exif> {
        Reader::new().read_from_container(&mut Cursor::new(bytes)).ok()
    }

    fn remove(bytes: &[u8], sel: Selection) -> (Vec<u8>, Vec<String>) {
        let doc = containers::parse(bytes).unwrap();
        let out = containers::strip(bytes, &doc, &sel).unwrap();
        (out.bytes, out.removed)
    }

    fn still_decodes(bytes: &[u8]) {
        let img = image::load_from_memory(bytes).expect("the cleaned file must still decode");
        assert_eq!((img.width(), img.height()), (8, 8));
    }

    #[test]
    fn gps_only_keeps_the_rest_of_exif() {
        for le in [true, false] {
            let file = jpeg_with(&sample_tiff(le, true), &[]);
            let (out, removed) = remove(&file, Selection { gps: true, ..Selection::default() });
            let exif = exif_of_jpeg(&out).expect("EXIF stays");
            assert!(exif.get_field(Tag::GPSLatitude, In::PRIMARY).is_none());
            assert!(exif.get_field(Tag::GPSLongitude, In::PRIMARY).is_none());
            assert!(exif.get_field(Tag::Make, In::PRIMARY).is_some());
            assert!(exif.get_field(Tag::Orientation, In::PRIMARY).is_some());
            assert!(exif.get_field(Tag::DateTimeOriginal, In::PRIMARY).is_some());
            assert!(exif.get_field(Tag::Compression, In::THUMBNAIL).is_some());
            assert_eq!(removed.len(), 4);
            still_decodes(&out);
        }
    }

    #[test]
    fn removed_values_do_not_stay_in_the_file() {
        let file = jpeg_with(&sample_tiff(true, true), &[]);
        assert!(file.windows(15).any(|w| w == b"Somebody Secret"));
        let (out, _) = remove(&file, Selection::privacy());
        assert!(!out.windows(15).any(|w| w == b"Somebody Secret"));
        assert!(!out.windows(7).any(|w| w == b"TestCam"));
    }

    #[test]
    fn privacy_keeps_orientation_and_settings() {
        let file = jpeg_with(&sample_tiff(true, true), &[]);
        let (out, _) = remove(&file, Selection::privacy());
        let exif = exif_of_jpeg(&out).expect("some EXIF stays");
        assert!(exif.get_field(Tag::Orientation, In::PRIMARY).is_some());
        assert!(exif.get_field(Tag::ExposureTime, In::PRIMARY).is_some());
        assert!(exif.get_field(Tag::Make, In::PRIMARY).is_none());
        assert!(exif.get_field(Tag::Artist, In::PRIMARY).is_none());
        assert!(exif.get_field(Tag::DateTimeOriginal, In::PRIMARY).is_none());
        assert!(exif.get_field(Tag::Compression, In::THUMBNAIL).is_none());
        still_decodes(&out);
    }

    #[test]
    fn removing_everything_drops_the_exif_segment() {
        let file = jpeg_with(&sample_tiff(true, true), &[]);
        let sel = Selection { orientation: true, ..Selection::all() };
        let (out, _) = remove(&file, sel);
        assert!(exif_of_jpeg(&out).is_none());
        assert!(!out.windows(6).any(|w| w == b"Exif\0\0"));
        still_decodes(&out);
    }

    #[test]
    fn single_tags_can_be_named() {
        let file = jpeg_with(&sample_tiff(false, false), &[]);
        let sel = Selection { tags: parse_tags("model, 0x013B"), ..Selection::default() };
        let (out, removed) = remove(&file, sel);
        let exif = exif_of_jpeg(&out).unwrap();
        assert!(exif.get_field(Tag::Model, In::PRIMARY).is_none());
        assert!(exif.get_field(Tag::Artist, In::PRIMARY).is_none());
        assert!(exif.get_field(Tag::Make, In::PRIMARY).is_some());
        assert_eq!(removed.len(), 2);
    }

    #[test]
    fn nothing_selected_changes_nothing() {
        let file = jpeg_with(&sample_tiff(true, true), &[]);
        let (out, removed) = remove(&file, Selection::default());
        assert_eq!(out, file);
        assert!(removed.is_empty());
    }

    #[test]
    fn jpeg_comment_xmp_and_icc_are_separate_groups() {
        let mut xmp = b"http://ns.adobe.com/xap/1.0/\0".to_vec();
        xmp.extend_from_slice(b"<x:xmpmeta><rdf:RDF><rdf:Description xmp:CreatorTool=\"Tool 1\"/></rdf:RDF></x:xmpmeta>");
        let extra = vec![(0xFE, b"hello".to_vec()), (0xE1, xmp), (0xE2, b"ICC_PROFILE\0\x01\x01profile".to_vec())];
        let file = jpeg_with(&sample_tiff(true, false), &extra);

        let (out, removed) = remove(&file, Selection { comments: true, ..Selection::default() });
        assert_eq!(removed.len(), 1);
        assert!(!out.windows(5).any(|w| w == b"hello"));
        assert!(out.windows(9).any(|w| w == b"ICC_PROFI"));

        let (out, _) = remove(&file, Selection { xmp: true, ..Selection::default() });
        assert!(!out.windows(6).any(|w| w == b"xmpmet"));
        assert!(out.windows(5).any(|w| w == b"hello"));

        let (out, _) = remove(&file, Selection { icc: true, ..Selection::default() });
        assert!(!out.windows(9).any(|w| w == b"ICC_PROFI"));
        still_decodes(&out);
    }

    #[test]
    fn report_lists_gps_and_the_findings() {
        let file = jpeg_with(&sample_tiff(true, true), &[]);
        let doc = containers::parse(&file).unwrap();
        let text = report::render("a.jpg", &file, &doc, None);
        assert!(text.starts_with("== Privacy findings =="));
        assert!(text.contains("GPS location: 51.034167, 7.016944"));
        assert!(text.contains("Make [0x010F]"));
        assert!(text.contains("TestCam"));
        assert!(text.contains("Embedded thumbnail"));
        assert!(text.contains("== File structure =="));
    }

    fn png_chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut out = (data.len() as u32).to_be_bytes().to_vec();
        out.extend_from_slice(kind);
        out.extend_from_slice(data);
        out.extend_from_slice(&containers::crc32_for_tests(&[&kind[..], data]).to_be_bytes());
        out
    }

    #[test]
    fn png_text_time_and_exif() {
        let base = picture(ImageFormat::Png);
        let iend = base.len() - 12;
        let mut file = base[..iend].to_vec();
        file.extend(png_chunk(b"tEXt", b"Software\0Secret Editor"));
        file.extend(png_chunk(b"tIME", &[0x07, 0xE4, 1, 2, 3, 4, 5]));
        file.extend(png_chunk(b"eXIf", &sample_tiff(true, false)));
        file.extend_from_slice(&base[iend..]);
        still_decodes(&file);

        let doc = containers::parse(&file).unwrap();
        let text = report::render("a.png", &file, &doc, None);
        assert!(text.contains("Software"));
        assert!(text.contains("Secret Editor"));
        assert!(text.contains("2020-01-02 03:04:05 UTC"));
        assert!(text.contains("GPS location"));

        let (out, removed) = remove(&file, Selection::privacy());
        assert!(!out.windows(6).any(|w| w == b"Secret"));
        assert!(!out.windows(4).any(|w| w == b"tIME"));
        assert!(removed.len() >= 3);
        still_decodes(&out);
        let exif = Reader::new().read_from_container(&mut Cursor::new(&out)).unwrap();
        assert!(exif.get_field(Tag::GPSLatitude, In::PRIMARY).is_none());
        assert!(exif.get_field(Tag::Orientation, In::PRIMARY).is_some());
    }

    #[test]
    fn webp_flags_and_size_follow_the_removal() {
        let plain = picture(ImageFormat::WebP);
        assert_eq!(&plain[12..16], b"VP8L");
        let tiff = sample_tiff(true, false);
        let mut file = b"RIFF\0\0\0\0WEBP".to_vec();
        file.extend_from_slice(b"VP8X");
        file.extend_from_slice(&10u32.to_le_bytes());
        file.extend_from_slice(&[0x08, 0, 0, 0, 7, 0, 0, 7, 0, 0]);
        file.extend_from_slice(&plain[12..]);
        file.extend_from_slice(b"EXIF");
        file.extend_from_slice(&(tiff.len() as u32).to_le_bytes());
        file.extend_from_slice(&tiff);
        if tiff.len() % 2 == 1 {
            file.push(0);
        }
        let size = (file.len() - 8) as u32;
        file[4..8].copy_from_slice(&size.to_le_bytes());
        still_decodes(&file);

        let sel = Selection { orientation: true, ..Selection::all() };
        let (out, removed) = remove(&file, sel);
        assert!(!removed.is_empty());
        assert_eq!(out[20] & 0x08, 0, "the EXIF flag is cleared");
        assert_eq!(u32::from_le_bytes([out[4], out[5], out[6], out[7]]) as usize, out.len() - 8);
        assert!(!out.windows(4).any(|w| w == b"EXIF"));
        still_decodes(&out);
    }

    #[test]
    fn unsupported_and_plain_formats() {
        let bmp = picture(ImageFormat::Bmp);
        let doc = containers::parse(&bmp).unwrap();
        let out = containers::strip(&bmp, &doc, &Selection::all()).unwrap();
        assert_eq!(out.bytes, bmp);
        let gif = picture(ImageFormat::Gif);
        let doc = containers::parse(&gif).unwrap();
        assert!(containers::strip(&gif, &doc, &Selection::all()).is_err());
        assert!(containers::parse(b"definitely not an image").is_err());
    }

    #[test]
    fn tool_runs_in_both_modes() {
        let mut registry = Registry::new();
        registry.register(Metadata).unwrap();
        let file = jpeg_with(&sample_tiff(true, true), &[]);
        let input = Input::single(Item::bytes(file).named("dir/photo.jpg"));

        let out = registry.run("image.metadata", &input, &Params::new(), &Job::new()).unwrap();
        assert_eq!(out.items.len(), 1);
        assert_eq!(out.items[0].name.as_deref(), Some("photo.jpg.metadata.txt"));
        assert!(matches!(out.items[0].data, Data::Text(_)));

        let params = Params::new().with("mode", "remove");
        let out = registry.run("image.metadata", &input, &params, &Job::new()).unwrap();
        assert_eq!(out.items.len(), 2);
        assert_eq!(out.items[0].name.as_deref(), Some("photo.jpg"));
        assert!(matches!(out.items[0].data, Data::Bytes(_)));
        let Data::Text(report) = &out.items[1].data else { panic!("report expected") };
        assert!(report.contains("== Removed =="));

        let params = Params::new().with("mode", "remove").with("preset", "custom").with("tags", "make");
        let out = registry.run("image.metadata", &input, &params, &Job::new()).unwrap();
        let Data::Text(report) = &out.items[1].data else { panic!("report expected") };
        assert!(report.contains("- Make (main)"));
        assert!(!report.contains("TestCam"));
    }
}
