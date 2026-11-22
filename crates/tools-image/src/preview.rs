//! Pictures for on-screen display.

use sundry_core::Result;

use crate::codec;
use crate::resize::resample;

/// Largest side of a preview; bigger pictures are scaled down (GPU texture limits).
pub const MAX_PREVIEW_SIDE: u32 = 8192;

/// 8-bit RGBA pixels, row by row.
pub struct Preview {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    /// Pixel size of the file itself (differs from `width`/`height` if scaled down).
    pub original: (u32, u32),
    pub format: &'static str,
}

/// Decodes a file for display (EXIF rotation applied).
pub fn decode_preview(bytes: &[u8]) -> Result<Preview> {
    let decoded = codec::decode(bytes)?;
    let format = codec::format_name(decoded.format);
    let original = decoded.image.dimensions();
    let longest = original.0.max(original.1);
    let image = if longest > MAX_PREVIEW_SIDE {
        let scale = f64::from(MAX_PREVIEW_SIDE) / f64::from(longest);
        let size = (
            (f64::from(original.0) * scale).round().max(1.0) as u32,
            (f64::from(original.1) * scale).round().max(1.0) as u32,
        );
        resample(decoded.image, (size.0, size.1), false, "bilinear")?
    } else {
        decoded.image
    };
    let (width, height) = image.dimensions();
    Ok(Preview { width, height, rgba: image.into_raw(), original, format })
}

/// Size and format name of a file, from its header only.
pub fn describe(bytes: &[u8]) -> Option<((u32, u32), &'static str)> {
    codec::probe(bytes).map(|(w, h, f)| ((w, h), codec::format_name(f)))
}
