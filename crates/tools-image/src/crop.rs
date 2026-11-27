//! `image.crop`: cuts a part out of pictures (an area, margins to trim, or an aspect ratio).

use image::RgbaImage;
use image::imageops::crop_imm;
use sundry_core::{
    Category, Data, Error, Input, InputKind, Item, Job, Media, Output, ParamSpec, Params, Result,
    Tool, ToolInfo,
};

use crate::codec::{self, Target};
use crate::map_items;
use crate::plan::{self, MAX_SIDE};

pub struct Crop;

const MODES: &[&str] = &["ratio", "box", "margins"];
const ANCHORS: &[&str] = &["center", "start", "end"];
const UNITS: &[&str] = &["px", "percent"];

/// Where the kept strip lies when only one axis is cut (left or top = start).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Anchor {
    Start,
    Center,
    End,
}

impl Anchor {
    fn parse(id: &str) -> Option<Self> {
        match id {
            "start" => Some(Anchor::Start),
            "center" => Some(Anchor::Center),
            "end" => Some(Anchor::End),
            _ => None,
        }
    }

    /// Offset of a piece of length `piece` inside `total` (`piece <= total`).
    fn offset(self, total: u32, piece: u32) -> u32 {
        match self {
            Anchor::Start => 0,
            Anchor::Center => (total - piece) / 2,
            Anchor::End => total - piece,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unit {
    Px,
    /// Percent of the side the value is measured along.
    Percent,
}

impl Unit {
    /// Whole pixels for `value` along a side of `total` pixels.
    fn to_px(self, value: f64, total: u32) -> i64 {
        let px = match self {
            Unit::Px => value,
            Unit::Percent => value * f64::from(total) / 100.0,
        };
        px.round() as i64
    }
}

/// What the user asked for.
#[derive(Debug, Clone, PartialEq)]
pub enum Area {
    /// A rectangle: offset from the top-left corner and size.
    Box { x: f64, y: f64, width: f64, height: f64, unit: Unit },
    /// How much to cut off each side.
    Margins { left: f64, top: f64, right: f64, bottom: f64, unit: Unit },
    /// The biggest part with the given proportion (width / height).
    Ratio { ratio: f64, anchor: Anchor },
}

/// A rectangle of pixels inside a picture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// Checks that the edges give a non-empty rectangle inside a `size` picture.
fn within(left: i64, top: i64, right: i64, bottom: i64, size: (u32, u32)) -> Result<Rect> {
    let (w, h) = (i64::from(size.0), i64::from(size.1));
    if right <= left || bottom <= top {
        return Err(Error::InvalidInput("the crop area is empty".to_owned()));
    }
    if left < 0 || top < 0 || right > w || bottom > h {
        return Err(Error::InvalidInput(format!(
            "the crop area does not fit into the {w}x{h} picture"
        )));
    }
    Ok(Rect {
        x: left as u32,
        y: top as u32,
        width: (right - left) as u32,
        height: (bottom - top) as u32,
    })
}

/// The rectangle to keep from a picture of `size`.
pub fn crop_rect(area: &Area, size: (u32, u32)) -> Result<Rect> {
    let (w, h) = size;
    match *area {
        // Both edges are rounded, not the width, so 50 % + 50 % always ends at the border.
        Area::Box { x, y, width, height, unit } => within(
            unit.to_px(x, w),
            unit.to_px(y, h),
            unit.to_px(x + width, w),
            unit.to_px(y + height, h),
            size,
        ),
        Area::Margins { left, top, right, bottom, unit } => within(
            unit.to_px(left, w),
            unit.to_px(top, h),
            i64::from(w) - unit.to_px(right, w),
            i64::from(h) - unit.to_px(bottom, h),
            size,
        ),
        Area::Ratio { ratio, anchor } => {
            if w == 0 || h == 0 {
                return Err(Error::InvalidInput("the picture is empty".to_owned()));
            }
            if f64::from(w) / f64::from(h) > ratio {
                let width = ((f64::from(h) * ratio).round() as u32).clamp(1, w);
                Ok(Rect { x: anchor.offset(w, width), y: 0, width, height: h })
            } else {
                let height = ((f64::from(w) / ratio).round() as u32).clamp(1, h);
                Ok(Rect { x: 0, y: anchor.offset(h, height), width: w, height })
            }
        }
    }
}

/// Settings checked and parsed once per run.
struct Settings {
    area: Area,
    target: Target,
}

impl Settings {
    fn from_params(params: &Params) -> Result<Self> {
        let unit = match params.get_text("unit")? {
            "percent" => Unit::Percent,
            _ => Unit::Px,
        };
        let number = |name: &str| params.get_float(name);
        let area = match params.get_text("mode")? {
            "box" => Area::Box {
                x: number("x")?,
                y: number("y")?,
                width: number("width")?,
                height: number("height")?,
                unit,
            },
            "margins" => Area::Margins {
                left: number("left")?,
                top: number("top")?,
                right: number("right")?,
                bottom: number("bottom")?,
                unit,
            },
            _ => {
                let anchor = Anchor::parse(params.get_text("anchor")?).ok_or_else(|| {
                    Error::InvalidParam { name: "anchor".to_owned(), reason: "unknown".to_owned() }
                })?;
                Area::Ratio { ratio: plan::parse_ratio(params.get_text("ratio")?)?, anchor }
            }
        };
        Ok(Self { area, target: Target::from_params(params)? })
    }

    fn process(&self, item: &Item) -> Result<Item> {
        let Data::Bytes(bytes) = &item.data else {
            return Err(Error::InvalidInput("expected an image file".to_owned()));
        };
        let decoded = codec::decode(bytes)?;
        let rect = crop_rect(&self.area, decoded.image.dimensions())?;
        let image: RgbaImage =
            crop_imm(&decoded.image, rect.x, rect.y, rect.width, rect.height).to_image();
        self.target.encode(image, decoded.format, item.name.as_deref(), [0; 4])
    }
}

impl Tool for Crop {
    fn info(&self) -> ToolInfo {
        ToolInfo::new(
            "image.crop",
            Category::Image,
            "Crop image",
            "Cuts a part out of images: a rectangle, margins trimmed from the sides, or the \
             biggest part with a given aspect ratio. One file or a whole batch. Metadata is \
             dropped and animations keep only their first frame.",
        )
        .with_input(InputKind::Files)
        .with_media(Media::Image)
        .per_file()
    }

    fn params(&self) -> Vec<ParamSpec> {
        let max = f64::from(MAX_SIDE);
        let mut specs = vec![
            ParamSpec::choice("mode", MODES, "ratio"),
            ParamSpec::text("ratio", "1:1").show_when("mode", &["ratio"]),
            ParamSpec::choice("anchor", ANCHORS, "center").show_when("mode", &["ratio"]),
            ParamSpec::choice("unit", UNITS, "px").show_when("mode", &["box", "margins"]),
            ParamSpec::float_range("x", 0.0, 0.0, max).show_when("mode", &["box"]),
            ParamSpec::float_range("y", 0.0, 0.0, max).show_when("mode", &["box"]),
            ParamSpec::float_range("width", 1000.0, 0.0, max).show_when("mode", &["box"]),
            ParamSpec::float_range("height", 1000.0, 0.0, max).show_when("mode", &["box"]),
            ParamSpec::float_range("left", 0.0, 0.0, max).show_when("mode", &["margins"]),
            ParamSpec::float_range("top", 0.0, 0.0, max).show_when("mode", &["margins"]),
            ParamSpec::float_range("right", 0.0, 0.0, max).show_when("mode", &["margins"]),
            ParamSpec::float_range("bottom", 0.0, 0.0, max).show_when("mode", &["margins"]),
        ];
        specs.extend(Target::specs());
        specs
    }

    fn run(&self, input: &Input, params: &Params, job: &Job) -> Result<Output> {
        let settings = Settings::from_params(params)?;
        map_items(input, job, |item| settings.process(item))
    }
}

#[cfg(test)]
mod tests {
    use image::{ImageFormat, Rgba};
    use sundry_core::registry::Registry;

    use super::*;
    use crate::codec::OutFormat;

    fn boxed(x: f64, y: f64, width: f64, height: f64, unit: Unit) -> Area {
        Area::Box { x, y, width, height, unit }
    }

    fn rect(x: u32, y: u32, width: u32, height: u32) -> Rect {
        Rect { x, y, width, height }
    }

    #[test]
    fn a_box_in_pixels() {
        let area = boxed(10.0, 20.0, 30.0, 40.0, Unit::Px);
        assert_eq!(crop_rect(&area, (100, 80)).unwrap(), rect(10, 20, 30, 40));
        // The whole picture is allowed; one pixel more is not.
        assert_eq!(
            crop_rect(&boxed(0.0, 0.0, 100.0, 80.0, Unit::Px), (100, 80)).unwrap(),
            rect(0, 0, 100, 80)
        );
        assert!(crop_rect(&boxed(0.0, 0.0, 101.0, 80.0, Unit::Px), (100, 80)).is_err());
        assert!(crop_rect(&boxed(80.0, 0.0, 30.0, 10.0, Unit::Px), (100, 80)).is_err());
        assert!(crop_rect(&boxed(0.0, 0.0, 0.0, 10.0, Unit::Px), (100, 80)).is_err());
    }

    #[test]
    fn a_box_in_percent_never_overshoots_on_odd_sizes() {
        // 50 % of 101 px is 50.5: both edges are rounded, so the right half still fits.
        let right_half = boxed(50.0, 0.0, 50.0, 100.0, Unit::Percent);
        assert_eq!(crop_rect(&right_half, (101, 60)).unwrap(), rect(51, 0, 50, 60));
        let all = boxed(0.0, 0.0, 100.0, 100.0, Unit::Percent);
        assert_eq!(crop_rect(&all, (101, 59)).unwrap(), rect(0, 0, 101, 59));
        assert!(crop_rect(&boxed(0.0, 0.0, 101.0, 100.0, Unit::Percent), (100, 100)).is_err());
    }

    #[test]
    fn margins_cut_the_sides() {
        let px = Area::Margins { left: 10.0, top: 5.0, right: 20.0, bottom: 5.0, unit: Unit::Px };
        assert_eq!(crop_rect(&px, (100, 50)).unwrap(), rect(10, 5, 70, 40));
        let pct =
            Area::Margins { left: 10.0, top: 10.0, right: 10.0, bottom: 10.0, unit: Unit::Percent };
        assert_eq!(crop_rect(&pct, (200, 100)).unwrap(), rect(20, 10, 160, 80));
        let none = Area::Margins { left: 0.0, top: 0.0, right: 0.0, bottom: 0.0, unit: Unit::Px };
        assert_eq!(crop_rect(&none, (7, 9)).unwrap(), rect(0, 0, 7, 9));
        let too_much =
            Area::Margins { left: 60.0, top: 0.0, right: 60.0, bottom: 0.0, unit: Unit::Px };
        assert!(crop_rect(&too_much, (100, 50)).is_err());
    }

    #[test]
    fn a_ratio_cuts_one_axis_only() {
        let square = |anchor| Area::Ratio { ratio: 1.0, anchor };
        assert_eq!(crop_rect(&square(Anchor::Center), (100, 60)).unwrap(), rect(20, 0, 60, 60));
        assert_eq!(crop_rect(&square(Anchor::Start), (100, 60)).unwrap(), rect(0, 0, 60, 60));
        assert_eq!(crop_rect(&square(Anchor::End), (100, 60)).unwrap(), rect(40, 0, 60, 60));
        assert_eq!(crop_rect(&square(Anchor::Center), (60, 100)).unwrap(), rect(0, 20, 60, 60));
        assert_eq!(crop_rect(&square(Anchor::End), (60, 100)).unwrap(), rect(0, 40, 60, 60));
        let wide = Area::Ratio { ratio: 2.0, anchor: Anchor::Center };
        assert_eq!(crop_rect(&wide, (100, 100)).unwrap(), rect(0, 25, 100, 50));
        // Already the right proportion: nothing is cut.
        let same = Area::Ratio { ratio: 16.0 / 9.0, anchor: Anchor::Center };
        assert_eq!(crop_rect(&same, (1920, 1080)).unwrap(), rect(0, 0, 1920, 1080));
    }

    #[test]
    fn extreme_ratios_and_tiny_pictures_do_not_panic() {
        for ratio in [0.01, 100.0] {
            for anchor in [Anchor::Start, Anchor::Center, Anchor::End] {
                for size in [(1, 1), (1, 9), (9, 1), (3, 5), (4000, 3)] {
                    let r = crop_rect(&Area::Ratio { ratio, anchor }, size).unwrap();
                    assert!(r.width >= 1 && r.height >= 1);
                    assert!(r.x + r.width <= size.0 && r.y + r.height <= size.1);
                }
            }
        }
    }

    fn registry() -> Registry {
        let mut registry = Registry::new();
        registry.register(Crop).unwrap();
        registry
    }

    /// Red is the column and green the row, so every pixel tells where it came from.
    fn picture(width: u32, height: u32) -> Item {
        let image = RgbaImage::from_fn(width, height, |x, y| Rgba([x as u8, y as u8, 7, 255]));
        let bytes = codec::encode(image, OutFormat::Png, 90, [0; 4]).unwrap();
        Item::bytes(bytes).named("dir/pic.JPG")
    }

    fn run(params: Params, item: Item) -> Result<Item> {
        let out = registry().run("image.crop", &Input::single(item), &params, &Job::new())?;
        Ok(out.items.into_iter().next().unwrap())
    }

    fn decoded(item: &Item) -> RgbaImage {
        codec::decode(item.data.as_bytes()).unwrap().image
    }

    #[test]
    fn the_default_settings_are_valid() {
        let out = run(Params::new(), picture(100, 60)).unwrap();
        assert_eq!(out.name.as_deref(), Some("pic.png"));
        let image = decoded(&out);
        assert_eq!(image.dimensions(), (60, 60));
        // Centered: the first column is column 20 of the source.
        assert_eq!(image.get_pixel(0, 0).0[..2], [20, 0]);
    }

    #[test]
    fn the_tool_keeps_the_chosen_pixels() {
        let params = Params::new()
            .with("mode", "box")
            .with("x", 10.0)
            .with("y", 20.0)
            .with("width", 5.0)
            .with("height", 4.0);
        let image = decoded(&run(params, picture(100, 80)).unwrap());
        assert_eq!(image.dimensions(), (5, 4));
        assert_eq!(image.get_pixel(0, 0).0, [10, 20, 7, 255]);
        assert_eq!(image.get_pixel(4, 3).0, [14, 23, 7, 255]);

        let params = Params::new().with("mode", "margins").with("left", 10.0).with("top", 30.0);
        let image = decoded(&run(params, picture(100, 80)).unwrap());
        assert_eq!(image.dimensions(), (90, 50));
        assert_eq!(image.get_pixel(0, 0).0, [10, 30, 7, 255]);
    }

    #[test]
    fn the_output_format_can_change() {
        let out = run(Params::new().with("format", "jpeg"), picture(40, 40)).unwrap();
        assert_eq!(out.name.as_deref(), Some("pic.jpg"));
        let (width, height, format) = codec::probe(out.data.as_bytes()).unwrap();
        assert_eq!((width, height, format), (40, 40, Some(ImageFormat::Jpeg)));
    }

    #[test]
    fn bad_settings_are_reported() {
        let outside = Params::new().with("mode", "box").with("x", 90.0).with("width", 50.0);
        let err = run(outside, picture(100, 80)).unwrap_err();
        assert!(err.to_string().starts_with("dir/pic.JPG: "), "{err}");
        let empty = Params::new().with("mode", "box").with("width", 0.0);
        assert!(run(empty, picture(100, 80)).is_err());
        assert!(run(Params::new().with("ratio", "wide"), picture(8, 8)).is_err());
        assert!(run(Params::new().with("mode", "diagonal"), picture(8, 8)).is_err());
        let broken = Item::bytes(b"junk".to_vec()).named("broken.png");
        assert!(run(Params::new(), broken).is_err());
    }
}
