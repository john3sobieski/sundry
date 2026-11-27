//! `image.rotate`: turns pictures by 90/180/270 degrees (lossless) or by any angle.

use image::{Rgba, RgbaImage, imageops};
use sundry_core::{
    Category, Data, Error, Input, InputKind, Item, Job, Media, Output, ParamSpec, Params, Result,
    Tool, ToolInfo,
};

use crate::codec::{self, Target};
use crate::map_items;
use crate::plan::{self, check_size};

pub struct Rotate;

const ANGLES: &[&str] = &["90", "180", "270", "custom"];

/// A turn is exact when the angle is this close to a multiple of 90 degrees.
const QUARTER_EPSILON: f64 = 1e-9;

/// Turns the picture clockwise by `quarters` times 90 degrees; the pixels are only moved.
pub fn rotate_quarters(image: &RgbaImage, quarters: u8) -> RgbaImage {
    match quarters % 4 {
        1 => imageops::rotate90(image),
        2 => imageops::rotate180(image),
        3 => imageops::rotate270(image),
        _ => image.clone(),
    }
}

/// Premultiplied color: red, green and blue (0..255) already scaled by alpha, alpha 0..255.
fn premultiplied(pixel: [u8; 4]) -> [f32; 4] {
    let alpha = f32::from(pixel[3]) / 255.0;
    [
        f32::from(pixel[0]) * alpha,
        f32::from(pixel[1]) * alpha,
        f32::from(pixel[2]) * alpha,
        f32::from(pixel[3]),
    ]
}

fn unpremultiplied(value: [f32; 4]) -> [u8; 4] {
    let alpha = value[3].clamp(0.0, 255.0);
    if alpha < 0.5 {
        return [0; 4];
    }
    let scale = 255.0 / alpha;
    let channel = |c: f32| (c * scale).round().clamp(0.0, 255.0) as u8;
    [channel(value[0]), channel(value[1]), channel(value[2]), alpha.round() as u8]
}

/// Size of the canvas that holds the whole picture turned by an angle with this sine and cosine.
fn expanded_size(size: (u32, u32), sin: f64, cos: f64) -> (u32, u32) {
    let (w, h) = (f64::from(size.0), f64::from(size.1));
    let width = w * cos.abs() + h * sin.abs();
    let height = w * sin.abs() + h * cos.abs();
    // The small offset keeps float noise (for example 100.00000000000001) from adding a pixel.
    let side = |v: f64| ((v - 1e-6).ceil() as u32).max(1);
    (side(width), side(height))
}

/// Turns the picture clockwise by `degrees` (bilinear, smooth edge). Angles that are multiples
/// of 90 are done exactly. With `expand` the canvas grows to hold the whole picture, otherwise
/// the size stays and the corners are cut. Uncovered areas get `fill`.
pub fn rotate_by(
    image: &RgbaImage,
    degrees: f64,
    expand: bool,
    fill: [u8; 4],
    job: &Job,
) -> Result<RgbaImage> {
    let turns = degrees.rem_euclid(360.0) / 90.0;
    if (turns - turns.round()).abs() < QUARTER_EPSILON {
        return Ok(rotate_quarters(image, (turns.round() as u8) % 4));
    }
    let (w, h) = image.dimensions();
    let (sin, cos) = degrees.to_radians().sin_cos();
    let (out_w, out_h) = if expand { expanded_size((w, h), sin, cos) } else { (w, h) };
    check_size((out_w, out_h))?;

    let fill = premultiplied(fill);
    let pixel = |x: i64, y: i64| -> [f32; 4] {
        if x < 0 || y < 0 || x >= i64::from(w) || y >= i64::from(h) {
            fill
        } else {
            premultiplied(image.get_pixel(x as u32, y as u32).0)
        }
    };
    let (in_cx, in_cy) = (f64::from(w) / 2.0, f64::from(h) / 2.0);
    let (out_cx, out_cy) = (f64::from(out_w) / 2.0, f64::from(out_h) / 2.0);
    let mut out = RgbaImage::new(out_w, out_h);
    for y in 0..out_h {
        job.check_cancelled()?;
        let ey = f64::from(y) + 0.5 - out_cy;
        for x in 0..out_w {
            let ex = f64::from(x) + 0.5 - out_cx;
            // Where this output pixel comes from (the inverse turn), as a pixel-corner position.
            let u = ex * cos + ey * sin + in_cx - 0.5;
            let v = -ex * sin + ey * cos + in_cy - 0.5;
            let (x0, y0) = (u.floor(), v.floor());
            let (fx, fy) = ((u - x0) as f32, (v - y0) as f32);
            let (x0, y0) = (x0 as i64, y0 as i64);
            let (p00, p10) = (pixel(x0, y0), pixel(x0 + 1, y0));
            let (p01, p11) = (pixel(x0, y0 + 1), pixel(x0 + 1, y0 + 1));
            let mixed: [f32; 4] = std::array::from_fn(|c| {
                let upper = p00[c] * (1.0 - fx) + p10[c] * fx;
                let lower = p01[c] * (1.0 - fx) + p11[c] * fx;
                upper * (1.0 - fy) + lower * fy
            });
            out.put_pixel(x, y, Rgba(unpremultiplied(mixed)));
        }
    }
    Ok(out)
}

#[derive(Debug, Clone, Copy)]
enum Turn {
    Quarters(u8),
    Custom { degrees: f64, expand: bool, fill: [u8; 4] },
}

/// Settings checked and parsed once per run.
struct Settings {
    turn: Turn,
    target: Target,
}

impl Settings {
    fn from_params(params: &Params) -> Result<Self> {
        let turn = match params.get_text("angle")? {
            "90" => Turn::Quarters(1),
            "180" => Turn::Quarters(2),
            "270" => Turn::Quarters(3),
            "custom" => Turn::Custom {
                degrees: params.get_float("degrees")?,
                expand: params.get_bool("expand")?,
                fill: plan::parse_color(params.get_text("background")?)?,
            },
            _ => {
                return Err(Error::InvalidParam {
                    name: "angle".to_owned(),
                    reason: "unknown".to_owned(),
                });
            }
        };
        Ok(Self { turn, target: Target::from_params(params)? })
    }

    fn process(&self, item: &Item, job: &Job) -> Result<Item> {
        let Data::Bytes(bytes) = &item.data else {
            return Err(Error::InvalidInput("expected an image file".to_owned()));
        };
        let decoded = codec::decode(bytes)?;
        let (image, background) = match self.turn {
            Turn::Quarters(n) => (rotate_quarters(&decoded.image, n), [0; 4]),
            Turn::Custom { degrees, expand, fill } => {
                (rotate_by(&decoded.image, degrees, expand, fill, job)?, fill)
            }
        };
        self.target.encode(image, decoded.format, item.name.as_deref(), background)
    }
}

impl Tool for Rotate {
    fn info(&self) -> ToolInfo {
        ToolInfo::new(
            "image.rotate",
            Category::Image,
            "Rotate image",
            "Turns images by 90, 180 or 270 degrees without any loss, or by a custom angle with \
             a smooth edge. One file or a whole batch. Metadata is dropped and animations keep \
             only their first frame.",
        )
        .with_input(InputKind::Files)
        .with_media(Media::Image)
        .per_file()
    }

    fn params(&self) -> Vec<ParamSpec> {
        let mut specs = vec![
            ParamSpec::choice("angle", ANGLES, "90"),
            ParamSpec::float_range("degrees", 15.0, -360.0, 360.0).show_when("angle", &["custom"]),
            ParamSpec::boolean("expand", true).show_when("angle", &["custom"]),
            ParamSpec::text("background", "transparent").show_when("angle", &["custom"]),
        ];
        specs.extend(Target::specs());
        specs
    }

    fn run(&self, input: &Input, params: &Params, job: &Job) -> Result<Output> {
        let settings = Settings::from_params(params)?;
        map_items(input, job, |item| settings.process(item, job))
    }
}

#[cfg(test)]
mod tests {
    use sundry_core::registry::Registry;

    use super::*;
    use crate::codec::OutFormat;

    const RED: Rgba<u8> = Rgba([255, 0, 0, 255]);
    const BLUE: Rgba<u8> = Rgba([0, 0, 255, 255]);

    /// Red left half, blue right half.
    fn halves(width: u32, height: u32) -> RgbaImage {
        RgbaImage::from_fn(width, height, |x, _| if x < width / 2 { RED } else { BLUE })
    }

    fn solid(width: u32, height: u32) -> RgbaImage {
        RgbaImage::from_pixel(width, height, Rgba([10, 120, 200, 255]))
    }

    #[test]
    fn quarter_turns_go_clockwise() {
        let image = RgbaImage::from_fn(2, 1, |x, _| if x == 0 { RED } else { BLUE });
        // The left pixel goes to the top, the right one to the bottom.
        let cw = rotate_quarters(&image, 1);
        assert_eq!(cw.dimensions(), (1, 2));
        assert_eq!((*cw.get_pixel(0, 0), *cw.get_pixel(0, 1)), (RED, BLUE));
        let half = rotate_quarters(&image, 2);
        assert_eq!((*half.get_pixel(0, 0), *half.get_pixel(1, 0)), (BLUE, RED));
        let ccw = rotate_quarters(&image, 3);
        assert_eq!((*ccw.get_pixel(0, 0), *ccw.get_pixel(0, 1)), (BLUE, RED));
        assert_eq!(rotate_quarters(&image, 4), image);
        assert_eq!(rotate_quarters(&image, 0), image);
    }

    #[test]
    fn multiples_of_90_are_exact_for_any_custom_angle() {
        let image = halves(7, 4);
        let job = Job::new();
        for (degrees, quarters) in [(90.0, 1), (-90.0, 3), (450.0, 1), (180.0, 2), (-270.0, 1)] {
            let out = rotate_by(&image, degrees, true, [0; 4], &job).unwrap();
            assert_eq!(out, rotate_quarters(&image, quarters), "{degrees}");
        }
        assert_eq!(rotate_by(&image, 0.0, true, [0; 4], &job).unwrap(), image);
        assert_eq!(rotate_by(&image, 360.0, false, [0; 4], &job).unwrap(), image);
    }

    #[test]
    fn a_custom_angle_expands_the_canvas_or_keeps_the_size() {
        let job = Job::new();
        let image = solid(100, 100);
        let grown = rotate_by(&image, 45.0, true, [0; 4], &job).unwrap();
        assert_eq!(grown.dimensions(), (142, 142));
        assert_eq!(grown.get_pixel(71, 71).0, [10, 120, 200, 255]);
        assert_eq!(grown.get_pixel(0, 0).0[3], 0);
        assert!(grown.pixels().any(|p| p.0[3] > 0 && p.0[3] < 255), "the edge is smooth");

        let same = rotate_by(&image, 45.0, false, [0; 4], &job).unwrap();
        assert_eq!(same.dimensions(), (100, 100));
        assert_eq!(same.get_pixel(50, 50).0, [10, 120, 200, 255]);
        assert_eq!(same.get_pixel(0, 0).0[3], 0);
        // Corners are cut, so the middle of each side is still covered.
        assert_eq!(same.get_pixel(50, 1).0[3], 255);
    }

    #[test]
    fn a_custom_angle_turns_clockwise() {
        let out = rotate_by(&halves(100, 100), 45.0, true, [0; 4], &Job::new()).unwrap();
        assert_eq!(out.dimensions(), (142, 142));
        // The red left half moves to the upper left, the blue right half to the lower right.
        assert_eq!(*out.get_pixel(51, 66), RED);
        assert_eq!(*out.get_pixel(91, 76), BLUE);
        let back = rotate_by(&halves(100, 100), -45.0, true, [0; 4], &Job::new()).unwrap();
        assert_eq!(*back.get_pixel(51, 76), RED);
    }

    #[test]
    fn the_fill_color_covers_the_new_corners() {
        let fill = [255, 255, 0, 255];
        let out = rotate_by(&solid(60, 60), 30.0, true, fill, &Job::new()).unwrap();
        assert_eq!(out.get_pixel(0, 0).0, fill);
        assert!(out.pixels().all(|p| p.0[3] == 255));
    }

    #[test]
    fn transparency_is_kept_while_turning() {
        let image = RgbaImage::from_pixel(40, 40, Rgba([1, 2, 3, 100]));
        let out = rotate_by(&image, 20.0, true, [0; 4], &Job::new()).unwrap();
        let (cx, cy) = (out.width() / 2, out.height() / 2);
        assert_eq!(out.get_pixel(cx, cy).0, [1, 2, 3, 100]);
    }

    #[test]
    fn huge_results_and_tiny_pictures_are_handled() {
        let job = Job::new();
        let wide = RgbaImage::new(30_000, 1);
        assert!(rotate_by(&wide, 45.0, true, [0; 4], &job).is_err());
        for (w, h) in [(1, 1), (1, 9), (9, 1), (3, 2)] {
            for degrees in [1.0, 30.0, 45.0, 123.4, -77.0] {
                let out = rotate_by(&solid(w, h), degrees, true, [0; 4], &job).unwrap();
                assert!(out.width() >= 1 && out.height() >= 1);
            }
        }
    }

    #[test]
    fn a_cancelled_job_stops_the_turn() {
        let job = Job::new();
        job.cancel();
        assert!(matches!(
            rotate_by(&solid(20, 20), 30.0, true, [0; 4], &job),
            Err(Error::Cancelled)
        ));
    }

    fn registry() -> Registry {
        let mut registry = Registry::new();
        registry.register(Rotate).unwrap();
        registry
    }

    fn picture(width: u32, height: u32) -> Item {
        let bytes = codec::encode(halves(width, height), OutFormat::Png, 90, [0; 4]).unwrap();
        Item::bytes(bytes).named("dir/pic.JPG")
    }

    fn run(params: Params, item: Item) -> Result<Item> {
        let out = registry().run("image.rotate", &Input::single(item), &params, &Job::new())?;
        Ok(out.items.into_iter().next().unwrap())
    }

    fn decoded(item: &Item) -> RgbaImage {
        codec::decode(item.data.as_bytes()).unwrap().image
    }

    #[test]
    fn the_default_settings_are_valid() {
        let out = run(Params::new(), picture(40, 20)).unwrap();
        assert_eq!(out.name.as_deref(), Some("pic.png"));
        let image = decoded(&out);
        assert_eq!(image, rotate_quarters(&halves(40, 20), 1));
        assert_eq!(image.dimensions(), (20, 40));
    }

    #[test]
    fn the_preset_angles_work() {
        for (id, quarters) in [("90", 1), ("180", 2), ("270", 3)] {
            let out = run(Params::new().with("angle", id), picture(30, 10)).unwrap();
            assert_eq!(decoded(&out), rotate_quarters(&halves(30, 10), quarters), "{id}");
        }
    }

    #[test]
    fn the_tool_turns_by_a_custom_angle() {
        let params = Params::new().with("angle", "custom").with("degrees", 45.0);
        let image = decoded(&run(params, picture(100, 100)).unwrap());
        assert_eq!(image.dimensions(), (142, 142));
        assert_eq!(image.get_pixel(0, 0).0[3], 0);

        let jpeg = Params::new()
            .with("angle", "custom")
            .with("degrees", 45.0)
            .with("format", "jpeg")
            .with("background", "#FF0000");
        let out = run(jpeg, picture(100, 100)).unwrap();
        assert_eq!(out.name.as_deref(), Some("pic.jpg"));
        let corner = decoded(&out).get_pixel(0, 0).0;
        assert!(corner[0] > 240 && corner[1] < 15 && corner[2] < 15, "{corner:?}");
    }

    #[test]
    fn a_hidden_bad_background_does_not_block_a_preset_angle() {
        let params = Params::new().with("angle", "180").with("background", "no-such-color");
        assert!(run(params, picture(8, 8)).is_ok());
    }

    #[test]
    fn bad_settings_are_reported() {
        let bad_color = Params::new().with("angle", "custom").with("background", "no-such-color");
        assert!(run(bad_color, picture(8, 8)).is_err());
        assert!(run(Params::new().with("angle", "45"), picture(8, 8)).is_err());
        assert!(
            run(Params::new().with("angle", "custom").with("degrees", 400.0), picture(8, 8))
                .is_err()
        );
        let broken = Item::bytes(b"junk".to_vec()).named("broken.png");
        assert!(run(Params::new(), broken).is_err());
    }
}
