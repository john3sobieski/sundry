//! Decoding and encoding of raster images (all through 8-bit RGBA in memory).

use std::io::Cursor;

use image::codecs::jpeg::JpegEncoder;
use image::metadata::Orientation;
use image::{
    DynamicImage, ExtendedColorType, ImageDecoder, ImageEncoder, ImageError, ImageFormat,
    ImageReader, Limits, Rgb, RgbImage, RgbaImage,
};
use sundry_core::{Error, Result};

/// File extensions the picker and drag-and-drop accept (lowercase, no dot).
/// AVIF is listed so that such files show a clear error instead of vanishing.
pub const INPUT_EXTENSIONS: &[&str] = &[
    "png", "jpg", "jpeg", "jpe", "jfif", "webp", "gif", "bmp", "tif", "tiff", "ico", "tga", "pbm",
    "pgm", "ppm", "pam", "pnm", "qoi", "avif",
];

/// Largest side of a decoded picture, in pixels.
const MAX_INPUT_SIDE: u32 = 32_768;
/// Memory a single decode may allocate.
const MAX_INPUT_ALLOC: u64 = 1 << 30;

/// A decoded picture: upright (EXIF orientation applied), 8-bit RGBA.
pub struct Decoded {
    pub image: RgbaImage,
    pub format: Option<ImageFormat>,
}

/// Formats a result can be written in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutFormat {
    Png,
    Jpeg,
    WebP,
    #[cfg(feature = "avif")]
    Avif,
    Bmp,
    Gif,
    Tiff,
    Ico,
}

impl OutFormat {
    /// Option ids of the `format` parameter (without `keep`).
    pub const IDS: &'static [&'static str] = &[
        "png",
        "jpeg",
        "webp",
        #[cfg(feature = "avif")]
        "avif",
        "bmp",
        "gif",
        "tiff",
        "ico",
    ];

    pub fn parse(id: &str) -> Option<Self> {
        match id {
            "png" => Some(OutFormat::Png),
            "jpeg" => Some(OutFormat::Jpeg),
            "webp" => Some(OutFormat::WebP),
            #[cfg(feature = "avif")]
            "avif" => Some(OutFormat::Avif),
            "bmp" => Some(OutFormat::Bmp),
            "gif" => Some(OutFormat::Gif),
            "tiff" => Some(OutFormat::Tiff),
            "ico" => Some(OutFormat::Ico),
            _ => None,
        }
    }

    /// The same format as a decoded file, if it can be written back.
    pub fn from_image_format(format: ImageFormat) -> Option<Self> {
        match format {
            ImageFormat::Png => Some(OutFormat::Png),
            ImageFormat::Jpeg => Some(OutFormat::Jpeg),
            ImageFormat::WebP => Some(OutFormat::WebP),
            ImageFormat::Bmp => Some(OutFormat::Bmp),
            ImageFormat::Gif => Some(OutFormat::Gif),
            ImageFormat::Tiff => Some(OutFormat::Tiff),
            ImageFormat::Ico => Some(OutFormat::Ico),
            _ => None,
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            OutFormat::Png => "png",
            OutFormat::Jpeg => "jpg",
            OutFormat::WebP => "webp",
            #[cfg(feature = "avif")]
            OutFormat::Avif => "avif",
            OutFormat::Bmp => "bmp",
            OutFormat::Gif => "gif",
            OutFormat::Tiff => "tiff",
            OutFormat::Ico => "ico",
        }
    }
}

/// Short display name of a decoded format.
pub fn format_name(format: Option<ImageFormat>) -> &'static str {
    match format {
        Some(ImageFormat::Png) => "PNG",
        Some(ImageFormat::Jpeg) => "JPEG",
        Some(ImageFormat::WebP) => "WebP",
        Some(ImageFormat::Gif) => "GIF",
        Some(ImageFormat::Bmp) => "BMP",
        Some(ImageFormat::Tiff) => "TIFF",
        Some(ImageFormat::Ico) => "ICO",
        Some(ImageFormat::Avif) => "AVIF",
        Some(ImageFormat::Tga) => "TGA",
        Some(ImageFormat::Pnm) => "PNM",
        Some(ImageFormat::Qoi) => "QOI",
        _ => "image",
    }
}

fn limits() -> Limits {
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_INPUT_SIDE);
    limits.max_image_height = Some(MAX_INPUT_SIDE);
    limits.max_alloc = Some(MAX_INPUT_ALLOC);
    limits
}

fn decode_error(err: ImageError) -> Error {
    match err {
        ImageError::Unsupported(_) => {
            Error::Unsupported("this image format or variant is not supported".to_owned())
        }
        ImageError::Limits(_) => Error::InvalidInput("the image is too large".to_owned()),
        other => Error::InvalidInput(format!("cannot decode the image: {other}")),
    }
}

/// Decodes any supported file. Animated images give their first frame.
pub fn decode(bytes: &[u8]) -> Result<Decoded> {
    let mut reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| Error::InvalidInput(format!("cannot read the file: {e}")))?;
    let format = reader.format();
    match format {
        None => return Err(Error::Unsupported("unknown image format".to_owned())),
        Some(ImageFormat::Avif) => {
            return Err(Error::Unsupported("AVIF input is not supported yet".to_owned()));
        }
        Some(_) => {}
    }
    reader.limits(limits());
    let mut decoder = reader.into_decoder().map_err(decode_error)?;
    let orientation = decoder.orientation().unwrap_or(Orientation::NoTransforms);
    let mut image = DynamicImage::from_decoder(decoder).map_err(decode_error)?;
    image.apply_orientation(orientation);
    Ok(Decoded { image: image.into_rgba8(), format })
}

/// Pixel size and format of a file, read from its header (EXIF rotation is not applied).
pub fn probe(bytes: &[u8]) -> Option<(u32, u32, Option<ImageFormat>)> {
    let reader = ImageReader::new(Cursor::new(bytes)).with_guessed_format().ok()?;
    let format = reader.format();
    let (width, height) = reader.into_dimensions().ok()?;
    Some((width, height, format))
}

fn is_opaque(image: &RgbaImage) -> bool {
    image.pixels().all(|p| p.0[3] == 255)
}

/// Draws the picture over `background` (white when that is not opaque).
fn flatten(image: &RgbaImage, background: [u8; 4]) -> RgbImage {
    let opaque_bg = background[3] == 255;
    let base = if opaque_bg { [background[0], background[1], background[2]] } else { [255; 3] };
    let mut out = RgbImage::new(image.width(), image.height());
    for (src, dst) in image.pixels().zip(out.pixels_mut()) {
        let alpha = u32::from(src.0[3]);
        let mut rgb = [0u8; 3];
        for c in 0..3 {
            let mixed = u32::from(src.0[c]) * alpha + u32::from(base[c]) * (255 - alpha);
            rgb[c] = ((mixed + 127) / 255) as u8;
        }
        *dst = Rgb(rgb);
    }
    out
}

fn encode_error(err: ImageError) -> Error {
    Error::Processing(format!("cannot encode the image: {err}"))
}

/// Encodes `image`. `quality` (1-100) is used by JPEG and AVIF; `background` is
/// what transparency becomes in formats without alpha (JPEG).
pub fn encode(
    image: RgbaImage,
    format: OutFormat,
    quality: u8,
    background: [u8; 4],
) -> Result<Vec<u8>> {
    let (width, height) = image.dimensions();
    if format == OutFormat::Ico && (width > 256 || height > 256) {
        return Err(Error::InvalidInput("ICO images can be at most 256x256 pixels".to_owned()));
    }
    let opaque = is_opaque(&image);
    let quality = quality.clamp(1, 100);
    let mut out = Vec::new();
    match format {
        OutFormat::Jpeg => {
            let rgb = flatten(&image, background);
            JpegEncoder::new_with_quality(&mut out, quality)
                .write_image(rgb.as_raw(), width, height, ExtendedColorType::Rgb8)
                .map_err(encode_error)?;
        }
        #[cfg(feature = "avif")]
        OutFormat::Avif => {
            let encoder =
                image::codecs::avif::AvifEncoder::new_with_speed_quality(&mut out, 7, quality);
            if opaque {
                let rgb = DynamicImage::ImageRgba8(image).into_rgb8();
                encoder
                    .write_image(rgb.as_raw(), width, height, ExtendedColorType::Rgb8)
                    .map_err(encode_error)?;
            } else {
                encoder
                    .write_image(image.as_raw(), width, height, ExtendedColorType::Rgba8)
                    .map_err(encode_error)?;
            }
        }
        other => {
            let target = match other {
                OutFormat::Png => ImageFormat::Png,
                OutFormat::WebP => ImageFormat::WebP,
                OutFormat::Bmp => ImageFormat::Bmp,
                OutFormat::Gif => ImageFormat::Gif,
                OutFormat::Tiff => ImageFormat::Tiff,
                _ => ImageFormat::Ico,
            };
            // GIF is always written from RGBA; the others drop an unused alpha channel.
            let dynamic = if opaque && other != OutFormat::Gif {
                DynamicImage::ImageRgb8(DynamicImage::ImageRgba8(image).into_rgb8())
            } else {
                DynamicImage::ImageRgba8(image)
            };
            let mut cursor = Cursor::new(Vec::new());
            dynamic.write_to(&mut cursor, target).map_err(encode_error)?;
            out = cursor.into_inner();
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use image::Rgba;

    use super::*;

    fn gradient(width: u32, height: u32, alpha: u8) -> RgbaImage {
        RgbaImage::from_fn(width, height, |x, y| Rgba([(x * 7) as u8, (y * 5) as u8, 90, alpha]))
    }

    const ALL: &[OutFormat] = &[
        OutFormat::Png,
        OutFormat::Jpeg,
        OutFormat::WebP,
        OutFormat::Bmp,
        OutFormat::Gif,
        OutFormat::Tiff,
        OutFormat::Ico,
    ];

    #[test]
    fn every_output_format_decodes_back() {
        for &format in ALL {
            for alpha in [255, 128] {
                let bytes = encode(gradient(40, 30, alpha), format, 80, [0; 4]).unwrap();
                let back = decode(&bytes).unwrap_or_else(|e| panic!("{format:?}: {e}"));
                assert_eq!(back.image.dimensions(), (40, 30), "{format:?}");
                let (w, h, _) = probe(&bytes).unwrap();
                assert_eq!((w, h), (40, 30), "{format:?}");
            }
        }
    }

    #[test]
    fn lossless_formats_keep_pixels() {
        let source = gradient(16, 16, 200);
        for format in [OutFormat::Png, OutFormat::WebP, OutFormat::Tiff] {
            let bytes = encode(source.clone(), format, 80, [0; 4]).unwrap();
            let back = decode(&bytes).unwrap().image;
            assert_eq!(back, source, "{format:?}");
        }
    }

    #[test]
    fn jpeg_flattens_transparency_onto_the_background() {
        let clear = RgbaImage::from_pixel(8, 8, Rgba([10, 20, 30, 0]));
        let white = decode(&encode(clear.clone(), OutFormat::Jpeg, 95, [0; 4]).unwrap()).unwrap();
        assert!(white.image.pixels().all(|p| p.0[..3].iter().all(|&c| c > 240)));
        let red = decode(&encode(clear, OutFormat::Jpeg, 95, [255, 0, 0, 255]).unwrap()).unwrap();
        let p = red.image.get_pixel(4, 4).0;
        assert!(p[0] > 240 && p[1] < 15 && p[2] < 15, "{p:?}");
    }

    #[test]
    fn ico_is_limited_to_256() {
        assert!(encode(gradient(257, 10, 255), OutFormat::Ico, 80, [0; 4]).is_err());
        assert!(encode(gradient(256, 256, 255), OutFormat::Ico, 80, [0; 4]).is_ok());
    }

    #[test]
    fn garbage_and_unsupported_input() {
        assert!(matches!(decode(b"not an image at all"), Err(Error::Unsupported(_))));
        assert!(decode(&[]).is_err());
        let png = encode(gradient(4, 4, 255), OutFormat::Png, 80, [0; 4]).unwrap();
        assert!(decode(&png[..png.len() / 2]).is_err());
        let avif_header = b"\0\0\0\x1cftypavif\0\0\0\0avifmif1miaf";
        assert!(matches!(decode(avif_header), Err(Error::Unsupported(_))));
    }

    #[test]
    fn format_ids_round_trip() {
        for id in OutFormat::IDS {
            let format = OutFormat::parse(id).unwrap();
            assert!(!format.extension().is_empty());
        }
        assert_eq!(OutFormat::from_image_format(ImageFormat::Png), Some(OutFormat::Png));
        assert_eq!(OutFormat::from_image_format(ImageFormat::Tga), None);
        assert_eq!(OutFormat::parse("keep"), None);
    }
}
