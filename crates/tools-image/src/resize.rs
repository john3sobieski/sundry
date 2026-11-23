//! `image.resize`: changes the size or proportion of pictures.

use fast_image_resize::images::Image as FirImage;
use fast_image_resize::{FilterType, PixelType, ResizeAlg, ResizeOptions, Resizer};
use image::{Rgba, RgbaImage};
use sundry_core::{
    Category, Data, Error, Input, InputKind, Item, Job, Media, Output, ParamSpec, Params, Result,
    Tool, ToolInfo,
};

use crate::codec::{self, OutFormat};
use crate::map_items;
use crate::plan::{self, Fit, MAX_SIDE, Mode};

const FILTERS: &[&str] = &["lanczos3", "catmull_rom", "mitchell", "bilinear", "box", "nearest"];
const MODES: &[&str] = &["size", "percent", "long_side", "short_side", "ratio"];
const FITS: &[&str] = &["stretch", "inside", "contain", "cover"];

pub struct Resize;

fn algorithm(filter: &str) -> ResizeAlg {
    match filter {
        "nearest" => ResizeAlg::Nearest,
        "bilinear" => ResizeAlg::Convolution(FilterType::Bilinear),
        "box" => ResizeAlg::Convolution(FilterType::Box),
        "mitchell" => ResizeAlg::Convolution(FilterType::Mitchell),
        "catmull_rom" => ResizeAlg::Convolution(FilterType::CatmullRom),
        _ => ResizeAlg::Convolution(FilterType::Lanczos3),
    }
}

fn processing(err: impl std::fmt::Display) -> Error {
    Error::Processing(format!("resize failed: {err}"))
}

fn resize_buffer(image: RgbaImage, out: (u32, u32), crop: bool, filter: &str) -> Result<RgbaImage> {
    let (width, height) = image.dimensions();
    if (width, height) == out {
        return Ok(image);
    }
    let src = FirImage::from_vec_u8(width, height, image.into_raw(), PixelType::U8x4)
        .map_err(processing)?;
    let mut dst = FirImage::new(out.0, out.1, PixelType::U8x4);
    let mut options = ResizeOptions::new().resize_alg(algorithm(filter));
    if crop {
        options = options.fit_into_destination(Some((0.5, 0.5)));
    }
    Resizer::new().resize(&src, &mut dst, &options).map_err(processing)?;
    RgbaImage::from_raw(out.0, out.1, dst.into_vec())
        .ok_or_else(|| processing("unexpected buffer size"))
}

/// Scales a whole picture (or its center crop) to `out`.
pub fn resample(image: RgbaImage, out: (u32, u32), crop: bool, filter: &str) -> Result<RgbaImage> {
    resize_buffer(image, out, crop, filter)
}

/// Settings checked and parsed once per run.
struct Settings {
    mode: Mode,
    no_upscale: bool,
    filter: String,
    format: Option<OutFormat>,
    quality: u8,
    background: [u8; 4],
}

impl Settings {
    fn from_params(params: &Params) -> Result<Self> {
        let invalid = |name: &str, reason: &str| Error::InvalidParam {
            name: name.into(),
            reason: reason.into(),
        };
        let side = |name: &str| -> Result<u32> {
            u32::try_from(params.get_int(name)?).map_err(|_| invalid(name, "out of range"))
        };
        let fit = Fit::parse(params.get_text("fit")?).ok_or_else(|| invalid("fit", "unknown"))?;
        let mode = match params.get_text("mode")? {
            "percent" => Mode::Percent(params.get_float("percent")?),
            "long_side" => Mode::LongSide(side("long_side")?),
            "short_side" => Mode::ShortSide(side("short_side")?),
            "ratio" => Mode::Ratio {
                ratio: plan::parse_ratio(params.get_text("ratio")?)?,
                width: side("width")?,
                fit,
            },
            _ => Mode::Size { width: side("width")?, height: side("height")?, fit },
        };
        let format = match params.get_text("format")? {
            "keep" => None,
            id => Some(OutFormat::parse(id).ok_or_else(|| invalid("format", "unknown"))?),
        };
        Ok(Self {
            mode,
            no_upscale: params.get_bool("no_upscale")?,
            filter: params.get_text("filter")?.to_owned(),
            format,
            quality: u8::try_from(params.get_int("quality")?.clamp(1, 100)).unwrap_or(90),
            background: plan::parse_color(params.get_text("background")?)?,
        })
    }

    fn process(&self, item: &Item) -> Result<Item> {
        let Data::Bytes(bytes) = &item.data else {
            return Err(Error::InvalidInput("expected an image file".to_owned()));
        };
        let decoded = codec::decode(bytes)?;
        let plan = plan::plan(&self.mode, decoded.image.dimensions(), self.no_upscale)?;
        let mut image = resize_buffer(decoded.image, plan.inner, plan.crop, &self.filter)?;
        if plan.out != plan.inner {
            let mut canvas = RgbaImage::from_pixel(plan.out.0, plan.out.1, Rgba(self.background));
            let x = i64::from((plan.out.0 - plan.inner.0) / 2);
            let y = i64::from((plan.out.1 - plan.inner.1) / 2);
            image::imageops::overlay(&mut canvas, &image, x, y);
            image = canvas;
        }
        // Keep the input format when it can be written back, otherwise PNG.
        let format = self.format.unwrap_or_else(|| {
            decoded.format.and_then(OutFormat::from_image_format).unwrap_or(OutFormat::Png)
        });
        let data = codec::encode(image, format, self.quality, self.background)?;
        Ok(Item::bytes(data).named(codec::output_name(item.name.as_deref(), format)))
    }
}

impl Tool for Resize {
    fn info(&self) -> ToolInfo {
        ToolInfo::new(
            "image.resize",
            Category::Image,
            "Resize image",
            "Changes the size or proportion of images, one file or a whole batch. Metadata is \
             dropped and animations keep only their first frame.",
        )
        .with_input(InputKind::Files)
        .with_media(Media::Image)
        .per_file()
    }

    fn params(&self) -> Vec<ParamSpec> {
        let mut formats = vec!["keep"];
        formats.extend_from_slice(OutFormat::IDS);
        let boxed = ["size", "ratio"];
        vec![
            ParamSpec::choice("mode", MODES, "size"),
            ParamSpec::int_range("width", 1920, 1, i64::from(MAX_SIDE)).show_when("mode", &boxed),
            ParamSpec::int_range("height", 1080, 1, i64::from(MAX_SIDE))
                .show_when("mode", &["size"]),
            ParamSpec::text("ratio", "16:9").show_when("mode", &["ratio"]),
            ParamSpec::choice("fit", FITS, "contain").show_when("mode", &boxed),
            ParamSpec::text("background", "transparent")
                .show_when("mode", &boxed)
                .show_when("fit", &["contain"]),
            ParamSpec::float_range("percent", 50.0, 0.1, 1000.0).show_when("mode", &["percent"]),
            ParamSpec::int_range("long_side", 1920, 1, i64::from(MAX_SIDE))
                .show_when("mode", &["long_side"]),
            ParamSpec::int_range("short_side", 1080, 1, i64::from(MAX_SIDE))
                .show_when("mode", &["short_side"]),
            ParamSpec::boolean("no_upscale", false),
            ParamSpec::choice("filter", FILTERS, "lanczos3"),
            ParamSpec::choice("format", &formats, "keep"),
            ParamSpec::int_range("quality", 90, 1, 100)
                .show_when("format", &["keep", "jpeg", "avif"]),
        ]
    }

    fn run(&self, input: &Input, params: &Params, job: &Job) -> Result<Output> {
        let settings = Settings::from_params(params)?;
        map_items(input, job, |item| settings.process(item))
    }
}

#[cfg(test)]
mod tests {
    use sundry_core::registry::Registry;

    use super::*;

    fn registry() -> Registry {
        let mut registry = Registry::new();
        registry.register(Resize).unwrap();
        registry
    }

    fn picture(width: u32, height: u32) -> Item {
        let image = RgbaImage::from_fn(width, height, |x, y| {
            Rgba([(x % 256) as u8, (y % 256) as u8, 7, 255])
        });
        let bytes = codec::encode(image, OutFormat::Png, 90, [0; 4]).unwrap();
        Item::bytes(bytes).named("dir/photo.JPG")
    }

    fn run(params: Params, item: Item) -> Result<Item> {
        let out = registry().run("image.resize", &Input::single(item), &params, &Job::new())?;
        Ok(out.items.into_iter().next().unwrap())
    }

    fn size_of(item: &Item) -> (u32, u32) {
        let (w, h, _) = codec::probe(item.data.as_bytes()).unwrap();
        (w, h)
    }

    #[test]
    fn the_default_settings_are_valid() {
        let out = run(Params::new(), picture(240, 192)).unwrap();
        assert_eq!(size_of(&out), (1920, 1080));
        assert_eq!(out.name.as_deref(), Some("photo.png"));
    }

    #[test]
    fn user_example_every_fit_mode() {
        // 244x192 has the proportion of 2440x1920; the target is 1920x1080.
        let expected = [
            ("stretch", (1920, 1080)),
            ("inside", (1373, 1080)),
            ("contain", (1920, 1080)),
            ("cover", (1920, 1080)),
        ];
        for (fit, size) in expected {
            let out = run(Params::new().with("fit", fit), picture(244, 192)).unwrap();
            assert_eq!(size_of(&out), size, "{fit}");
        }
    }

    #[test]
    fn contain_pads_with_the_background() {
        let params = Params::new()
            .with("width", 100)
            .with("height", 100)
            .with("fit", "contain")
            .with("background", "#ff0000")
            .with("format", "png");
        let out = run(params, picture(50, 25)).unwrap();
        let decoded = codec::decode(out.data.as_bytes()).unwrap().image;
        assert_eq!(decoded.dimensions(), (100, 100));
        assert_eq!(decoded.get_pixel(50, 2).0, [255, 0, 0, 255]);
        assert_ne!(decoded.get_pixel(50, 50).0, [255, 0, 0, 255]);
    }

    #[test]
    fn percent_ratio_and_format_change() {
        let params =
            Params::new().with("mode", "percent").with("percent", 50.0).with("format", "webp");
        let out = run(params, picture(200, 100)).unwrap();
        assert_eq!((size_of(&out), out.name.as_deref()), ((100, 50), Some("photo.webp")));

        let params = Params::new().with("mode", "ratio").with("ratio", "1:1").with("width", 64);
        let out = run(params, picture(200, 100)).unwrap();
        assert_eq!(size_of(&out), (64, 64));
    }

    #[test]
    fn bad_settings_and_files_are_reported() {
        let ratio = Params::new().with("mode", "ratio").with("ratio", "nonsense");
        assert!(matches!(run(ratio, picture(8, 8)), Err(Error::InvalidParam { .. })));
        let color = Params::new().with("background", "red-ish");
        assert!(matches!(run(color, picture(8, 8)), Err(Error::InvalidParam { .. })));
        let broken = Item::bytes(b"junk".to_vec()).named("broken.png");
        let err = run(Params::new(), broken).unwrap_err();
        assert!(err.to_string().starts_with("broken.png: "), "{err}");
    }

    #[test]
    fn several_items_keep_their_order() {
        let input = Input { items: vec![picture(10, 10), picture(20, 10)] };
        let params = Params::new().with("mode", "percent").with("percent", 200.0);
        let out = registry().run("image.resize", &input, &params, &Job::new()).unwrap();
        let sizes: Vec<_> = out.items.iter().map(size_of).collect();
        assert_eq!(sizes, [(20, 20), (40, 20)]);
    }
}
