//! `JBIG2Decode` images (ISO 32000-1 §7.4.7): the embedded JBIG2 stream of one page, with its
//! global segments, decoded to the 1-bit samples the filter produces.

use hayro_jbig2::{Decoder, Image};

use super::bilevel::Rows;

/// Decode `data` (with `/JBIG2Globals`' segments, when the image has them) to a `width` by
/// `height` image of 1-bit samples, each row starting on a byte boundary, 0 for black — what
/// the filter outputs (JBIG2's own bitmap has 1 for black; the filter inverts it, as pdf.js
/// and MuPDF do). A page larger than the image is cut to it, a smaller one padded white.
/// `None` when nothing decodes, or the page declares a size past the bound on images.
pub(crate) fn decode(
    data: &[u8],
    globals: Option<&[u8]>,
    width: u32,
    height: u32,
) -> Option<Vec<u8>> {
    if width == 0 || height == 0 {
        return None;
    }
    let image = Image::new_embedded(data, globals).ok()?;
    // The page is allocated at the size its information segment declares: bound it as images
    // are bounded elsewhere, before anything is allocated.
    if !super::png_encode::decodable(image.width(), image.height(), 1) {
        return None;
    }
    let mut out = Rows::new(width, height, true);
    let decoded = image.decode(&mut out);
    if decoded.is_err() && out.is_untouched() {
        return None;
    }
    Some(out.into_samples())
}

impl Decoder for Rows {
    fn push_pixel(&mut self, black: bool) {
        self.push(!black);
    }

    fn push_pixel_chunk(&mut self, black: bool, chunk_count: u32) {
        self.push_run(!black, chunk_count.saturating_mul(8));
    }

    fn next_line(&mut self) {
        Rows::next_line(self);
    }
}
