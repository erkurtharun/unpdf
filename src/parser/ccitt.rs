//! `CCITTFaxDecode` images (ISO 32000-1 §7.4.6): Group 3 and Group 4 fax encoding, decoded to
//! the 1-bit samples the filter produces.

use hayro_ccitt::{DecodeSettings, Decoder, DecoderContext, EncodingMode};

use super::backend::CcittParms;
use super::bilevel::Rows;

/// The widest row decoded — PDFium's bound on an image side (`kMaxImageDimension`), which
/// images are held to elsewhere too.
const MAX_COLUMNS: u32 = 131_071;

/// Decode `data` to a `width` by `height` image of 1-bit samples, each row starting on a byte
/// boundary — what the filter outputs, so the image's color space and `/Decode` read it as
/// any 1-bit image. A sample is 0 for black unless `/BlackIs1`.
///
/// A row wider than `width` is cut to it, a narrower one padded white; rows the data does not
/// reach are white. Data that goes wrong partway keeps the rows decoded before it, as a
/// viewer shows a damaged fax. `None` when `/Columns` is past [`MAX_COLUMNS`] or nothing
/// decodes.
pub(crate) fn decode(data: &[u8], parms: &CcittParms, width: u32, height: u32) -> Option<Vec<u8>> {
    if parms.columns == 0 || parms.columns > MAX_COLUMNS || width == 0 || height == 0 {
        return None;
    }
    let encoding = match parms.k {
        k if k < 0 => EncodingMode::Group4,
        0 => EncodingMode::Group3_1D,
        k => EncodingMode::Group3_2D {
            k: u32::try_from(k).unwrap_or(u32::MAX),
        },
    };
    let rows = parms.rows.filter(|&r| r > 0).unwrap_or(height).min(height);
    let settings = DecodeSettings {
        columns: parms.columns,
        rows,
        end_of_block: parms.end_of_block,
        end_of_line: parms.end_of_line,
        rows_are_byte_aligned: parms.encoded_byte_align,
        encoding,
        invert_black: false,
    };
    let mut out = Rows::new(width, height, !parms.black_is_1);
    let mut ctx = DecoderContext::new(settings);
    let decoded = hayro_ccitt::decode(data, &mut out, &mut ctx);
    if decoded.is_err() && out.is_untouched() {
        return None;
    }
    Some(out.into_samples())
}

impl Decoder for Rows {
    fn push_pixel(&mut self, white: bool) {
        self.push(white);
    }

    fn push_pixel_chunk(&mut self, white: bool, chunk_count: u32) {
        self.push_run(white, chunk_count.saturating_mul(8));
    }

    fn next_line(&mut self) {
        Rows::next_line(self);
    }
}
