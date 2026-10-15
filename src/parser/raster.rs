//! Page rasterization (feature `raster`).
//!
//! A page is painted from the same operator stream text extraction reads —
//! [`form_xobject::page_operations`](super::form_xobject::page_operations), forms interpreted
//! in place — so the page rendered is the page extracted: the same page tree, box, rotation
//! and damaged-stream recovery. What this cannot paint yet is counted in [`RasterGaps`], the
//! way extraction counts what it could not read: the rest of the page is still painted.

use tiny_skia::{
    Color, FillRule, FilterQuality, GradientStop, IntSize, LineCap, LineJoin, LinearGradient, Mask,
    Paint, Path, PathBuilder, Pattern, Pixmap, PixmapPaint, Point, RadialGradient, Rect, Shader,
    SpreadMode, Stroke, StrokeDash, Transform,
};

use std::collections::HashMap;
use std::rc::Rc;

use super::backend::{
    CodecParms, ContentOp, GlyphAdvance, ImageColorSpace, ObjectId, PageId, PdfBackend, PdfValue,
    RawXObject, Shading, ShadingGeometry, TilingPattern, Type3Font,
};
use super::form_xobject;
use super::png_encode::{self, PngColorType};
use super::raster_text::LoadedFont;
use super::text_state::{TextMatrix, TextParams};
use crate::error::{Error, Result};

/// How to rasterize a page.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RasterOptions {
    /// Resolution in dots per inch; a page point is `dpi / 72` pixels. Default 150.
    pub dpi: f32,
    /// Which of the page's boxes is painted. Default [`PageRegion::Crop`].
    pub region: PageRegion,
}

impl Default for RasterOptions {
    fn default() -> Self {
        Self {
            dpi: 150.0,
            region: PageRegion::Crop,
        }
    }
}

/// The region of a page a raster covers.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum PageRegion {
    /// The crop box — what a viewer shows, and what a printed page holds.
    #[default]
    Crop,
    /// The whole media box, including what the crop box cuts away.
    Media,
}

/// What a rasterized page could not show, by reason. All zero means everything the page's
/// content asks for was painted.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RasterGaps {
    /// Text runs not painted, or painted only in part: the font's program is not embedded
    /// and no standard face stands in for it, or is one this does not read, or a code
    /// selects no glyph in it (in a Type 3 font: no glyph procedure).
    pub text_runs: u32,
    /// Images not painted: a codec this does not decode, a color
    /// space it does not convert, or data that did not decode.
    pub images: u32,
    /// Inline images (`BI … EI`) not painted: a dictionary that does not read. One that reads
    /// but is in a codec or color space this does not decode is counted in `images`.
    pub inline_images: u32,
    /// Shadings (`sh`) and pattern colors not painted: mesh shadings (types 4–7), function
    /// shadings (type 1), a PostScript calculator function, and a tiling pattern nested more
    /// than three deep. Axial and radial shadings and tiling patterns are painted.
    pub shadings: u32,
    /// Content streams of the page that could not be decoded; whatever they held is missing.
    pub undecodable_content_streams: u32,
}

impl RasterGaps {
    /// Count what `other` left unpainted as left unpainted here too.
    fn absorb(&mut self, other: &RasterGaps) {
        self.text_runs += other.text_runs;
        self.images += other.images;
        self.inline_images += other.inline_images;
        self.shadings += other.shadings;
        self.undecodable_content_streams += other.undecodable_content_streams;
    }

    /// Whether anything the page asked for was left unpainted.
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// A rasterized page: `width × height` pixels, opaque RGBA, row by row from the top-left of
/// the page as displayed (its `/Rotate` applied).
#[derive(Debug, Clone)]
pub struct RasteredPage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    pub gaps: RasterGaps,
    /// Text runs painted in a standard face standing in for a font the PDF does not embed
    /// (feature `standard-fonts`): readable, but not the page's own typeface. Not a gap —
    /// the text is there.
    pub substituted_text_runs: u32,
}

impl RasteredPage {
    /// The page as a PNG (8-bit RGB).
    pub fn to_png(&self) -> Vec<u8> {
        let rgb: Vec<u8> = self
            .rgba
            .as_chunks::<4>()
            .0
            .iter()
            .flat_map(|p| [p[0], p[1], p[2]])
            .collect();
        png_encode::encode(self.width, self.height, PngColorType::Rgb, &rgb)
            .expect("an RGB buffer of width * height pixels encodes")
    }
}

/// Pages larger than this many pixels are refused rather than allocated.
const MAX_PIXELS: u64 = 200_000_000;

/// Rasterize page `page_num` of the document `backend` reads.
pub(crate) fn render_page(
    backend: &dyn PdfBackend,
    page_num: u32,
    options: &RasterOptions,
) -> Result<RasteredPage> {
    let pages = backend.pages();
    let page = *pages
        .get(&page_num)
        .ok_or(Error::PageOutOfRange(page_num, pages.len() as u32))?;
    if !(options.dpi.is_finite() && options.dpi > 0.0) {
        return Err(Error::Render(format!(
            "dpi must be positive, got {}",
            options.dpi
        )));
    }

    let page_box = match options.region {
        PageRegion::Crop => backend.crop_box(page),
        PageRegion::Media => backend.page_box(page),
    };
    let rotation = backend.page_rotation(page);
    let scale = options.dpi / 72.0;
    let (w, h) = (page_box.width() * scale, page_box.height() * scale);
    let (px_w, px_h) = match rotation {
        90 | 270 => (h, w),
        _ => (w, h),
    };
    let (px_w, px_h) = (px_w.round().max(1.0) as u32, px_h.round().max(1.0) as u32);
    if u64::from(px_w) * u64::from(px_h) > MAX_PIXELS {
        return Err(Error::Render(format!(
            "page {page_num} at {} dpi would be {px_w} x {px_h} pixels",
            options.dpi
        )));
    }

    // User space → the unrotated page, top-left origin, in pixels → the page as displayed.
    let unrotated = Transform::from_row(
        scale,
        0.0,
        0.0,
        -scale,
        -page_box.llx * scale,
        page_box.ury * scale,
    );
    let rotate = match rotation {
        90 => Transform::from_row(0.0, 1.0, -1.0, 0.0, h, 0.0),
        180 => Transform::from_row(-1.0, 0.0, 0.0, -1.0, w, h),
        270 => Transform::from_row(0.0, -1.0, 1.0, 0.0, 0.0, w),
        _ => Transform::identity(),
    };
    let base = rotate.pre_concat(unrotated);

    let mut pixmap = Pixmap::new(px_w, px_h)
        .ok_or_else(|| Error::Render(format!("cannot allocate {px_w} x {px_h} pixels")))?;
    pixmap.fill(tiny_skia::Color::WHITE);

    let painted = form_xobject::page_operations(backend, page)?;
    let mut painter = Painter::new(backend, page, pixmap, base);
    painter.gaps.undecodable_content_streams = painted.undecodable_streams as u32;
    for op in &painted.ops {
        painter.apply(op);
    }

    let Painter {
        pixmap,
        gaps,
        substituted_text_runs,
        ..
    } = painter;
    // The page was filled opaque white first, so every pixel is opaque and premultiplied
    // equals straight color.
    Ok(RasteredPage {
        width: px_w,
        height: px_h,
        rgba: pixmap.take(),
        gaps,
        substituted_text_runs,
    })
}

/// A paint color: RGB in `0..=1`.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Rgb(f32, f32, f32);

impl Rgb {
    const BLACK: Rgb = Rgb(0.0, 0.0, 0.0);
}

/// The color space colors are set in (`cs`/`CS`, implied by `g`/`rg`/`k`).
#[derive(Debug, Clone, PartialEq)]
enum PaintSpace {
    Gray,
    Rgb,
    Cmyk,
    Indexed(ImageColorSpace),
    /// A pattern, with the space an uncolored pattern's color is given in.
    Pattern(Option<Box<PaintSpace>>),
    /// A space this does not convert (`Separation`, `DeviceN`, `Lab`): its components are
    /// read as a tint, 0 white to 1 black.
    Tint(usize),
}

impl PaintSpace {
    fn color(&self, values: &[f32]) -> Option<Rgb> {
        let v = |i: usize| values.get(i).copied().unwrap_or(0.0).clamp(0.0, 1.0);
        Some(match self {
            PaintSpace::Gray => Rgb(v(0), v(0), v(0)),
            PaintSpace::Rgb => Rgb(v(0), v(1), v(2)),
            PaintSpace::Cmyk => {
                let k = 1.0 - v(3);
                Rgb((1.0 - v(0)) * k, (1.0 - v(1)) * k, (1.0 - v(2)) * k)
            }
            PaintSpace::Indexed(space) => {
                let ImageColorSpace::Indexed {
                    base,
                    hival,
                    lookup,
                } = space
                else {
                    return None;
                };
                let index = (values.first()?.round().max(0.0) as usize).min(*hival as usize);
                let n = base.components();
                let entry = lookup.get(index * n..index * n + n)?;
                let comps: Vec<f32> = entry.iter().map(|&b| f32::from(b) / 255.0).collect();
                PaintSpace::of_image_space(base).color(&comps)?
            }
            PaintSpace::Pattern(base) => return base.as_ref()?.color(values),
            PaintSpace::Tint(n) => {
                let tint = (0..*n).map(v).fold(0.0f32, f32::max);
                Rgb(1.0 - tint, 1.0 - tint, 1.0 - tint)
            }
        })
    }

    fn of_image_space(space: &ImageColorSpace) -> PaintSpace {
        match space {
            ImageColorSpace::Gray => PaintSpace::Gray,
            ImageColorSpace::Rgb => PaintSpace::Rgb,
            ImageColorSpace::Cmyk => PaintSpace::Cmyk,
            indexed @ ImageColorSpace::Indexed { .. } => PaintSpace::Indexed(indexed.clone()),
        }
    }
}

/// The graphics state (ISO 32000-1 §8.4) as far as painting needs it.
#[derive(Clone)]
struct GState {
    /// User space → device pixels.
    ctm: Transform,
    fill: Rgb,
    stroke: Rgb,
    fill_space: PaintSpace,
    stroke_space: PaintSpace,
    fill_alpha: f32,
    stroke_alpha: f32,
    /// Whether the current fill/stroke color is a pattern.
    fill_pattern: bool,
    stroke_pattern: bool,
    /// What the current fill/stroke pattern paints with; `None` when it is a pattern this
    /// does not paint.
    fill_pattern_paint: Option<Rc<PatternPaint>>,
    stroke_pattern_paint: Option<Rc<PatternPaint>>,
    /// The default space of the content being painted → device pixels: what a pattern's
    /// matrix is relative to (ISO 32000-1 §8.7.3.1) — the page's, or, inside a form, the form's
    /// space as it was entered.
    pattern_base: Transform,
    line_width: f32,
    cap: LineCap,
    join: LineJoin,
    miter: f32,
    dash: Option<(Vec<f32>, f32)>,
    /// The clip, as a coverage mask; `None` is the whole page.
    clip: Option<Mask>,
    /// The text state (ISO 32000-1 §9.3) — part of the graphics state, saved by `q`.
    text: TextState,
}

/// The text state: the font `Tf` selects and the parameters around it.
#[derive(Clone, Default)]
struct TextState {
    /// The font resource `Tf` names, and the form whose resources it is looked up in.
    font: Option<(Option<ObjectId>, Vec<u8>)>,
    params: TextParams,
}

impl GState {
    fn new(base: Transform) -> Self {
        Self {
            ctm: base,
            fill: Rgb::BLACK,
            stroke: Rgb::BLACK,
            fill_space: PaintSpace::Gray,
            stroke_space: PaintSpace::Gray,
            fill_alpha: 1.0,
            stroke_alpha: 1.0,
            fill_pattern: false,
            stroke_pattern: false,
            fill_pattern_paint: None,
            stroke_pattern_paint: None,
            pattern_base: base,
            line_width: 1.0,
            cap: LineCap::Butt,
            join: LineJoin::Miter,
            miter: 10.0,
            dash: None,
            clip: None,
            text: TextState::default(),
        }
    }
}

/// A font resource: the form whose resources name it (`None`: the page's), and its name.
type FontResource = (Option<ObjectId>, Vec<u8>);

struct Painter<'a> {
    backend: &'a dyn PdfBackend,
    page: PageId,
    /// The form whose operations are being painted (`None`: the page's own).
    current_form: Option<ObjectId>,
    pixmap: Pixmap,
    state: GState,
    stack: Vec<GState>,
    /// The path under construction, in user space.
    path: PathBuilder,
    /// A clip set by `W`/`W*`, applied when the path is next painted or ended.
    pending_clip: Option<FillRule>,
    /// The text matrix and text line matrix, inside `BT … ET`.
    text_matrix: TextMatrix,
    /// Fonts loaded so far, by the resource that names them; `None` when unpaintable.
    fonts: HashMap<FontResource, Option<LoadedFont>>,
    /// Type 3 fonts looked up so far, by the same key; `None` for a font of any other type.
    type3: HashMap<FontResource, Option<Rc<Type3Font>>>,
    /// How many Type 3 glyph procedures are running, one inside another.
    glyph_depth: usize,
    /// How many tiling pattern cells are being painted, one inside another: this painter's.
    pattern_depth: usize,
    /// Tiling pattern cells rendered so far, by where and in what color they are painted.
    tiles: HashMap<TileKey, Option<Rc<PatternPaint>>>,
    /// Painting a shape that takes its color from outside: a Type 3 glyph procedure that
    /// began with `d1`, or the cell of an uncolored tiling pattern. Its own color operators
    /// are ignored.
    uncolored: bool,
    gaps: RasterGaps,
    substituted_text_runs: u32,
}

/// A tiling pattern cell as painted: the content it was named from (form and name), its
/// pattern space → device pixels, and the color of an uncolored one -- as bits, to be a key.
type TileKey = (Option<ObjectId>, Vec<u8>, [u32; 6], [u32; 3]);

/// What a pattern color paints with, placed on the device.
enum PatternPaint {
    /// A shading pattern: its shading, and pattern space → device pixels.
    Shading(Shading, Transform),
    /// A tiling pattern: one cell, rendered, and its pixels → device pixels. The cell repeats.
    Tile(Pixmap, Transform),
}

/// The paint for one drawing operation. It owns what it paints with, so drawing with it does
/// not hold the painter borrowed.
enum PaintSource {
    Ready(Paint<'static>),
    /// A tiling pattern's cell, with its pixels → the space the drawing is in.
    Tile(Rc<PatternPaint>, Transform, f32),
}

impl PaintSource {
    fn paint(&self) -> Paint<'_> {
        match self {
            PaintSource::Ready(paint) => paint.clone(),
            PaintSource::Tile(tile, transform, alpha) => {
                let PatternPaint::Tile(pixmap, _) = &**tile else {
                    unreachable!("a tile source holds a tile");
                };
                Paint {
                    shader: Pattern::new(
                        pixmap.as_ref(),
                        SpreadMode::Repeat,
                        FilterQuality::Bilinear,
                        alpha.clamp(0.0, 1.0),
                        *transform,
                    ),
                    anti_alias: true,
                    ..Paint::default()
                }
            }
        }
    }
}

impl<'a> Painter<'a> {
    /// A painter for `page` drawing into `pixmap`, user space placed on it by `base`.
    fn new(backend: &'a dyn PdfBackend, page: PageId, pixmap: Pixmap, base: Transform) -> Self {
        Painter {
            backend,
            page,
            current_form: None,
            pixmap,
            state: GState::new(base),
            stack: Vec::new(),
            path: PathBuilder::new(),
            pending_clip: None,
            text_matrix: TextMatrix::default(),
            fonts: HashMap::new(),
            type3: HashMap::new(),
            glyph_depth: 0,
            pattern_depth: 0,
            tiles: HashMap::new(),
            uncolored: false,
            gaps: RasterGaps::default(),
            substituted_text_runs: 0,
        }
    }
}

fn number(v: &PdfValue) -> Option<f32> {
    match v {
        PdfValue::Integer(i) => Some(*i as f32),
        PdfValue::Real(r) => Some(*r),
        _ => None,
    }
}

impl Painter<'_> {
    fn apply(&mut self, op: &ContentOp) {
        // A form's operations follow the `q`, `cm` and clip that place it, so where they start
        // the CTM is the form's space. (Leaving a form, the `Q` that follows restores the outer
        // state, this included.)
        if op.form != self.current_form {
            self.current_form = op.form;
            self.state.pattern_base = self.state.ctm;
        }
        // An uncolored glyph's procedure, or an uncolored pattern's cell, sets no color of its
        // own (ISO 32000-1 §9.6.5, §8.7.3.3).
        if self.uncolored
            && matches!(
                op.operator.as_str(),
                "g" | "G" | "rg" | "RG" | "k" | "K" | "cs" | "CS" | "sc" | "scn" | "SC" | "SCN"
            )
        {
            return;
        }
        let n = |i: usize| op.operands.get(i).and_then(number);
        let all = || op.operands.iter().filter_map(number).collect::<Vec<f32>>();
        // The text state and the text position, read as text extraction reads them.
        self.state.text.params.apply(op);
        self.text_matrix.apply(op, &self.state.text.params);
        match op.operator.as_str() {
            // Graphics state.
            "q" => self.stack.push(self.state.clone()),
            "Q" => {
                if let Some(saved) = self.stack.pop() {
                    self.state = saved;
                }
            }
            "cm" => {
                if let (Some(a), Some(b), Some(c), Some(d), Some(e), Some(f)) =
                    (n(0), n(1), n(2), n(3), n(4), n(5))
                {
                    self.state.ctm = self
                        .state
                        .ctm
                        .pre_concat(Transform::from_row(a, b, c, d, e, f));
                }
            }
            "w" => {
                if let Some(w) = n(0) {
                    self.state.line_width = w.max(0.0);
                }
            }
            "J" => {
                self.state.cap = match n(0).map(|v| v as i64) {
                    Some(1) => LineCap::Round,
                    Some(2) => LineCap::Square,
                    _ => LineCap::Butt,
                }
            }
            "j" => {
                self.state.join = match n(0).map(|v| v as i64) {
                    Some(1) => LineJoin::Round,
                    Some(2) => LineJoin::Bevel,
                    _ => LineJoin::Miter,
                }
            }
            "M" => {
                if let Some(m) = n(0) {
                    self.state.miter = m.max(1.0);
                }
            }
            "d" => {
                let array: Vec<f32> = match op.operands.first() {
                    Some(PdfValue::Array(items)) => items.iter().filter_map(number).collect(),
                    _ => Vec::new(),
                };
                let phase = n(1).unwrap_or(0.0);
                self.state.dash =
                    (!array.is_empty() && array.iter().any(|&v| v > 0.0)).then_some((array, phase));
            }
            "gs" => {
                if let Some(PdfValue::Name(name)) = op.operands.first() {
                    if let Some(gs) = self.backend.ext_gstate(op.scope(self.page), name) {
                        if let Some(lw) = gs.line_width {
                            self.state.line_width = lw.max(0.0);
                        }
                        if let Some(a) = gs.stroke_alpha {
                            self.state.stroke_alpha = a.clamp(0.0, 1.0);
                        }
                        if let Some(a) = gs.fill_alpha {
                            self.state.fill_alpha = a.clamp(0.0, 1.0);
                        }
                    }
                }
            }

            // Color.
            "g" => self.set_fill(PaintSpace::Gray, &all()),
            "G" => self.set_stroke(PaintSpace::Gray, &all()),
            "rg" => self.set_fill(PaintSpace::Rgb, &all()),
            "RG" => self.set_stroke(PaintSpace::Rgb, &all()),
            "k" => self.set_fill(PaintSpace::Cmyk, &all()),
            "K" => self.set_stroke(PaintSpace::Cmyk, &all()),
            "cs" | "CS" => {
                let space = match op.operands.first() {
                    Some(PdfValue::Name(name)) => self.space_named(op, name),
                    _ => PaintSpace::Gray,
                };
                // Setting a space resets the color to its initial value: black, or the
                // first palette entry.
                let initial = match &space {
                    PaintSpace::Cmyk => vec![0.0, 0.0, 0.0, 1.0],
                    PaintSpace::Gray | PaintSpace::Tint(_) | PaintSpace::Indexed(_) => vec![0.0],
                    PaintSpace::Rgb => vec![0.0, 0.0, 0.0],
                    PaintSpace::Pattern(_) => Vec::new(),
                };
                if op.operator == "cs" {
                    self.set_fill(space, &initial);
                } else {
                    self.set_stroke(space, &initial);
                }
                // `Tint` spaces start at full tint (black) — tint 1, read as 1 - 1 = 0.
                if let PaintSpace::Tint(_) = self.state.fill_space {
                    if op.operator == "cs" {
                        self.state.fill = Rgb::BLACK;
                    }
                }
            }
            "sc" | "scn" => {
                let space = self.state.fill_space.clone();
                self.set_fill(space, &all());
                if self.state.fill_pattern {
                    self.state.fill_pattern_paint = self.pattern_named(op, self.state.fill);
                }
            }
            "SC" | "SCN" => {
                let space = self.state.stroke_space.clone();
                self.set_stroke(space, &all());
                if self.state.stroke_pattern {
                    self.state.stroke_pattern_paint = self.pattern_named(op, self.state.stroke);
                }
            }

            // Path construction (user space).
            "m" => {
                if let (Some(x), Some(y)) = (n(0), n(1)) {
                    self.path.move_to(x, y);
                }
            }
            "l" => {
                if let (Some(x), Some(y)) = (n(0), n(1)) {
                    self.path.line_to(x, y);
                }
            }
            "c" => {
                if let (Some(x1), Some(y1), Some(x2), Some(y2), Some(x3), Some(y3)) =
                    (n(0), n(1), n(2), n(3), n(4), n(5))
                {
                    self.path.cubic_to(x1, y1, x2, y2, x3, y3);
                }
            }
            "v" => {
                if let (Some(x2), Some(y2), Some(x3), Some(y3), Some(cur)) =
                    (n(0), n(1), n(2), n(3), self.path.last_point())
                {
                    self.path.cubic_to(cur.x, cur.y, x2, y2, x3, y3);
                }
            }
            "y" => {
                if let (Some(x1), Some(y1), Some(x3), Some(y3)) = (n(0), n(1), n(2), n(3)) {
                    self.path.cubic_to(x1, y1, x3, y3, x3, y3);
                }
            }
            "h" => self.path.close(),
            "re" => {
                if let (Some(x), Some(y), Some(w), Some(h)) = (n(0), n(1), n(2), n(3)) {
                    // A rectangle is a closed subpath of its own; negative sizes are allowed.
                    self.path.move_to(x, y);
                    self.path.line_to(x + w, y);
                    self.path.line_to(x + w, y + h);
                    self.path.line_to(x, y + h);
                    self.path.close();
                }
            }

            // Clipping: takes effect when the path is next painted or ended.
            "W" => self.pending_clip = Some(FillRule::Winding),
            "W*" => self.pending_clip = Some(FillRule::EvenOdd),

            // Path painting.
            "S" => self.paint(false, None, true),
            "s" => {
                self.path.close();
                self.paint(false, None, true)
            }
            "f" | "F" => self.paint(false, Some(FillRule::Winding), false),
            "f*" => self.paint(false, Some(FillRule::EvenOdd), false),
            "B" => self.paint(false, Some(FillRule::Winding), true),
            "B*" => self.paint(false, Some(FillRule::EvenOdd), true),
            "b" => self.paint(true, Some(FillRule::Winding), true),
            "b*" => self.paint(true, Some(FillRule::EvenOdd), true),
            "n" => self.paint(false, None, false),

            // External objects.
            "Do" => {
                if let Some(PdfValue::Name(name)) = op.operands.first() {
                    // Forms were interpreted in place; a `Do` left is an image, or a kind of
                    // XObject nothing paints (PostScript).
                    if let Some(image) = self.backend.image_xobject(op.scope(self.page), name) {
                        self.draw_image(&image);
                    }
                }
            }
            "BI" => {
                let image = match op.operands.first() {
                    Some(PdfValue::Str(body)) => {
                        self.backend.inline_image(op.scope(self.page), body)
                    }
                    _ => None,
                };
                match image {
                    Some(image) => self.draw_image(&image),
                    None => self.gaps.inline_images += 1,
                }
            }
            // Type 3 glyph metrics: `d1` marks the glyph uncolored.
            "d1" if self.glyph_depth > 0 => self.uncolored = true,
            "sh" => {
                let shading = match op.operands.first() {
                    Some(PdfValue::Name(name)) => self.backend.shading(op.scope(self.page), name),
                    _ => None,
                };
                match shading {
                    Some(shading) => self.paint_shading(&shading),
                    None => self.gaps.shadings += 1,
                }
            }

            // Text: the state and position were applied above; `'` and `"` have moved to
            // the next line, and `"` set its spacing.
            "Tf" => {
                if let Some(PdfValue::Name(name)) = op.operands.first() {
                    self.state.text.font = Some((op.form, name.clone()));
                }
            }
            "Tj" | "'" => self.show(op, op.operands.get(..1).unwrap_or(&[])),
            "TJ" => {
                if let Some(PdfValue::Array(items)) = op.operands.first() {
                    self.show(op, items);
                }
            }
            "\"" => self.show(op, op.operands.get(2..3).unwrap_or(&[])),
            _ => {}
        }
    }

    /// Paint the strings and position adjustments of a text-showing operator, advancing the
    /// text matrix as text extraction does.
    fn show(&mut self, op: &ContentOp, items: &[PdfValue]) {
        let Some(key) = self.state.text.font.clone() else {
            return;
        };
        let text = self.state.text.params.clone();
        let scope = op.scope(self.page);
        if !self.type3.contains_key(&key) {
            let font = self.backend.type3_font(scope, &key.1).map(Rc::new);
            self.type3.insert(key.clone(), font);
        }
        if let Some(font) = self.type3.get(&key).cloned().flatten() {
            self.show_type3(&font, scope, &key.1, &text, items);
            return;
        }
        if !self.fonts.contains_key(&key) {
            let loaded = self
                .backend
                .font_program(scope, &key.1)
                .and_then(LoadedFont::load);
            self.fonts.insert(key.clone(), loaded);
        }
        let vertical = self.backend.is_vertical_font(scope, &key.1);
        // Mode 3 paints nothing (an OCR layer over a scan): nothing is missing.
        let invisible = matches!(text.render_mode, 3 | 7);
        let mut missing = false;
        let mut substituted = false;
        let mut unpainted_color = false;
        // Taken out of the cache while its glyphs are painted, put back after.
        let mut loaded = self.fonts.remove(&key).flatten();

        for item in items {
            let bytes = match item {
                PdfValue::Str(bytes) => bytes,
                PdfValue::Integer(_) | PdfValue::Real(_) => {
                    self.text_matrix
                        .advance(text.adjustment(number(item).unwrap_or(0.0), vertical));
                    continue;
                }
                _ => continue,
            };
            let advances = self.backend.glyph_advances(scope, &key.1, bytes);
            let Some(font) = loaded.as_mut() else {
                missing |= !invisible;
                // Still move past the run, so what follows lands where it should.
                if let Some(advances) = &advances {
                    self.text_matrix.advance(text.advance_of(advances));
                }
                continue;
            };
            let width = font.code_bytes();
            for (i, chunk) in bytes.chunks(width).enumerate() {
                let code = chunk.iter().fold(0u32, |acc, &b| acc << 8 | u32::from(b));
                let gid = font.glyph(code);
                // The width extraction measures with; the program's own when the font
                // dictionary declares none.
                let measured = match advances.as_ref().and_then(|a| a.get(i)) {
                    Some(g) => *g,
                    None => {
                        let units = gid.and_then(|g| font.program_advance(g)).unwrap_or(0.0);
                        GlyphAdvance {
                            width: units * font.glyph_to_text.sx * 1000.0,
                            vertical: None,
                            is_word_space: width == 1 && code == 32,
                        }
                    }
                };
                let w0 = measured.width;
                match gid {
                    Some(gid) if !invisible => {
                        let mut glyph_space = font.glyph_to_text;
                        if font.stand_in {
                            // A stand-in face is drawn to the width the font gives the
                            // glyph, so the line keeps the length the page set.
                            let own = font.program_advance(gid).unwrap_or(0.0)
                                * font.glyph_to_text.sx
                                * 1000.0;
                            if own > 0.0 && w0 > 0.0 {
                                let fit = (w0 / own).clamp(0.5, 2.0);
                                glyph_space = glyph_space.post_scale(fit, 1.0);
                            }
                            substituted = true;
                        }
                        if let Some(outline) = font.outline(gid).cloned() {
                            // A vertical glyph hangs from the text position by its position
                            // vector.
                            let origin = text.origin_offset(&measured);
                            let to_text = Transform::from_row(
                                text.size * text.horizontal_scale,
                                0.0,
                                0.0,
                                text.size,
                                0.0,
                                text.rise,
                            );
                            let [a, b, c, d, e, f] = self.text_matrix.tm;
                            let transform = self
                                .state
                                .ctm
                                .pre_concat(Transform::from_row(a, b, c, d, e, f))
                                .pre_concat(Transform::from_translate(origin.tx, origin.ty))
                                .pre_concat(to_text)
                                .pre_concat(glyph_space);
                            unpainted_color |=
                                !self.paint_glyph(&outline, transform, text.render_mode);
                        }
                    }
                    Some(_) => {}
                    None => missing |= !invisible && code != 32,
                }
                self.text_matrix.advance(text.glyph_advance(&measured));
            }
        }
        self.fonts.insert(key, loaded);
        if missing {
            self.gaps.text_runs += 1;
        }
        // Text in a pattern this does not paint, once per run.
        if unpainted_color {
            self.gaps.shadings += 1;
        }
        if substituted {
            self.substituted_text_runs += 1;
        }
    }

    /// Show strings in a Type 3 font: each code's glyph procedure runs where the glyph goes,
    /// then the text position advances by the glyph's width.
    fn show_type3(
        &mut self,
        font: &Type3Font,
        scope: super::backend::ResourceScope,
        font_name: &[u8],
        text: &TextParams,
        items: &[PdfValue],
    ) {
        // Mode 3 paints nothing; mode 7 only clips, which glyph procedures do not do here.
        let invisible = matches!(text.render_mode, 3 | 7);
        let mut missing = false;
        for item in items {
            let bytes = match item {
                PdfValue::Str(bytes) => bytes,
                PdfValue::Integer(_) | PdfValue::Real(_) => {
                    self.text_matrix
                        .advance(text.adjustment(number(item).unwrap_or(0.0), false));
                    continue;
                }
                _ => continue,
            };
            let advances = self.backend.glyph_advances(scope, font_name, bytes);
            for (i, &code) in bytes.iter().enumerate() {
                let measured =
                    advances
                        .as_ref()
                        .and_then(|a| a.get(i).copied())
                        .unwrap_or(GlyphAdvance {
                            width: 0.0,
                            vertical: None,
                            is_word_space: code == 32,
                        });
                if !invisible {
                    match font.glyphs.get(&code) {
                        Some(ops) => self.run_glyph(ops, font.matrix, text, &measured),
                        None => missing |= code != 32,
                    }
                }
                self.text_matrix.advance(text.glyph_advance(&measured));
            }
        }
        if missing {
            self.gaps.text_runs += 1;
        }
    }

    /// Run one Type 3 glyph procedure with glyph space placed at the current text position.
    /// What the procedure does to the graphics, text and path state stays inside it.
    fn run_glyph(
        &mut self,
        ops: &[ContentOp],
        matrix: [f32; 6],
        text: &TextParams,
        measured: &GlyphAdvance,
    ) {
        // A glyph that shows text in a Type 3 font that shows text ... ends here.
        const MAX_GLYPH_DEPTH: usize = 4;
        if self.glyph_depth >= MAX_GLYPH_DEPTH {
            return;
        }
        let origin = text.origin_offset(measured);
        let to_text = Transform::from_row(
            text.size * text.horizontal_scale,
            0.0,
            0.0,
            text.size,
            0.0,
            text.rise,
        );
        let [a, b, c, d, e, f] = self.text_matrix.tm;
        let [ma, mb, mc, md, me, mf] = matrix;
        let glyph_space = self
            .state
            .ctm
            .pre_concat(Transform::from_row(a, b, c, d, e, f))
            .pre_concat(Transform::from_translate(origin.tx, origin.ty))
            .pre_concat(to_text)
            .pre_concat(Transform::from_row(ma, mb, mc, md, me, mf));

        let saved_state = self.state.clone();
        let saved_depth = self.stack.len();
        let saved_path = std::mem::replace(&mut self.path, PathBuilder::new());
        let saved_clip = self.pending_clip.take();
        let saved_text = self.text_matrix.clone();
        let saved_form = self.current_form;
        let saved_uncolored = std::mem::replace(&mut self.uncolored, false);
        self.state.ctm = glyph_space;
        self.glyph_depth += 1;
        for op in ops {
            self.apply(op);
        }
        self.glyph_depth -= 1;
        self.stack.truncate(saved_depth);
        self.state = saved_state;
        self.path = saved_path;
        self.pending_clip = saved_clip;
        self.text_matrix = saved_text;
        self.current_form = saved_form;
        self.uncolored = saved_uncolored;
    }

    /// Paint one glyph outline in text render mode `mode` (fill, stroke, or both). `false` when
    /// part of it was not painted: its color is a pattern this does not paint.
    fn paint_glyph(&mut self, outline: &Path, transform: Transform, mode: i64) -> bool {
        let fill = matches!(mode, 0 | 2 | 4 | 6);
        let stroke = matches!(mode, 1 | 2 | 5 | 6);
        let mut painted = true;
        if fill {
            match self.current_paint(false, transform) {
                Some(source) => self.pixmap.fill_path(
                    outline,
                    &source.paint(),
                    FillRule::Winding,
                    transform,
                    self.state.clip.as_ref(),
                ),
                None => painted = false,
            }
        }
        if stroke {
            // The line width is in user space; the glyph path is in glyph space. Stroke in
            // device space with the width the CTM gives it.
            let Some(source) = self.current_paint(true, Transform::identity()) else {
                return false;
            };
            let paint = source.paint();
            if let Some(device) = outline.clone().transform(transform) {
                let width = self.state.line_width * transform_scale(&self.state.ctm);
                let stroke = Stroke {
                    width: if width < 1.0 { 0.0 } else { width },
                    ..Stroke::default()
                };
                self.pixmap.stroke_path(
                    &device,
                    &paint,
                    &stroke,
                    Transform::identity(),
                    self.state.clip.as_ref(),
                );
            }
        }
        painted
    }

    fn space_named(&self, op: &ContentOp, name: &[u8]) -> PaintSpace {
        if let Some(base) = self.backend.named_pattern_space(op.scope(self.page), name) {
            return PaintSpace::Pattern(base.map(|b| Box::new(PaintSpace::of_image_space(&b))));
        }
        match name {
            b"DeviceGray" | b"G" | b"CalGray" => PaintSpace::Gray,
            b"DeviceRGB" | b"RGB" | b"CalRGB" => PaintSpace::Rgb,
            b"DeviceCMYK" | b"CMYK" => PaintSpace::Cmyk,
            b"Pattern" => PaintSpace::Pattern(None),
            _ => match self.backend.named_color_space(op.scope(self.page), name) {
                Some(space) => PaintSpace::of_image_space(&space),
                // `Separation`, `DeviceN`, `Lab`, or a `Pattern` space with a base: read as
                // a tint, so separations still come out dark where they are inked.
                None => PaintSpace::Tint(1),
            },
        }
    }

    fn set_fill(&mut self, space: PaintSpace, values: &[f32]) {
        self.state.fill_pattern = matches!(space, PaintSpace::Pattern(_));
        self.state.fill_pattern_paint = None;
        if let Some(color) = space.color(values) {
            self.state.fill = color;
        }
        self.state.fill_space = space;
    }

    fn set_stroke(&mut self, space: PaintSpace, values: &[f32]) {
        self.state.stroke_pattern = matches!(space, PaintSpace::Pattern(_));
        self.state.stroke_pattern_paint = None;
        if let Some(color) = space.color(values) {
            self.state.stroke = color;
        }
        self.state.stroke_space = space;
    }

    /// Paint the current path — fill with `fill`'s rule, stroke if `stroke` — then apply a
    /// pending clip and start a new path.
    fn paint(&mut self, close: bool, fill: Option<FillRule>, stroke: bool) {
        if close {
            self.path.close();
        }
        let builder = std::mem::replace(&mut self.path, PathBuilder::new());
        let path = builder.finish();
        if let Some(path) = &path {
            if let Some(rule) = fill {
                match self.current_paint(false, self.state.ctm) {
                    Some(source) => self.pixmap.fill_path(
                        path,
                        &source.paint(),
                        rule,
                        self.state.ctm,
                        self.state.clip.as_ref(),
                    ),
                    None => self.gaps.shadings += 1,
                }
            }
            if stroke {
                match self.current_paint(true, self.state.ctm) {
                    Some(source) => self.stroke(path, &source.paint()),
                    None => self.gaps.shadings += 1,
                }
            }
        }
        if let Some(rule) = self.pending_clip.take() {
            self.clip(path.as_ref(), rule);
        }
    }

    /// What the pattern `scn`/`SCN` names (its last operand) paints with, placed by its matrix
    /// relative to the default space of the content being painted. `color` is the color given
    /// with it, which an uncolored tiling pattern paints in.
    fn pattern_named(&mut self, op: &ContentOp, color: Rgb) -> Option<Rc<PatternPaint>> {
        let Some(PdfValue::Name(name)) = op.operands.last() else {
            return None;
        };
        let scope = op.scope(self.page);
        let place = |[a, b, c, d, e, f]: [f32; 6]| {
            self.state
                .pattern_base
                .pre_concat(Transform::from_row(a, b, c, d, e, f))
        };
        if let Some((shading, matrix)) = self.backend.shading_pattern(scope, name) {
            return Some(Rc::new(PatternPaint::Shading(shading, place(matrix))));
        }
        let pattern = self.backend.tiling_pattern(scope, name)?;
        let to_device = place(pattern.matrix);
        let t = to_device;
        let key = (
            op.form,
            name.clone(),
            [t.sx, t.ky, t.kx, t.sy, t.tx, t.ty].map(f32::to_bits),
            [color.0, color.1, color.2].map(f32::to_bits),
        );
        if let Some(tile) = self.tiles.get(&key) {
            return tile.clone();
        }
        let tile = self
            .render_tile(&pattern, to_device, color)
            .map(|(pixmap, to_device)| Rc::new(PatternPaint::Tile(pixmap, to_device)));
        self.tiles.insert(key, tile.clone());
        tile
    }

    /// Paint one cell of a tiling pattern into a pixmap of its own, at the resolution the
    /// device shows it with, and say how its pixels land on the device.
    ///
    /// The cell is `x_step` by `y_step` from the `/BBox`'s lower left corner, clipped to the
    /// `/BBox`, as pdf.js paints it: what a cell paints past its step is cut off rather than
    /// overlapping the next. The cell's resolution is capped, as pdf.js caps it (3000 pixels a
    /// side); a coarser cell is still repeated at its exact step.
    fn render_tile(
        &mut self,
        pattern: &TilingPattern,
        to_device: Transform,
        color: Rgb,
    ) -> Option<(Pixmap, Transform)> {
        const MAX_PATTERN_DEPTH: usize = 3;
        const MAX_TILE_SIDE: f32 = 3000.0;
        if self.pattern_depth >= MAX_PATTERN_DEPTH {
            return None;
        }
        let Some(ops) = &pattern.ops else {
            self.gaps.undecodable_content_streams += 1;
            return None;
        };
        let (x_step, y_step) = (pattern.x_step.abs(), pattern.y_step.abs());
        if !(x_step.is_finite() && y_step.is_finite() && x_step > 0.0 && y_step > 0.0) {
            return None;
        }
        // Device pixels per pattern unit along each axis of pattern space.
        let along_x = to_device.sx.hypot(to_device.ky);
        let along_y = to_device.kx.hypot(to_device.sy);
        let side = |len: f32| {
            if len.is_finite() {
                len.ceil().clamp(1.0, MAX_TILE_SIDE)
            } else {
                1.0
            }
        };
        let (w, h) = (side(x_step * along_x), side(y_step * along_y));
        // The cell's pixels: pattern space scaled to fill them exactly, y down.
        let (kx, ky) = (w / x_step, h / y_step);
        let (x0, y0) = (pattern.bbox.llx, pattern.bbox.lly);
        let cell = Transform::from_row(kx, 0.0, 0.0, -ky, -x0 * kx, (y0 + y_step) * ky);
        let tile_to_device = to_device.pre_concat(cell.invert()?);

        let pixmap = Pixmap::new(w as u32, h as u32)?;
        let mut tile = Painter::new(self.backend, self.page, pixmap, cell);
        tile.current_form = Some(pattern.id);
        tile.pattern_depth = self.pattern_depth + 1;
        tile.glyph_depth = self.glyph_depth;
        if pattern.uncolored {
            tile.uncolored = true;
            tile.state.fill = color;
            tile.state.stroke = color;
        }
        // The cell clips what it paints (ISO 32000-1 §8.7.3.3).
        let b = &pattern.bbox;
        let bbox = Rect::from_ltrb(
            b.llx.min(b.urx),
            b.lly.min(b.ury),
            b.llx.max(b.urx),
            b.lly.max(b.ury),
        );
        if let Some(bbox) = bbox {
            tile.clip(Some(&PathBuilder::from_rect(bbox)), FillRule::Winding);
        }
        let cell_ops = form_xobject::content_operations(self.backend, self.page, ops, pattern.id);
        tile.gaps.undecodable_content_streams += cell_ops.undecodable_streams as u32;
        for op in &cell_ops.ops {
            tile.apply(op);
        }
        self.gaps.absorb(&tile.gaps);
        self.substituted_text_runs += tile.substituted_text_runs;
        Some((tile.pixmap, tile_to_device))
    }

    /// The paint for filling (`stroke` false) or stroking with the current color, drawn with
    /// `path_transform` (the CTM for a path, the glyph's transform for a glyph). That transform
    /// applies to a pattern's shader too, so the shader is placed by its inverse followed by
    /// the pattern's own placement on the device. `None` when the color is a pattern this does
    /// not paint.
    fn current_paint(&self, stroke: bool, path_transform: Transform) -> Option<PaintSource> {
        let s = &self.state;
        let (pattern, pattern_paint, color, alpha) = if stroke {
            (
                s.stroke_pattern,
                &s.stroke_pattern_paint,
                s.stroke,
                s.stroke_alpha,
            )
        } else {
            (s.fill_pattern, &s.fill_pattern_paint, s.fill, s.fill_alpha)
        };
        if !pattern {
            return Some(PaintSource::Ready(solid(color, alpha)));
        }
        let pattern_paint = pattern_paint.as_ref()?;
        let to_path = path_transform.invert()?;
        match &**pattern_paint {
            PatternPaint::Shading(shading, to_device) => {
                shading_paint(shading, to_path.pre_concat(*to_device), alpha)
                    .map(PaintSource::Ready)
            }
            PatternPaint::Tile(_, to_device) => Some(PaintSource::Tile(
                pattern_paint.clone(),
                to_path.pre_concat(*to_device),
                alpha,
            )),
        }
    }

    /// `sh`: paint a shading over the clip — or over its `/BBox` — in the current user space,
    /// which is the shading's space.
    fn paint_shading(&mut self, shading: &Shading) {
        // The rectangle painted and the transform it is drawn with — which the shader is
        // drawn with too: in user space the shader needs no transform of its own, in device
        // pixels it needs the CTM.
        let (rect, transform, shader_transform) = match shading.bbox {
            Some([x0, y0, x1, y1]) => (
                Rect::from_ltrb(x0.min(x1), y0.min(y1), x0.max(x1), y0.max(y1)),
                self.state.ctm,
                Transform::identity(),
            ),
            None => {
                let (w, h) = (self.pixmap.width() as f32, self.pixmap.height() as f32);
                (
                    Rect::from_xywh(0.0, 0.0, w, h),
                    Transform::identity(),
                    self.state.ctm,
                )
            }
        };
        let paint = shading_paint(shading, shader_transform, self.state.fill_alpha);
        let (Some(rect), Some(paint)) = (rect, paint) else {
            self.gaps.shadings += 1;
            return;
        };
        self.pixmap
            .fill_rect(rect, &paint, transform, self.state.clip.as_ref());
    }

    fn stroke(&mut self, path: &Path, paint: &Paint) {
        // Width 0 is the thinnest line the device can show; a line thinner than a pixel is
        // drawn one pixel wide so it does not vanish.
        let scale = transform_scale(&self.state.ctm);
        let width = if self.state.line_width * scale < 1.0 {
            0.0 // tiny-skia: a hairline
        } else {
            self.state.line_width
        };
        let mut stroke = Stroke {
            width,
            miter_limit: self.state.miter,
            line_cap: self.state.cap,
            line_join: self.state.join,
            dash: None,
        };
        if let Some((array, phase)) = &self.state.dash {
            let mut array = array.clone();
            if array.len() % 2 == 1 {
                array.extend_from_within(..);
            }
            stroke.dash = StrokeDash::new(array, *phase);
        }
        self.pixmap.stroke_path(
            path,
            paint,
            &stroke,
            self.state.ctm,
            self.state.clip.as_ref(),
        );
    }

    /// Intersect the clip with `path` (no path: an empty clip).
    fn clip(&mut self, path: Option<&Path>, rule: FillRule) {
        let (w, h) = (self.pixmap.width(), self.pixmap.height());
        let Some(path) = path else {
            self.state.clip = Mask::new(w, h);
            return;
        };
        match &mut self.state.clip {
            Some(mask) => mask.intersect_path(path, rule, true, self.state.ctm),
            None => {
                if let Some(mut mask) = Mask::new(w, h) {
                    mask.fill_path(path, rule, true, self.state.ctm);
                    self.state.clip = Some(mask);
                }
            }
        }
    }

    fn draw_image(&mut self, image: &RawXObject) {
        let Some(pixmap) = self.image_pixmap(image) else {
            self.gaps.images += 1;
            return;
        };
        // How many device pixels the image's width and height land on. An image drawn much
        // smaller than its samples is averaged down first: sampling it directly drops the
        // thin strokes of a scanned page's text.
        let ctm = self.state.ctm;
        let target = (ctm.sx.hypot(ctm.ky), ctm.kx.hypot(ctm.sy));
        let pixmap = downsample(pixmap, target);
        let (w, h) = (pixmap.width() as f32, pixmap.height() as f32);
        // Image space: the unit square, its first row at the top (y = 1).
        let to_unit = Transform::from_row(1.0 / w, 0.0, 0.0, -1.0 / h, 0.0, 1.0);
        let transform = self.state.ctm.pre_concat(to_unit);
        let paint = PixmapPaint {
            opacity: self.state.fill_alpha,
            quality: FilterQuality::Bilinear,
            ..PixmapPaint::default()
        };
        self.pixmap.draw_pixmap(
            0,
            0,
            pixmap.as_ref(),
            &paint,
            transform,
            self.state.clip.as_ref(),
        );
    }

    /// An image as a premultiplied pixmap: its samples, its soft mask as alpha, or — a
    /// stencil mask — the fill color where the stencil marks.
    fn image_pixmap(&self, image: &RawXObject) -> Option<Pixmap> {
        let (w, h) = (image.width?, image.height?);
        if w == 0 || h == 0 || !png_encode::decodable(w, h, 4) {
            return None;
        }
        let mut rgba: Vec<u8> = if image.image_mask {
            let samples = plain_samples(image, w, h)?;
            // 0 marks, unless `/Decode [1 0]` inverts it.
            let marks_on_one = image
                .decode
                .as_deref()
                .is_some_and(|d| d.first() == Some(&1.0));
            let (_, bits) =
                png_encode::image_pixels(w, h, 1, &ImageColorSpace::Gray, None, &samples)?;
            let Rgb(r, g, b) = self.state.fill;
            let c = [r, g, b].map(|v| (v * 255.0).round() as u8);
            bits.iter()
                .flat_map(|&v| {
                    let marked = (v == 0) != marks_on_one;
                    if marked {
                        [c[0], c[1], c[2], 255]
                    } else {
                        [0, 0, 0, 0]
                    }
                })
                .collect()
        } else {
            let (color_type, pixels) = decode_samples(image, w, h)?;
            match color_type {
                PngColorType::Gray => pixels.iter().flat_map(|&v| [v, v, v, 255]).collect(),
                PngColorType::Rgb => pixels
                    .as_chunks::<3>()
                    .0
                    .iter()
                    .flat_map(|p| [p[0], p[1], p[2], 255])
                    .collect(),
            }
        };
        if rgba.len() != w as usize * h as usize * 4 {
            return None;
        }

        // A soft mask of the image's own size gives each pixel's opacity.
        if let Some(smask) = image.smask.as_deref() {
            if smask.width == Some(w) && smask.height == Some(h) {
                if let Some((PngColorType::Gray, alpha)) = decode_samples(smask, w, h) {
                    for (px, a) in rgba.as_chunks_mut::<4>().0.iter_mut().zip(alpha) {
                        px[3] = a;
                    }
                }
            }
        }
        // Premultiply.
        for px in rgba.as_chunks_mut::<4>().0 {
            let a = u16::from(px[3]);
            for c in &mut px[..3] {
                *c = ((u16::from(*c) * a + 127) / 255) as u8;
            }
        }
        Pixmap::from_vec(rgba, IntSize::from_wh(w, h)?)
    }
}

/// Average `pixmap` down by whole factors while it is at least twice the `(width, height)`
/// in device pixels it will be drawn at, so each output pixel is the mean of the samples it
/// covers.
fn downsample(pixmap: Pixmap, target: (f32, f32)) -> Pixmap {
    let (w, h) = (pixmap.width(), pixmap.height());
    let fx = ((w as f32 / target.0.max(1.0)).floor() as u32).max(1);
    let fy = ((h as f32 / target.1.max(1.0)).floor() as u32).max(1);
    if fx < 2 && fy < 2 {
        return pixmap;
    }
    let (nw, nh) = (w.div_ceil(fx), h.div_ceil(fy));
    let src = pixmap.data();
    let mut out = vec![0u8; nw as usize * nh as usize * 4];
    for oy in 0..nh {
        for ox in 0..nw {
            let mut sum = [0u32; 4];
            let mut count = 0u32;
            for y in oy * fy..((oy + 1) * fy).min(h) {
                let row = (y * w) as usize * 4;
                for x in ox * fx..((ox + 1) * fx).min(w) {
                    let i = row + x as usize * 4;
                    for c in 0..4 {
                        sum[c] += u32::from(src[i + c]);
                    }
                    count += 1;
                }
            }
            let o = (oy * nw + ox) as usize * 4;
            for c in 0..4 {
                out[o + c] = ((sum[c] + count / 2) / count) as u8;
            }
        }
    }
    // Averages of premultiplied pixels stay premultiplied.
    IntSize::from_wh(nw, nh)
        .and_then(|size| Pixmap::from_vec(out, size))
        .unwrap_or(pixmap)
}

/// An image's samples as 8-bit gray or RGB pixels, decoding the codecs this reads.
fn decode_samples(image: &RawXObject, w: u32, h: u32) -> Option<(PngColorType, Vec<u8>)> {
    match image.filter.as_deref() {
        Some("DCTDecode") => return decode_jpeg(&image.data, w, h),
        Some("JPXDecode") => return decode_jpx(image, w, h),
        _ => {}
    }
    let samples = plain_samples(image, w, h)?;
    png_encode::image_pixels(
        w,
        h,
        image.bits_per_component?,
        image.color.as_ref()?,
        image.decode.as_deref(),
        &samples,
    )
}

/// A JPEG 2000 image's pixels. Its codestream says its size and, when the image dictionary
/// gives no `/ColorSpace`, its color space; `/Decode` is ignored (ISO 32000-1 §8.9.5.3). A
/// codestream of another size than the dictionary declares is not drawn, as other decoders
/// fail it, and an alpha channel is dropped (`/SMaskInData` is not read).
fn decode_jpx(image: &RawXObject, w: u32, h: u32) -> Option<(PngColorType, Vec<u8>)> {
    let decoded = super::jpx::decode(&image.data, image.color.as_ref(), |width, height| {
        png_encode::decodable(width, height, 4)
    })?;
    if (decoded.width, decoded.height) != (w, h) {
        return None;
    }
    png_encode::image_pixels(w, h, 8, &decoded.color, None, &decoded.samples)
}

/// The image's samples as an image without a codec stores them: its data, or the 1-bit
/// samples a bi-level codec (CCITT, JBIG2) decodes to. `None` for any other codec.
fn plain_samples(image: &RawXObject, w: u32, h: u32) -> Option<std::borrow::Cow<'_, [u8]>> {
    use std::borrow::Cow;
    match (image.filter.as_deref(), &image.codec_parms) {
        (None, _) => Some(Cow::Borrowed(&image.data)),
        (Some("CCITTFaxDecode"), Some(CodecParms::Ccitt(parms))) => {
            super::ccitt::decode(&image.data, parms, w, h).map(Cow::Owned)
        }
        (Some("JBIG2Decode"), Some(CodecParms::Jbig2 { globals })) => {
            super::jbig2::decode(&image.data, globals.as_deref(), w, h).map(Cow::Owned)
        }
        _ => None,
    }
}

/// Decode a JPEG to gray or RGB pixels of the size the image dictionary declares.
fn decode_jpeg(data: &[u8], w: u32, h: u32) -> Option<(PngColorType, Vec<u8>)> {
    use zune_jpeg::zune_core::bytestream::ZCursor;
    use zune_jpeg::zune_core::colorspace::ColorSpace;
    use zune_jpeg::zune_core::options::DecoderOptions;

    // The dictionary's size bounds what the decoder may allocate: a JPEG whose own header says
    // more is refused before its pixels are, not after.
    if !png_encode::decodable(w, h, 3) {
        return None;
    }
    let options = DecoderOptions::default()
        .jpeg_set_out_colorspace(ColorSpace::RGB)
        .set_max_width(w as usize)
        .set_max_height(h as usize);
    let mut decoder = zune_jpeg::JpegDecoder::new_with_options(ZCursor::new(data), options);
    let pixels = decoder.decode().ok()?;
    let info = decoder.info()?;
    if (u32::from(info.width), u32::from(info.height)) != (w, h) {
        return None;
    }
    (pixels.len() == w as usize * h as usize * 3).then_some((PngColorType::Rgb, pixels))
}

/// A paint whose shader is `shading`, its shading space mapped by `transform` into the space
/// the paint is drawn in. `None` when its colors do not convert or its geometry is degenerate.
///
/// The colors are the shading's samples as gradient stops. Where the shading does not extend
/// past an end, a transparent stop sits exactly at that end and the first color just inside
/// it, so padding past the end paints nothing.
fn shading_paint(shading: &Shading, transform: Transform, alpha: f32) -> Option<Paint<'static>> {
    let space = PaintSpace::of_image_space(&shading.color);
    let colors: Option<Vec<Rgb>> = shading.samples.iter().map(|s| space.color(s)).collect();
    let colors = colors?;
    let last = colors.len().checked_sub(1)?;
    // How far inside an end its first color starts when the end is not extended.
    const EDGE: f32 = 1.0 / 4096.0;
    let color = |c: Rgb| Color::from_rgba(c.0, c.1, c.2, alpha.clamp(0.0, 1.0));
    let mut stops = Vec::with_capacity(colors.len() + 2);
    if !shading.extend.0 {
        stops.push(GradientStop::new(0.0, Color::TRANSPARENT));
    }
    for (i, &c) in colors.iter().enumerate() {
        let mut pos = i as f32 / last.max(1) as f32;
        if i == 0 && !shading.extend.0 {
            pos = EDGE;
        }
        if i == last && !shading.extend.1 {
            pos = 1.0 - EDGE;
        }
        stops.push(GradientStop::new(pos, color(c)?));
    }
    if !shading.extend.1 {
        stops.push(GradientStop::new(1.0, Color::TRANSPARENT));
    }
    let shader: Shader<'static> = match shading.geometry {
        ShadingGeometry::Axial { from, to } => LinearGradient::new(
            Point::from_xy(from.0, from.1),
            Point::from_xy(to.0, to.1),
            stops,
            SpreadMode::Pad,
            transform,
        )?,
        ShadingGeometry::Radial { from, to } => RadialGradient::new(
            Point::from_xy(from.0, from.1),
            from.2,
            Point::from_xy(to.0, to.1),
            to.2,
            stops,
            SpreadMode::Pad,
            transform,
        )?,
    };
    Some(Paint {
        shader,
        anti_alias: true,
        ..Paint::default()
    })
}

fn solid(color: Rgb, alpha: f32) -> Paint<'static> {
    let mut paint = Paint::default();
    paint.set_color_rgba8(
        (color.0 * 255.0).round() as u8,
        (color.1 * 255.0).round() as u8,
        (color.2 * 255.0).round() as u8,
        (alpha.clamp(0.0, 1.0) * 255.0).round() as u8,
    );
    paint.anti_alias = true;
    paint
}

/// How much `t` scales lengths, on average over its two axes.
fn transform_scale(t: &Transform) -> f32 {
    let sx = t.sx.hypot(t.ky);
    let sy = t.kx.hypot(t.sy);
    ((sx * sy).abs()).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(w: u32, h: u32, rgba: [u8; 4]) -> Pixmap {
        let data: Vec<u8> = (0..w * h).flat_map(|_| rgba).collect();
        Pixmap::from_vec(data, IntSize::from_wh(w, h).unwrap()).unwrap()
    }

    #[test]
    fn an_image_drawn_much_smaller_is_averaged_down() {
        // Alternating black and white columns, drawn at a quarter of their size.
        let mut data = Vec::new();
        for _ in 0..8 {
            for x in 0..8 {
                let v = if x % 2 == 0 { 0 } else { 255 };
                data.extend([v, v, v, 255]);
            }
        }
        let pixmap = Pixmap::from_vec(data, IntSize::from_wh(8, 8).unwrap()).unwrap();
        let small = downsample(pixmap, (2.0, 2.0));
        assert_eq!((small.width(), small.height()), (2, 2));
        // Each output pixel is the mean of black and white: mid-gray, not one or the other.
        assert!(small
            .data()
            .chunks(4)
            .all(|p| p[0].abs_diff(128) <= 1 && p[3] == 255));
    }

    #[test]
    fn an_image_near_its_drawn_size_is_left_alone() {
        let small = downsample(solid(10, 10, [9, 9, 9, 255]), (6.0, 6.0));
        assert_eq!((small.width(), small.height()), (10, 10));
    }
}
