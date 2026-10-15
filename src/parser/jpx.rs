//! `JPXDecode` images (ISO 32000-1 §7.4.9): JPEG 2000, decoded to 8-bit samples.

use hayro_jpeg2000::{ColorSpace, DecodeSettings, Image};

use super::backend::ImageColorSpace;

/// A JPEG 2000 image decoded: 8-bit samples, interleaved, in `color`.
pub(crate) struct Decoded {
    pub width: u32,
    pub height: u32,
    /// The space the samples are in: the image dictionary's `/ColorSpace` when it gives one
    /// (ISO 32000-1 §8.9.5.3: it overrides the codestream's), else the codestream's own.
    pub color: ImageColorSpace,
    /// One byte per component per pixel, an alpha channel left out.
    pub samples: Vec<u8>,
}

/// Decode `data`, a JPEG 2000 codestream or JP2 file. `color` is the image dictionary's
/// `/ColorSpace` when it has one. `fits` is asked whether an image of the declared size may
/// be decoded at all, before anything is allocated for it.
///
/// `None` when the data does not decode, its size is refused, or its components do not match
/// the color space (an ICC-based or unknown space in the codestream with no `/ColorSpace` to
/// say what it is, other than by its number of components).
pub(crate) fn decode(
    data: &[u8],
    color: Option<&ImageColorSpace>,
    fits: impl Fn(u32, u32) -> bool,
) -> Option<Decoded> {
    let image = Image::new(data, &DecodeSettings::default()).ok()?;
    let (width, height) = (image.width(), image.height());
    if width == 0 || height == 0 || !fits(width, height) {
        return None;
    }
    let channels = usize::from(image.color_space().num_channels());
    let alpha = image.has_alpha();
    let color = match color {
        Some(given) => given.clone(),
        None => match image.color_space() {
            ColorSpace::Gray => ImageColorSpace::Gray,
            ColorSpace::RGB => ImageColorSpace::Rgb,
            ColorSpace::CMYK => ImageColorSpace::Cmyk,
            ColorSpace::Icc { num_channels, .. } | ColorSpace::Unknown { num_channels } => {
                match num_channels {
                    1 => ImageColorSpace::Gray,
                    3 => ImageColorSpace::Rgb,
                    4 => ImageColorSpace::Cmyk,
                    _ => return None,
                }
            }
        },
    };
    if color.components() != channels {
        return None;
    }
    let decoded = image.decode().ok()?;
    let stride = channels + usize::from(alpha);
    let pixels = width as usize * height as usize;
    if decoded.len() != pixels * stride {
        return None;
    }
    let samples = if alpha {
        decoded
            .chunks_exact(stride)
            .flat_map(|px| px[..channels].iter().copied())
            .collect()
    } else {
        decoded
    };
    Some(Decoded {
        width,
        height,
        color,
        samples,
    })
}
