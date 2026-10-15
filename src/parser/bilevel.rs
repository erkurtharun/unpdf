//! 1-bit images as the bi-level codecs (`CCITTFaxDecode`, `JBIG2Decode`) decode them: pixels
//! pushed row by row, packed into the samples the filter outputs.

/// A `width` by `height` image of 1-bit samples, each row starting on a byte boundary. A
/// sample is 1 for white unless built for the inverse; pixels the codec never pushes stay
/// white.
pub(crate) struct Rows {
    width: u32,
    height: u32,
    stride: usize,
    /// Whether a white pixel is a 1 sample.
    white_is_1: bool,
    data: Vec<u8>,
    row: u32,
    col: u32,
}

impl Rows {
    pub(crate) fn new(width: u32, height: u32, white_is_1: bool) -> Self {
        let stride = width.div_ceil(8) as usize;
        let fill = if white_is_1 { 0xFF } else { 0x00 };
        Rows {
            width,
            height,
            stride,
            white_is_1,
            data: vec![fill; stride * height as usize],
            row: 0,
            col: 0,
        }
    }

    /// One pixel; one past the row's width is not kept.
    pub(crate) fn push(&mut self, white: bool) {
        if self.row < self.height && self.col < self.width {
            let byte = self.row as usize * self.stride + (self.col / 8) as usize;
            let mask = 0x80 >> (self.col % 8);
            if white == self.white_is_1 {
                self.data[byte] |= mask;
            } else {
                self.data[byte] &= !mask;
            }
        }
        self.col = self.col.saturating_add(1);
    }

    /// `count` pixels of one color.
    pub(crate) fn push_run(&mut self, white: bool, count: u32) {
        let kept = count.min(self.width.saturating_sub(self.col));
        for _ in 0..kept {
            self.push(white);
        }
        // Pixels past the row's width are not kept; only the position moves.
        self.col = self.col.saturating_add(count - kept);
    }

    pub(crate) fn next_line(&mut self) {
        self.row = self.row.saturating_add(1);
        self.col = 0;
    }

    /// Whether no pixel has been pushed.
    pub(crate) fn is_untouched(&self) -> bool {
        self.row == 0 && self.col == 0
    }

    pub(crate) fn into_samples(self) -> Vec<u8> {
        self.data
    }
}
