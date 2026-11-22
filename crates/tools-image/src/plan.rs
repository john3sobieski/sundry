//! Geometry of a resize: from the source size and the settings to the output size.

use sundry_core::{Error, Result};

/// Largest allowed side of a result, in pixels.
pub const MAX_SIDE: u32 = 32_768;
/// Largest allowed area of a result (RGBA needs 4 bytes per pixel in memory).
pub const MAX_PIXELS: u64 = 120_000_000;

/// How a picture is fitted into a box of another proportion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fit {
    /// Fill the box exactly; the picture is distorted.
    Stretch,
    /// Scale to fit inside the box; the result may be smaller than the box on one side.
    Inside,
    /// Scale to fit inside the box and pad the rest: the result is exactly the box.
    Contain,
    /// Scale to fill the box and crop the overflow from the center: exactly the box.
    Cover,
}

impl Fit {
    pub fn parse(id: &str) -> Option<Self> {
        match id {
            "stretch" => Some(Fit::Stretch),
            "inside" => Some(Fit::Inside),
            "contain" => Some(Fit::Contain),
            "cover" => Some(Fit::Cover),
            _ => None,
        }
    }
}

/// What the user asked for.
#[derive(Debug, Clone, PartialEq)]
pub enum Mode {
    Percent(f64),
    LongSide(u32),
    ShortSide(u32),
    Size { width: u32, height: u32, fit: Fit },
    /// A box with the given width whose height follows from `ratio` (width / height).
    Ratio { ratio: f64, width: u32, fit: Fit },
}

/// How to produce the result from the source picture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Plan {
    /// Size of the final picture.
    pub out: (u32, u32),
    /// Size the (possibly cropped) source is scaled to; smaller than `out` when padded.
    pub inner: (u32, u32),
    /// Crop the source around its center to the proportion of `out` first.
    pub crop: bool,
}

fn dim(value: f64) -> u32 {
    value.round().clamp(1.0, f64::from(u32::MAX)) as u32
}

fn scaled(src: (u32, u32), scale: f64) -> (u32, u32) {
    (dim(f64::from(src.0) * scale), dim(f64::from(src.1) * scale))
}

fn same(out: (u32, u32)) -> Plan {
    Plan { out, inner: out, crop: false }
}

/// Plans one resize. `no_upscale` keeps proportional modes from enlarging a picture.
pub fn plan(mode: &Mode, src: (u32, u32), no_upscale: bool) -> Result<Plan> {
    let (sw, sh) = (f64::from(src.0), f64::from(src.1));
    if src.0 == 0 || src.1 == 0 {
        return Err(Error::InvalidInput("the image has no pixels".to_owned()));
    }
    let limit = |scale: f64| if no_upscale { scale.min(1.0) } else { scale };
    let plan = match *mode {
        Mode::Percent(percent) => same(scaled(src, limit(percent / 100.0))),
        Mode::LongSide(side) => same(scaled(src, limit(f64::from(side) / sw.max(sh)))),
        Mode::ShortSide(side) => same(scaled(src, limit(f64::from(side) / sw.min(sh)))),
        Mode::Size { width, height, fit } => boxed(src, (width, height), fit, no_upscale),
        Mode::Ratio { ratio, width, fit } => {
            let height = dim(f64::from(width) / ratio);
            boxed(src, (width, height), fit, no_upscale)
        }
    };
    check_size(plan.out)?;
    Ok(plan)
}

fn boxed(src: (u32, u32), (width, height): (u32, u32), fit: Fit, no_upscale: bool) -> Plan {
    let (sw, sh) = (f64::from(src.0), f64::from(src.1));
    let (bw, bh) = (f64::from(width), f64::from(height));
    let fit_scale = (bw / sw).min(bh / sh);
    match fit {
        Fit::Stretch => same((width, height)),
        Fit::Inside => same(scaled(src, if no_upscale { fit_scale.min(1.0) } else { fit_scale })),
        Fit::Contain => {
            let inner = scaled(src, fit_scale);
            let inner = (inner.0.min(width), inner.1.min(height));
            Plan { out: (width, height), inner, crop: false }
        }
        Fit::Cover => Plan { out: (width, height), inner: (width, height), crop: true },
    }
}

fn check_size(out: (u32, u32)) -> Result<()> {
    if out.0 > MAX_SIDE || out.1 > MAX_SIDE || u64::from(out.0) * u64::from(out.1) > MAX_PIXELS {
        return Err(Error::InvalidInput(format!(
            "the result would be too large ({}x{} pixels)",
            out.0, out.1
        )));
    }
    Ok(())
}

/// Parses an aspect ratio typed by hand: `16:9`, `16/9`, `16x9`, `1.7778` or `1,5`.
pub fn parse_ratio(text: &str) -> Result<f64> {
    let bad =
        |reason: &str| Error::InvalidParam { name: "ratio".to_owned(), reason: reason.to_owned() };
    let number = |part: &str| {
        part.trim().replace(',', ".").parse::<f64>().ok().filter(|v| v.is_finite() && *v > 0.0)
    };
    let text = text.trim();
    let split = text.find([':', '/', 'x', 'X', '\u{d7}']);
    let ratio = match split {
        Some(at) => {
            let sep_len = text[at..].chars().next().map_or(1, char::len_utf8);
            match (number(&text[..at]), number(&text[at + sep_len..])) {
                (Some(w), Some(h)) => w / h,
                _ => return Err(bad("expected two positive numbers, like 16:9")),
            }
        }
        None => number(text).ok_or_else(|| bad("expected a ratio like 16:9 or 1.5"))?,
    };
    if !(0.01..=100.0).contains(&ratio) {
        return Err(bad("the ratio must be between 1:100 and 100:1"));
    }
    Ok(ratio)
}

/// Parses a color typed by hand into RGBA: `transparent`, `white`, `black`,
/// `#RGB`, `#RRGGBB` or `#RRGGBBAA` (the `#` is optional).
pub fn parse_color(text: &str) -> Result<[u8; 4]> {
    let bad = || Error::InvalidParam {
        name: "background".to_owned(),
        reason: "expected transparent, white, black or a hex color like #1A2B3C".to_owned(),
    };
    let t = text.trim().to_lowercase();
    match t.as_str() {
        "" | "transparent" | "none" => return Ok([0, 0, 0, 0]),
        "white" => return Ok([255, 255, 255, 255]),
        "black" => return Ok([0, 0, 0, 255]),
        _ => {}
    }
    let hex = t.strip_prefix('#').unwrap_or(&t);
    if !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(bad());
    }
    let digit = |i: usize| u8::from_str_radix(&hex[i..=i], 16).map_err(|_| bad());
    let pair = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).map_err(|_| bad());
    match hex.len() {
        3 => Ok([digit(0)? * 17, digit(1)? * 17, digit(2)? * 17, 255]),
        6 => Ok([pair(0)?, pair(2)?, pair(4)?, 255]),
        8 => Ok([pair(0)?, pair(2)?, pair(4)?, pair(6)?]),
        _ => Err(bad()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SRC: (u32, u32) = (2440, 1920);

    fn size(fit: Fit) -> Mode {
        Mode::Size { width: 1920, height: 1080, fit }
    }

    #[test]
    fn size_modes_for_a_different_proportion() {
        let p = plan(&size(Fit::Stretch), SRC, false).unwrap();
        assert_eq!((p.out, p.inner, p.crop), ((1920, 1080), (1920, 1080), false));
        let p = plan(&size(Fit::Inside), SRC, false).unwrap();
        assert_eq!((p.out, p.crop), ((1373, 1080), false));
        let p = plan(&size(Fit::Contain), SRC, false).unwrap();
        assert_eq!((p.out, p.inner, p.crop), ((1920, 1080), (1373, 1080), false));
        let p = plan(&size(Fit::Cover), SRC, false).unwrap();
        assert_eq!((p.out, p.inner, p.crop), ((1920, 1080), (1920, 1080), true));
    }

    #[test]
    fn proportional_modes() {
        assert_eq!(plan(&Mode::Percent(50.0), SRC, false).unwrap().out, (1220, 960));
        assert_eq!(plan(&Mode::LongSide(1000), SRC, false).unwrap().out, (1000, 787));
        assert_eq!(plan(&Mode::ShortSide(960), SRC, false).unwrap().out, (1220, 960));
        assert_eq!(plan(&Mode::Percent(0.01), (10, 10), false).unwrap().out, (1, 1));
    }

    #[test]
    fn never_enlarge_only_limits_the_scale() {
        assert_eq!(plan(&Mode::Percent(200.0), (100, 50), true).unwrap().out, (100, 50));
        assert_eq!(plan(&Mode::Percent(200.0), (100, 50), false).unwrap().out, (200, 100));
        assert_eq!(plan(&Mode::LongSide(4000), (100, 50), true).unwrap().out, (100, 50));
        let inside = Mode::Size { width: 500, height: 500, fit: Fit::Inside };
        assert_eq!(plan(&inside, (100, 50), true).unwrap().out, (100, 50));
        assert_eq!(plan(&inside, (100, 50), false).unwrap().out, (500, 250));
        // An exact box stays exact.
        assert_eq!(plan(&size(Fit::Cover), (100, 50), true).unwrap().out, (1920, 1080));
    }

    #[test]
    fn ratio_mode_derives_the_height() {
        let mode = Mode::Ratio { ratio: 16.0 / 9.0, width: 1280, fit: Fit::Cover };
        assert_eq!(plan(&mode, SRC, false).unwrap().out, (1280, 720));
        let mode = Mode::Ratio { ratio: 1.0, width: 512, fit: Fit::Contain };
        let p = plan(&mode, (1000, 500), false).unwrap();
        assert_eq!((p.out, p.inner), ((512, 512), (512, 256)));
    }

    #[test]
    fn huge_results_are_refused() {
        assert!(plan(&Mode::Percent(1000.0), (30_000, 30_000), false).is_err());
        let wide = Mode::Size { width: 32_768, height: 32_768, fit: Fit::Stretch };
        assert!(plan(&wide, SRC, false).is_err());
        assert!(plan(&Mode::Percent(10.0), (0, 5), false).is_err());
    }

    #[test]
    fn ratios_are_parsed_leniently() {
        let close = |a: f64, b: f64| (a - b).abs() < 1e-9;
        assert!(close(parse_ratio("16:9").unwrap(), 16.0 / 9.0));
        assert!(close(parse_ratio(" 4 / 3 ").unwrap(), 4.0 / 3.0));
        assert!(close(parse_ratio("21x9").unwrap(), 21.0 / 9.0));
        assert!(close(parse_ratio("1,5").unwrap(), 1.5));
        assert!(close(parse_ratio("1\u{d7}1").unwrap(), 1.0));
        for bad in ["", "abc", "16:", ":9", "0:1", "-1:1", "1:1000", "1000:1", "16:9:1"] {
            assert!(parse_ratio(bad).is_err(), "accepted `{bad}`");
        }
    }

    #[test]
    fn colors_are_parsed() {
        assert_eq!(parse_color("transparent").unwrap(), [0, 0, 0, 0]);
        assert_eq!(parse_color("").unwrap(), [0, 0, 0, 0]);
        assert_eq!(parse_color("White").unwrap(), [255, 255, 255, 255]);
        assert_eq!(parse_color("#1a2B3c").unwrap(), [0x1a, 0x2b, 0x3c, 255]);
        assert_eq!(parse_color("fa0").unwrap(), [0xff, 0xaa, 0x00, 255]);
        assert_eq!(parse_color("#11223380").unwrap(), [0x11, 0x22, 0x33, 0x80]);
        for bad in ["#12", "#12345", "red", "#gg0000", "#1234567890"] {
            assert!(parse_color(bad).is_err(), "accepted `{bad}`");
        }
    }
}
