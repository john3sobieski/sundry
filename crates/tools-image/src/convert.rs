//! `image.convert`: re-encodes pictures into another file format.

use sundry_core::{
    Category, Data, Error, Input, InputKind, Item, Job, Media, Output, ParamSpec, Params, Result,
    Tool, ToolInfo,
};

use crate::codec::{self, OutFormat};
use crate::map_items;
use crate::plan;

pub struct Convert;

/// Settings checked and parsed once per run.
struct Settings {
    format: OutFormat,
    quality: u8,
    background: [u8; 4],
}

impl Settings {
    fn from_params(params: &Params) -> Result<Self> {
        let invalid = |name: &str, reason: &str| Error::InvalidParam {
            name: name.into(),
            reason: reason.into(),
        };
        let format = OutFormat::parse(params.get_text("format")?)
            .ok_or_else(|| invalid("format", "unknown"))?;
        Ok(Self {
            format,
            quality: u8::try_from(params.get_int("quality")?.clamp(1, 100)).unwrap_or(90),
            background: plan::parse_color(params.get_text("background")?)?,
        })
    }

    fn process(&self, item: &Item) -> Result<Item> {
        let Data::Bytes(bytes) = &item.data else {
            return Err(Error::InvalidInput("expected an image file".to_owned()));
        };
        // The format is detected from the content, not from the file name.
        if let Some((_, _, Some(source))) = codec::probe(bytes)
            && OutFormat::from_image_format(source) == Some(self.format)
        {
            let name = codec::format_name(Some(source));
            return Err(Error::InvalidInput(format!("already in {name} format")));
        }
        let decoded = codec::decode(bytes)?;
        let data = codec::encode(decoded.image, self.format, self.quality, self.background)?;
        Ok(Item::bytes(data).named(codec::output_name(item.name.as_deref(), self.format)))
    }
}

impl Tool for Convert {
    fn info(&self) -> ToolInfo {
        ToolInfo::new(
            "image.convert",
            Category::Image,
            "Convert image",
            "Converts images between PNG, JPEG, WebP, AVIF, BMP, GIF, TIFF and ICO, one file or \
             a whole batch. Metadata is dropped and animations keep only their first frame.",
        )
        .with_input(InputKind::Files)
        .with_media(Media::Image)
        .per_file()
    }

    fn params(&self) -> Vec<ParamSpec> {
        vec![
            ParamSpec::choice("format", OutFormat::IDS, "png"),
            ParamSpec::int_range("quality", 90, 1, 100).show_when("format", &["jpeg", "avif"]),
            ParamSpec::text("background", "white").show_when("format", &["jpeg"]),
        ]
    }

    fn run(&self, input: &Input, params: &Params, job: &Job) -> Result<Output> {
        let settings = Settings::from_params(params)?;
        map_items(input, job, |item| settings.process(item))
    }
}

#[cfg(test)]
mod tests {
    use image::{Rgba, RgbaImage};
    use sundry_core::registry::Registry;

    use super::*;

    fn registry() -> Registry {
        let mut registry = Registry::new();
        registry.register(Convert).unwrap();
        registry
    }

    fn pixels(width: u32, height: u32, alpha: u8) -> RgbaImage {
        RgbaImage::from_fn(width, height, |x, y| {
            let noise = (x * 31 + y * 17 + x * y) % 256;
            Rgba([(x % 256) as u8, (y % 256) as u8, noise as u8, alpha])
        })
    }

    fn picture_as(format: OutFormat, width: u32, height: u32, alpha: u8, name: &str) -> Item {
        let bytes = codec::encode(pixels(width, height, alpha), format, 90, [0; 4]);
        Item::bytes(bytes.unwrap()).named(name)
    }

    fn picture(width: u32, height: u32, alpha: u8, name: &str) -> Item {
        picture_as(OutFormat::Png, width, height, alpha, name)
    }

    fn convert(params: Params, input: Input) -> Result<Output> {
        registry().run("image.convert", &input, &params, &Job::new())
    }

    fn convert_one(params: Params, item: Item) -> Result<Item> {
        Ok(convert(params, Input::single(item))?.items.into_iter().next().unwrap())
    }

    #[test]
    fn the_default_settings_are_valid() {
        let item = picture_as(OutFormat::Jpeg, 20, 10, 255, "dir/photo.JPG");
        let out = convert_one(Params::new(), item).unwrap();
        assert_eq!(out.name.as_deref(), Some("photo.png"));
        let (w, h, format) = codec::probe(out.data.as_bytes()).unwrap();
        assert_eq!((w, h, format), (20, 10, Some(image::ImageFormat::Png)));
    }

    #[test]
    fn every_target_format_is_written() {
        for id in OutFormat::IDS {
            let params = Params::new().with("format", *id);
            // The source must differ from the target format.
            let source = if *id == "png" { OutFormat::Bmp } else { OutFormat::Png };
            let out = convert_one(params, picture_as(source, 24, 16, 255, "a")).unwrap();
            let format = OutFormat::parse(id).unwrap();
            assert_eq!(out.name.as_deref(), Some(format!("a.{}", format.extension()).as_str()));
            let bytes = out.data.as_bytes();
            if *id == "avif" {
                // AVIF cannot be decoded here; check the container signature instead.
                assert_eq!(&bytes[4..8], b"ftyp");
            } else {
                assert_eq!(codec::probe(bytes).map(|(w, h, _)| (w, h)), Some((24, 16)), "{id}");
                assert!(codec::decode(bytes).is_ok(), "{id}");
            }
        }
    }

    #[test]
    fn lossless_targets_keep_pixels_and_transparency() {
        let source = pixels(16, 16, 128);
        for id in ["png", "webp", "tiff"] {
            let params = Params::new().with("format", id);
            let from = if id == "png" { OutFormat::Tiff } else { OutFormat::Png };
            let out = convert_one(params, picture_as(from, 16, 16, 128, "a")).unwrap();
            let back = codec::decode(out.data.as_bytes()).unwrap().image;
            assert_eq!(back, source, "{id}");
        }
    }

    #[test]
    fn jpeg_uses_the_background_and_the_quality() {
        let clear = |name: &str| picture(8, 8, 0, name);
        let white = convert_one(Params::new().with("format", "jpeg"), clear("a.png")).unwrap();
        let image = codec::decode(white.data.as_bytes()).unwrap().image;
        assert!(image.pixels().all(|p| p.0[..3].iter().all(|&c| c > 240)));

        let params = Params::new().with("format", "jpeg").with("background", "#000000");
        let black = convert_one(params, clear("a.png")).unwrap();
        let image = codec::decode(black.data.as_bytes()).unwrap().image;
        assert!(image.pixels().all(|p| p.0[..3].iter().all(|&c| c < 15)));

        let size = |quality: i64| {
            let params = Params::new().with("format", "jpeg").with("quality", quality);
            convert_one(params, picture(64, 64, 255, "a.png")).unwrap().data.as_bytes().len()
        };
        assert!(size(10) < size(95));
    }

    #[test]
    fn input_in_any_supported_format_is_accepted() {
        let source = pixels(12, 12, 255);
        for format in [OutFormat::Jpeg, OutFormat::Bmp, OutFormat::Gif, OutFormat::Tiff] {
            let bytes = codec::encode(source.clone(), format, 90, [0; 4]).unwrap();
            let item = Item::bytes(bytes).named("in");
            let out = convert_one(Params::new().with("format", "webp"), item).unwrap();
            assert_eq!(out.name.as_deref(), Some("in.webp"), "{format:?}");
        }
    }

    #[test]
    fn the_same_format_is_refused() {
        let formats = [
            (OutFormat::Png, "png"),
            (OutFormat::Jpeg, "jpeg"),
            (OutFormat::Bmp, "bmp"),
            (OutFormat::Gif, "gif"),
        ];
        for (format, id) in formats {
            let params = Params::new().with("format", id);
            let err = convert_one(params, picture_as(format, 8, 8, 255, "a.xyz")).unwrap_err();
            assert!(err.to_string().starts_with("a.xyz: already in "), "{format:?}: {err}");
        }
        // Detection goes by content: a JPEG named `.png` is still a JPEG.
        let item = picture_as(OutFormat::Jpeg, 8, 8, 255, "fake.png");
        assert!(convert_one(Params::new().with("format", "jpeg"), item.clone()).is_err());
        assert!(convert_one(Params::new().with("format", "png"), item).is_ok());
    }

    #[test]
    fn a_batch_keeps_order_and_names() {
        let input =
            Input { items: vec![picture(10, 10, 255, "one.png"), picture(20, 10, 255, "two.png")] };
        let out = convert(Params::new().with("format", "bmp"), input).unwrap();
        let names: Vec<_> = out.items.iter().map(|i| i.name.clone().unwrap()).collect();
        assert_eq!(names, ["one.bmp", "two.bmp"]);
    }

    #[test]
    fn unnamed_input_gets_a_default_name() {
        let item =
            Item::bytes(codec::encode(pixels(4, 4, 255), OutFormat::Png, 90, [0; 4]).unwrap());
        let out = convert_one(Params::new().with("format", "gif"), item).unwrap();
        assert_eq!(out.name.as_deref(), Some("image.gif"));
    }

    #[test]
    fn errors_name_the_file() {
        let params = Params::new().with("format", "ico");
        let err = convert_one(params, picture(300, 10, 255, "wide.png")).unwrap_err();
        assert!(err.to_string().starts_with("wide.png: "), "{err}");

        let broken = Item::bytes(b"junk".to_vec()).named("broken.png");
        let err = convert_one(Params::new(), broken).unwrap_err();
        assert!(err.to_string().starts_with("broken.png: "), "{err}");

        assert!(convert_one(Params::new().with("format", "pdf"), picture(4, 4, 255, "a")).is_err());
        let color = Params::new().with("format", "jpeg").with("background", "red-ish");
        assert!(matches!(
            convert_one(color, picture(4, 4, 255, "a")),
            Err(Error::InvalidParam { .. })
        ));
    }
}
