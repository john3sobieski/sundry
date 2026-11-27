//! `image.flip`: mirrors pictures left-right or top-bottom.

use image::RgbaImage;
use image::imageops;
use sundry_core::{
    Category, Data, Error, Input, InputKind, Item, Job, Media, Output, ParamSpec, Params, Result,
    Tool, ToolInfo,
};

use crate::codec::{self, Target};
use crate::map_items;

pub struct Flip;

const DIRECTIONS: &[&str] = &["horizontal", "vertical"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// Mirror left to right.
    Horizontal,
    /// Mirror top to bottom.
    Vertical,
}

/// Mirrors the picture; the pixels themselves are not changed.
pub fn flip(image: &RgbaImage, direction: Direction) -> RgbaImage {
    match direction {
        Direction::Horizontal => imageops::flip_horizontal(image),
        Direction::Vertical => imageops::flip_vertical(image),
    }
}

/// Settings checked and parsed once per run.
struct Settings {
    direction: Direction,
    target: Target,
}

impl Settings {
    fn from_params(params: &Params) -> Result<Self> {
        let direction = match params.get_text("direction")? {
            "horizontal" => Direction::Horizontal,
            "vertical" => Direction::Vertical,
            _ => {
                return Err(Error::InvalidParam {
                    name: "direction".to_owned(),
                    reason: "unknown".to_owned(),
                });
            }
        };
        Ok(Self { direction, target: Target::from_params(params)? })
    }

    fn process(&self, item: &Item) -> Result<Item> {
        let Data::Bytes(bytes) = &item.data else {
            return Err(Error::InvalidInput("expected an image file".to_owned()));
        };
        let decoded = codec::decode(bytes)?;
        let image = flip(&decoded.image, self.direction);
        self.target.encode(image, decoded.format, item.name.as_deref(), [0; 4])
    }
}

impl Tool for Flip {
    fn info(&self) -> ToolInfo {
        ToolInfo::new(
            "image.flip",
            Category::Image,
            "Flip image",
            "Mirrors images left-right or top-bottom, one file or a whole batch. Metadata is \
             dropped and animations keep only their first frame.",
        )
        .with_input(InputKind::Files)
        .with_media(Media::Image)
        .per_file()
    }

    fn params(&self) -> Vec<ParamSpec> {
        let mut specs = vec![ParamSpec::choice("direction", DIRECTIONS, "horizontal")];
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
    use image::Rgba;
    use sundry_core::registry::Registry;

    use super::*;
    use crate::codec::OutFormat;

    /// 3x2 picture whose six pixels all differ (red = column, green = row).
    fn grid() -> RgbaImage {
        RgbaImage::from_fn(3, 2, |x, y| Rgba([x as u8 * 10, y as u8 * 10, 5, 255]))
    }

    fn at(image: &RgbaImage, x: u32, y: u32) -> [u8; 2] {
        let p = image.get_pixel(x, y).0;
        [p[0], p[1]]
    }

    #[test]
    fn horizontal_mirrors_the_columns() {
        let out = flip(&grid(), Direction::Horizontal);
        assert_eq!(out.dimensions(), (3, 2));
        assert_eq!(at(&out, 0, 0), [20, 0]);
        assert_eq!(at(&out, 2, 0), [0, 0]);
        assert_eq!(at(&out, 1, 1), [10, 10]);
        assert_eq!(at(&out, 0, 1), [20, 10]);
    }

    #[test]
    fn vertical_mirrors_the_rows() {
        let out = flip(&grid(), Direction::Vertical);
        assert_eq!(at(&out, 0, 0), [0, 10]);
        assert_eq!(at(&out, 2, 1), [20, 0]);
        assert_eq!(at(&out, 1, 0), [10, 10]);
    }

    #[test]
    fn flipping_twice_gives_the_original() {
        for direction in [Direction::Horizontal, Direction::Vertical] {
            assert_eq!(flip(&flip(&grid(), direction), direction), grid());
        }
        let one = RgbaImage::from_pixel(1, 1, Rgba([1, 2, 3, 4]));
        assert_eq!(flip(&one, Direction::Horizontal), one);
    }

    #[test]
    fn alpha_moves_with_its_pixel() {
        let mut image = RgbaImage::from_pixel(4, 1, Rgba([9, 9, 9, 255]));
        image.put_pixel(0, 0, Rgba([9, 9, 9, 0]));
        let out = flip(&image, Direction::Horizontal);
        assert_eq!(out.get_pixel(3, 0).0[3], 0);
        assert_eq!(out.get_pixel(0, 0).0[3], 255);
    }

    fn registry() -> Registry {
        let mut registry = Registry::new();
        registry.register(Flip).unwrap();
        registry
    }

    fn picture() -> Item {
        let bytes = codec::encode(grid(), OutFormat::Png, 90, [0; 4]).unwrap();
        Item::bytes(bytes).named("dir/pic.JPG")
    }

    fn run(params: Params, item: Item) -> Result<Item> {
        let out = registry().run("image.flip", &Input::single(item), &params, &Job::new())?;
        Ok(out.items.into_iter().next().unwrap())
    }

    #[test]
    fn the_default_settings_are_valid() {
        let out = run(Params::new(), picture()).unwrap();
        assert_eq!(out.name.as_deref(), Some("pic.png"));
        let image = codec::decode(out.data.as_bytes()).unwrap().image;
        assert_eq!(image, flip(&grid(), Direction::Horizontal));
    }

    #[test]
    fn the_tool_flips_vertically_and_can_change_the_format() {
        let params = Params::new().with("direction", "vertical").with("format", "bmp");
        let out = run(params, picture()).unwrap();
        assert_eq!(out.name.as_deref(), Some("pic.bmp"));
        let image = codec::decode(out.data.as_bytes()).unwrap().image;
        assert_eq!(image, flip(&grid(), Direction::Vertical));
    }

    #[test]
    fn bad_settings_are_reported() {
        assert!(run(Params::new().with("direction", "diagonal"), picture()).is_err());
        let broken = Item::bytes(b"junk".to_vec()).named("broken.png");
        assert!(run(Params::new(), broken).is_err());
    }
}
