//! `image.round_corners`: rounds the corners of pictures with an anti-aliased alpha mask.

use image::RgbaImage;
use sundry_core::{
    Category, Data, Error, Input, InputKind, Item, Job, Media, Output, ParamSpec, Params, Result,
    Tool, ToolInfo,
};

use crate::codec::{self, OutFormat};
use crate::map_items;
use crate::plan::MAX_SIDE;

pub struct RoundCorners;

/// Formats without an alpha channel cannot hold the transparent corners.
const NO_ALPHA: &[&str] = &["jpeg", "bmp", "gif"];

/// Share of the picture that can be rounded: a radius of half the shorter side is a full circle.
const MAX_PERCENT: f64 = 50.0;

/// Alpha of one pixel of the top-left corner (0 = outside the arc, 1 = inside).
fn coverage(x: u32, y: u32, radius: f64) -> f64 {
    let dx = radius - (f64::from(x) + 0.5);
    let dy = radius - (f64::from(y) + 0.5);
    if dx <= 0.0 || dy <= 0.0 {
        return 1.0;
    }
    (radius - dx.hypot(dy) + 0.5).clamp(0.0, 1.0)
}

/// Which corners are rounded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Corners {
    pub top_left: bool,
    pub top_right: bool,
    pub bottom_right: bool,
    pub bottom_left: bool,
}

impl Corners {
    pub const ALL: Corners =
        Corners { top_left: true, top_right: true, bottom_right: true, bottom_left: true };
    pub const NONE: Corners =
        Corners { top_left: false, top_right: false, bottom_right: false, bottom_left: false };

    pub fn any(self) -> bool {
        self.top_left || self.top_right || self.bottom_right || self.bottom_left
    }
}

/// Rounds the chosen corners. A radius above half of the shorter side is reduced to it.
pub fn round_corners(image: &mut RgbaImage, radius: f64, corners: Corners) {
    let (width, height) = image.dimensions();
    let radius = radius.min(f64::from(width.min(height)) / 2.0);
    if radius <= 0.0 {
        return;
    }
    let reach = (radius.ceil() as u32).min(width).min(height);
    for y in 0..reach {
        for x in 0..reach {
            let cover = coverage(x, y, radius);
            if cover >= 1.0 {
                continue;
            }
            for (px, py, on) in [
                (x, y, corners.top_left),
                (width - 1 - x, y, corners.top_right),
                (width - 1 - x, height - 1 - y, corners.bottom_right),
                (x, height - 1 - y, corners.bottom_left),
            ] {
                if !on {
                    continue;
                }
                let pixel = image.get_pixel_mut(px, py);
                pixel.0[3] = (f64::from(pixel.0[3]) * cover).round() as u8;
            }
        }
    }
}

#[derive(Clone, Copy)]
enum Radius {
    Px(f64),
    /// Percent of the shorter side.
    Percent(f64),
}

impl Radius {
    fn pixels(self, size: (u32, u32)) -> f64 {
        match self {
            Radius::Px(value) => value,
            Radius::Percent(value) => value / 100.0 * f64::from(size.0.min(size.1)),
        }
    }
}

/// Settings checked and parsed once per run.
struct Settings {
    radius: Radius,
    corners: Corners,
    format: OutFormat,
    quality: u8,
}

impl Settings {
    fn from_params(params: &Params) -> Result<Self> {
        let invalid = |name: &str, reason: &str| Error::InvalidParam {
            name: name.into(),
            reason: reason.into(),
        };
        let radius = match params.get_text("unit")? {
            "percent" => Radius::Percent(params.get_float("radius_percent")?),
            _ => Radius::Px(params.get_int("radius_px")? as f64),
        };
        let corners = Corners {
            top_left: params.get_bool("top_left")?,
            top_right: params.get_bool("top_right")?,
            bottom_right: params.get_bool("bottom_right")?,
            bottom_left: params.get_bool("bottom_left")?,
        };
        if !corners.any() {
            return Err(invalid("top_left", "select at least one corner"));
        }
        let format = OutFormat::parse(params.get_text("format")?)
            .ok_or_else(|| invalid("format", "unknown"))?;
        Ok(Self {
            radius,
            corners,
            format,
            quality: u8::try_from(params.get_int("quality")?.clamp(1, 100)).unwrap_or(90),
        })
    }

    fn process(&self, item: &Item) -> Result<Item> {
        let Data::Bytes(bytes) = &item.data else {
            return Err(Error::InvalidInput("expected an image file".to_owned()));
        };
        let mut image = codec::decode(bytes)?.image;
        let radius = self.radius.pixels(image.dimensions());
        round_corners(&mut image, radius, self.corners);
        let data = codec::encode(image, self.format, self.quality, [0; 4])?;
        Ok(Item::bytes(data).named(codec::output_name(item.name.as_deref(), self.format)))
    }
}

impl Tool for RoundCorners {
    fn info(&self) -> ToolInfo {
        ToolInfo::new(
            "image.round_corners",
            Category::Image,
            "Round corners",
            "Rounds all or only chosen corners of images with a smooth transparent edge, one file \
             or a whole batch. The result needs a format with transparency. Metadata is dropped and \
             animations keep only their first frame.",
        )
        .with_input(InputKind::Files)
        .with_media(Media::Image)
        .per_file()
    }

    fn params(&self) -> Vec<ParamSpec> {
        let formats: Vec<&str> =
            OutFormat::IDS.iter().copied().filter(|id| !NO_ALPHA.contains(id)).collect();
        vec![
            ParamSpec::choice("unit", &["px", "percent"], "px"),
            ParamSpec::int_range("radius_px", 24, 0, i64::from(MAX_SIDE))
                .show_when("unit", &["px"]),
            ParamSpec::float_range("radius_percent", 10.0, 0.0, MAX_PERCENT)
                .show_when("unit", &["percent"]),
            ParamSpec::boolean("top_left", true),
            ParamSpec::boolean("top_right", true),
            ParamSpec::boolean("bottom_right", true),
            ParamSpec::boolean("bottom_left", true),
            ParamSpec::choice("format", &formats, "png"),
            ParamSpec::int_range("quality", 90, 1, 100).show_when("format", &["avif"]),
        ]
    }

    fn run(&self, input: &Input, params: &Params, job: &Job) -> Result<Output> {
        let settings = Settings::from_params(params)?;
        map_items(input, job, |item| settings.process(item))
    }
}

#[cfg(test)]
mod tests {
    use image::Rgba;
    use sundry_core::registry::Registry;

    use super::*;

    fn opaque(width: u32, height: u32) -> RgbaImage {
        RgbaImage::from_pixel(width, height, Rgba([10, 120, 200, 255]))
    }

    fn alpha(image: &RgbaImage, x: u32, y: u32) -> u8 {
        image.get_pixel(x, y).0[3]
    }

    #[test]
    fn corners_become_transparent_and_the_rest_stays() {
        let mut image = opaque(100, 100);
        round_corners(&mut image, 20.0, Corners::ALL);
        for (x, y) in [(0, 0), (99, 0), (0, 99), (99, 99), (3, 3)] {
            assert_eq!(alpha(&image, x, y), 0, "({x}, {y})");
        }
        for (x, y) in [(50, 50), (50, 0), (0, 50), (99, 50), (50, 99), (20, 20)] {
            assert_eq!(alpha(&image, x, y), 255, "({x}, {y})");
        }
        assert_eq!(image.get_pixel(50, 50).0[..3], [10, 120, 200]);
    }

    #[test]
    fn the_edge_is_anti_aliased_and_symmetric() {
        let mut image = opaque(64, 64);
        round_corners(&mut image, 16.0, Corners::ALL);
        let soft = image.pixels().filter(|p| p.0[3] > 0 && p.0[3] < 255).count();
        assert!(soft > 0);
        for y in 0..64 {
            for x in 0..64 {
                assert_eq!(alpha(&image, x, y), alpha(&image, 63 - x, y));
                assert_eq!(alpha(&image, x, y), alpha(&image, x, 63 - y));
            }
        }
    }

    #[test]
    fn existing_transparency_is_kept() {
        let mut image = RgbaImage::from_pixel(40, 40, Rgba([1, 2, 3, 100]));
        round_corners(&mut image, 10.0, Corners::ALL);
        assert_eq!(alpha(&image, 20, 20), 100);
        assert_eq!(alpha(&image, 0, 0), 0);
    }

    #[test]
    fn zero_radius_changes_nothing_and_a_big_one_is_a_circle() {
        let mut image = opaque(30, 20);
        round_corners(&mut image, 0.0, Corners::ALL);
        assert_eq!(image, opaque(30, 20));

        let mut huge = opaque(30, 20);
        let mut exact = opaque(30, 20);
        round_corners(&mut huge, 5000.0, Corners::ALL);
        round_corners(&mut exact, 10.0, Corners::ALL);
        assert_eq!(huge, exact);
        assert_eq!(alpha(&huge, 15, 10), 255);
        assert_eq!(alpha(&huge, 0, 0), 0);
    }

    #[test]
    fn tiny_and_odd_sizes_do_not_panic() {
        for (w, h) in [(1, 1), (1, 7), (5, 5), (5, 40), (2, 2)] {
            let mut image = opaque(w, h);
            round_corners(&mut image, 100.0, Corners::ALL);
        }
    }

    #[test]
    fn single_corners_can_be_rounded() {
        let only = |corners: Corners| {
            let mut image = opaque(100, 100);
            round_corners(&mut image, 20.0, corners);
            [(0, 0), (99, 0), (99, 99), (0, 99)].map(|(x, y)| alpha(&image, x, y))
        };
        assert_eq!(only(Corners { top_left: true, ..Corners::NONE }), [0, 255, 255, 255]);
        assert_eq!(only(Corners { top_right: true, ..Corners::NONE }), [255, 0, 255, 255]);
        assert_eq!(only(Corners { bottom_right: true, ..Corners::NONE }), [255, 255, 0, 255]);
        assert_eq!(only(Corners { bottom_left: true, ..Corners::NONE }), [255, 255, 255, 0]);
        let diagonal = Corners { top_left: true, bottom_right: true, ..Corners::NONE };
        assert_eq!(only(diagonal), [0, 255, 0, 255]);
        assert_eq!(only(Corners::NONE), [255; 4]);
    }

    fn registry() -> Registry {
        let mut registry = Registry::new();
        registry.register(RoundCorners).unwrap();
        registry
    }

    fn picture(width: u32, height: u32) -> Item {
        let bytes = codec::encode(opaque(width, height), OutFormat::Png, 90, [0; 4]).unwrap();
        Item::bytes(bytes).named("dir/pic.JPG")
    }

    fn run(params: Params, item: Item) -> Result<Item> {
        let input = Input::single(item);
        let out = registry().run("image.round_corners", &input, &params, &Job::new())?;
        Ok(out.items.into_iter().next().unwrap())
    }

    fn decoded(item: &Item) -> RgbaImage {
        codec::decode(item.data.as_bytes()).unwrap().image
    }

    #[test]
    fn the_default_settings_are_valid() {
        let out = run(Params::new(), picture(100, 100)).unwrap();
        assert_eq!(out.name.as_deref(), Some("pic.png"));
        let image = decoded(&out);
        assert_eq!(alpha(&image, 0, 0), 0);
        assert_eq!(alpha(&image, 50, 50), 255);
    }

    #[test]
    fn the_tool_rounds_only_the_chosen_corners() {
        let params = Params::new()
            .with("top_left", false)
            .with("top_right", false)
            .with("bottom_left", false);
        let image = decoded(&run(params, picture(100, 100)).unwrap());
        assert_eq!(alpha(&image, 99, 99), 0);
        for (x, y) in [(0, 0), (99, 0), (0, 99)] {
            assert_eq!(alpha(&image, x, y), 255, "({x}, {y})");
        }
        let none = Params::new()
            .with("top_left", false)
            .with("top_right", false)
            .with("bottom_right", false)
            .with("bottom_left", false);
        assert!(matches!(run(none, picture(8, 8)), Err(Error::InvalidParam { .. })));
    }

    #[test]
    fn percent_is_taken_from_the_shorter_side() {
        // 50 % of 60 px is a radius of 30: the 200x60 picture gets round ends.
        let params = Params::new().with("unit", "percent").with("radius_percent", 50.0);
        let image = decoded(&run(params, picture(200, 60)).unwrap());
        assert_eq!(alpha(&image, 0, 0), 0);
        assert!(alpha(&image, 0, 30) > 200);
        assert_eq!(alpha(&image, 100, 0), 255);
        assert_eq!(alpha(&image, 100, 30), 255);
    }

    #[test]
    fn transparency_survives_in_other_formats() {
        for id in ["webp", "tiff"] {
            let out = run(Params::new().with("format", id), picture(64, 64)).unwrap();
            assert_eq!(alpha(&decoded(&out), 0, 0), 0, "{id}");
        }
    }

    #[test]
    fn bad_settings_are_reported() {
        for id in NO_ALPHA {
            assert!(run(Params::new().with("format", *id), picture(8, 8)).is_err(), "{id}");
        }
        let too_big = Params::new().with("unit", "percent").with("radius_percent", 60.0);
        assert!(run(too_big, picture(8, 8)).is_err());
        let wide = Params::new().with("format", "ico");
        let err = run(wide, picture(300, 10)).unwrap_err();
        assert!(err.to_string().starts_with("dir/pic.JPG: "), "{err}");
        let broken = Item::bytes(b"junk".to_vec()).named("broken.png");
        assert!(run(Params::new(), broken).is_err());
    }
}
