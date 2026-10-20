//! Page rasterization: what lands where, in what color, and what is reported as not painted.
#![cfg(feature = "raster")]

mod common;

use common::{assemble, page_pdf, page_pdf_with_xobjects, stream_object};
use unpdf::parser::raster::{PageRegion, RasterOptions, RasteredPage};
use unpdf::parser::PdfParser;

const A4: &str = "/MediaBox[0 0 595 842]";
/// One pixel per point.
const PT: RasterOptions = RasterOptions {
    dpi: 72.0,
    region: PageRegion::Crop,
};

fn render(pdf: &[u8], options: &RasterOptions) -> RasteredPage {
    PdfParser::from_bytes(pdf)
        .unwrap()
        .render_page(1, options)
        .unwrap()
}

/// The RGB of the pixel at `(x, y)`, counted from the top-left.
fn px(page: &RasteredPage, x: u32, y: u32) -> [u8; 3] {
    let i = ((y * page.width + x) * 4) as usize;
    [page.rgba[i], page.rgba[i + 1], page.rgba[i + 2]]
}

fn near(a: [u8; 3], b: [u8; 3]) -> bool {
    a.iter().zip(b).all(|(&x, y)| x.abs_diff(y) <= 6)
}

const WHITE: [u8; 3] = [255, 255, 255];
const RED: [u8; 3] = [255, 0, 0];

#[test]
fn a_filled_rectangle_lands_where_the_page_puts_it() {
    // A red square 100 pt from the left and 100 pt from the bottom.
    let page = render(&page_pdf("", A4, b"1 0 0 rg 100 100 50 50 re f"), &PT);
    assert_eq!((page.width, page.height), (595, 842));
    assert!(near(px(&page, 125, 842 - 125), RED));
    assert!(near(px(&page, 10, 10), WHITE));
    assert!(near(px(&page, 125, 842 - 200), WHITE));
    assert!(page.gaps.is_empty(), "{:?}", page.gaps);
}

#[test]
fn resolution_scales_the_page() {
    let page = render(&page_pdf("", A4, b""), &RasterOptions { dpi: 144.0, ..PT });
    assert_eq!((page.width, page.height), (1190, 1684));
}

#[test]
fn a_rotated_page_is_painted_as_displayed() {
    // /Rotate 90 turns the page clockwise: its bottom-left corner goes to the top-left.
    let page = render(
        &page_pdf("", &format!("{A4}/Rotate 90"), b"1 0 0 rg 0 0 50 50 re f"),
        &PT,
    );
    assert_eq!((page.width, page.height), (842, 595));
    assert!(near(px(&page, 25, 25), RED), "{:?}", px(&page, 25, 25));
    assert!(near(px(&page, 842 - 25, 595 - 25), WHITE));
}

#[test]
fn a_page_box_off_the_origin_starts_at_its_own_corner() {
    let page = render(
        &page_pdf(
            "",
            "/MediaBox[100 100 695 942]",
            b"1 0 0 rg 100 100 50 50 re f",
        ),
        &PT,
    );
    assert_eq!((page.width, page.height), (595, 842));
    // The box's lower-left corner is the bottom-left pixel.
    assert!(near(px(&page, 25, 842 - 25), RED));
}

#[test]
fn the_clip_bounds_what_is_painted() {
    // Clip to a 100 x 100 square, then fill the whole page.
    let page = render(
        &page_pdf(
            "",
            A4,
            b"q 200 200 100 100 re W n 1 0 0 rg 0 0 595 842 re f Q",
        ),
        &PT,
    );
    assert!(near(px(&page, 250, 842 - 250), RED));
    assert!(near(px(&page, 100, 842 - 100), WHITE));
}

#[test]
fn a_stroke_draws_the_outline_only() {
    let page = render(
        &page_pdf("", A4, b"0 0 1 RG 4 w 100 100 m 300 100 l S"),
        &PT,
    );
    assert!(near(px(&page, 200, 842 - 100), [0, 0, 255]));
    assert!(near(px(&page, 200, 842 - 120), WHITE));
}

#[test]
fn colors_in_each_device_space() {
    let page = render(
        &page_pdf(
            "",
            A4,
            b"0.5 g 0 0 100 100 re f  0 1 0 rg 100 0 100 100 re f  0 0 0 1 k 200 0 100 100 re f",
        ),
        &PT,
    );
    assert!(near(px(&page, 50, 842 - 50), [128, 128, 128]));
    assert!(near(px(&page, 150, 842 - 50), [0, 255, 0]));
    assert!(near(px(&page, 250, 842 - 50), [0, 0, 0]));
}

#[test]
fn a_full_page_image_covers_the_page() {
    // `/Im0` is one gray (0x80) pixel.
    let page = render(&page_pdf("", A4, b"q 595 0 0 842 0 0 cm /Im0 Do Q"), &PT);
    assert!(near(px(&page, 300, 400), [128, 128, 128]));
    assert!(page.gaps.is_empty());
}

#[test]
fn an_inline_image_is_painted_and_the_content_after_it_still_is() {
    // 2 x 1 RGB, red then blue, written raw between `ID` and `EI`; a green square after it.
    let mut content = b"q 200 0 0 100 50 50 cm BI /W 2 /H 1 /CS /RGB /BPC 8 ID ".to_vec();
    content.extend_from_slice(&[0xFF, 0x00, 0x00, 0x00, 0x00, 0xFF]);
    content.extend_from_slice(b"\nEI Q 0 1 0 rg 400 400 50 50 re f");
    let page = render(&page_pdf("", A4, &content), &PT);
    assert!(
        near(px(&page, 100, 842 - 100), RED),
        "{:?}",
        px(&page, 100, 742)
    );
    assert!(
        near(px(&page, 200, 842 - 100), [0, 0, 255]),
        "{:?}",
        px(&page, 200, 742)
    );
    assert!(near(px(&page, 425, 842 - 425), [0, 255, 0]));
    assert!(page.gaps.is_empty(), "{:?}", page.gaps);
}

#[test]
fn an_inline_image_reads_abbreviated_filters_and_a_dictionary_ending_against_id() {
    // One gray 0x80 pixel, hex-encoded; the dictionary's last value ends in `]`, right
    // against `ID`.
    let page = render(
        &page_pdf(
            "",
            A4,
            b"q 100 0 0 100 0 0 cm BI /W 1 /H 1 /CS /G /BPC 8 /F [/AHx]ID 80> EI Q",
        ),
        &PT,
    );
    assert!(
        near(px(&page, 50, 842 - 50), [128, 128, 128]),
        "{:?}",
        px(&page, 50, 792)
    );
    assert!(page.gaps.is_empty(), "{:?}", page.gaps);
}

#[test]
fn an_inline_stencil_mask_paints_the_fill_color() {
    // 2 x 1 stencil: the left bit (0) marks.
    let mut content = b"1 0 0 rg q 200 0 0 100 50 50 cm BI /W 2 /H 1 /IM true ID ".to_vec();
    content.push(0b0100_0000);
    content.extend_from_slice(b" EI Q");
    let page = render(&page_pdf("", A4, &content), &PT);
    assert!(
        near(px(&page, 100, 842 - 100), RED),
        "{:?}",
        px(&page, 100, 742)
    );
    assert!(
        near(px(&page, 200, 842 - 100), WHITE),
        "{:?}",
        px(&page, 200, 742)
    );
    assert!(page.gaps.is_empty(), "{:?}", page.gaps);
}

#[test]
fn an_inline_image_that_does_not_read_is_reported() {
    let page = render(
        &page_pdf("", A4, b"BI /W 1 /H 1 /CS /Unknown /BPC 8 ID x EI"),
        &PT,
    );
    assert_eq!(
        page.gaps.images + page.gaps.inline_images,
        1,
        "{:?}",
        page.gaps
    );
}

/// Red to blue, linearly.
const RED_TO_BLUE: &str = "<</FunctionType 2/Domain[0 1]/C0[1 0 0]/C1[0 0 1]/N 1>>";

/// Shading resources inherited from the Pages node: `/Sh` is `shading`.
fn with_shading(shading: &str) -> String {
    format!("/Resources<</Shading<</Sh {shading}>>>>")
}

#[test]
fn an_axial_shading_runs_from_its_start_color_to_its_end_color() {
    let shading = format!(
        "<</ShadingType 2/ColorSpace/DeviceRGB/Coords[0 0 595 0]/Function {RED_TO_BLUE}\
          /Extend[true true]>>"
    );
    let page = render(&page_pdf(&with_shading(&shading), A4, b"/Sh sh"), &PT);
    assert!(near(px(&page, 2, 400), RED), "{:?}", px(&page, 2, 400));
    assert!(
        near(px(&page, 592, 400), [0, 0, 255]),
        "{:?}",
        px(&page, 592, 400)
    );
    let mid = px(&page, 297, 400);
    assert!(near(mid, [128, 0, 128]), "{mid:?}");
    assert!(page.gaps.is_empty(), "{:?}", page.gaps);
}

#[test]
fn a_shading_that_does_not_extend_stops_at_its_ends() {
    let shading = format!(
        "<</ShadingType 2/ColorSpace/DeviceRGB/Coords[100 0 200 0]/Function {RED_TO_BLUE}>>"
    );
    let page = render(&page_pdf(&with_shading(&shading), A4, b"/Sh sh"), &PT);
    assert!(near(px(&page, 50, 400), WHITE), "{:?}", px(&page, 50, 400));
    assert!(
        near(px(&page, 150, 400), [128, 0, 128]),
        "{:?}",
        px(&page, 150, 400)
    );
    assert!(
        near(px(&page, 250, 400), WHITE),
        "{:?}",
        px(&page, 250, 400)
    );
}

#[test]
fn a_radial_shading_runs_between_its_circles() {
    // Red at the centre to blue at radius 100, nothing past it.
    let shading = format!(
        "<</ShadingType 3/ColorSpace/DeviceRGB/Coords[300 400 0 300 400 100]/Function {RED_TO_BLUE}>>"
    );
    let page = render(&page_pdf(&with_shading(&shading), A4, b"/Sh sh"), &PT);
    assert!(
        near(px(&page, 300, 842 - 400), RED),
        "{:?}",
        px(&page, 300, 442)
    );
    assert!(
        near(px(&page, 350, 842 - 400), [128, 0, 128]),
        "{:?}",
        px(&page, 350, 442)
    );
    assert!(
        near(px(&page, 300, 842 - 550), WHITE),
        "{:?}",
        px(&page, 300, 292)
    );
}

#[test]
fn a_shading_pattern_fills_a_path() {
    // The pattern's shading is placed in the page's default space, not the current one.
    let resources = format!(
        "/Resources<</Pattern<</P0<</PatternType 2/Shading<</ShadingType 2/ColorSpace/DeviceRGB\
          /Coords[100 0 300 0]/Function {RED_TO_BLUE}/Extend[true true]>>>>>>>>"
    );
    let page = render(
        &page_pdf(
            &resources,
            A4,
            b"q 1 0 0 1 50 0 cm /Pattern cs /P0 scn 50 100 200 200 re f Q",
        ),
        &PT,
    );
    // The rectangle spans x 100..300 on the page; the gradient runs over the same span.
    assert!(
        near(px(&page, 102, 842 - 200), RED),
        "{:?}",
        px(&page, 102, 642)
    );
    assert!(
        near(px(&page, 200, 842 - 200), [128, 0, 128]),
        "{:?}",
        px(&page, 200, 642)
    );
    assert!(
        near(px(&page, 200, 842 - 50), WHITE),
        "{:?}",
        px(&page, 200, 792)
    );
    assert!(page.gaps.is_empty(), "{:?}", page.gaps);
}

#[test]
fn a_shading_pattern_inside_a_form_is_placed_in_the_form_s_space() {
    // The form is moved 100 pt right; its pattern runs over x 0..200 of the form's space,
    // which is 100..300 on the page.
    let content = b"/Pattern cs /P0 scn 0 100 200 200 re f";
    let form = stream_object(
        &format!(
            "<</Type/XObject/Subtype/Form/BBox[0 0 595 842]/Matrix[1 0 0 1 100 0]\
              /Resources<</Pattern<</P0<</PatternType 2/Shading<</ShadingType 2\
              /ColorSpace/DeviceRGB/Coords[0 0 200 0]/Function {RED_TO_BLUE}/Extend[true true]>>\
              >>>>>>/Length {}>>",
            content.len()
        ),
        content,
    );
    let page = render(
        &page_pdf_with_xobjects(A4, b"/Fm Do", vec![("Fm", form)]),
        &PT,
    );
    assert!(
        near(px(&page, 200, 842 - 200), [128, 0, 128]),
        "{:?}",
        px(&page, 200, 642)
    );
    assert!(
        near(px(&page, 102, 842 - 200), RED),
        "{:?}",
        px(&page, 102, 642)
    );
    assert!(page.gaps.is_empty(), "{:?}", page.gaps);
}

#[test]
fn a_stitched_function_shades_each_part() {
    // Red to blue over the first half, then blue to green.
    let function = "<</FunctionType 3/Domain[0 1]/Bounds[0.5]/Encode[0 1 0 1]/Functions[\
                    <</FunctionType 2/Domain[0 1]/C0[1 0 0]/C1[0 0 1]/N 1>>\
                    <</FunctionType 2/Domain[0 1]/C0[0 0 1]/C1[0 1 0]/N 1>>]>>";
    let shading = format!(
        "<</ShadingType 2/ColorSpace/DeviceRGB/Coords[0 0 400 0]/Function {function}\
          /Extend[true true]>>"
    );
    let page = render(&page_pdf(&with_shading(&shading), A4, b"/Sh sh"), &PT);
    assert!(
        near(px(&page, 200, 400), [0, 0, 255]),
        "{:?}",
        px(&page, 200, 400)
    );
    assert!(
        near(px(&page, 300, 400), [0, 128, 128]),
        "{:?}",
        px(&page, 300, 400)
    );
}

#[test]
fn a_shading_with_a_bounding_box_paints_only_inside_it() {
    // A red-to-blue axis across the page, bounded to x 100..200, under a moved origin.
    let shading = format!(
        "<</ShadingType 2/ColorSpace/DeviceRGB/Coords[0 0 400 0]/Function {RED_TO_BLUE}\
          /Extend[true true]/BBox[100 0 200 842]>>"
    );
    let page = render(
        &page_pdf(&with_shading(&shading), A4, b"q 1 0 0 1 100 0 cm /Sh sh Q"),
        &PT,
    );
    // Shading space is user space: x 100..200 there is 200..300 on the page.
    assert!(
        near(px(&page, 150, 400), WHITE),
        "{:?}",
        px(&page, 150, 400)
    );
    assert!(
        near(px(&page, 300 - 2, 400), [128, 0, 128]),
        "{:?}",
        px(&page, 298, 400)
    );
    assert!(
        near(px(&page, 350, 400), WHITE),
        "{:?}",
        px(&page, 350, 400)
    );
}

#[test]
fn a_shading_this_does_not_read_is_reported() {
    // A free-form triangle mesh (type 4).
    let page = render(
        &page_pdf(
            &with_shading("<</ShadingType 4/ColorSpace/DeviceRGB>>"),
            A4,
            b"/Sh sh",
        ),
        &PT,
    );
    assert_eq!(page.gaps.shadings, 1, "{:?}", page.gaps);
}

/// A page showing `content` with `/T3`, a Type 3 font whose 1000-unit glyph space is a text
/// space unit: `a` (code 97) is an uncolored square that sets blue in vain, `b` a colored
/// green half-width bar.
fn type3_pdf(content: &[u8]) -> Vec<u8> {
    let a = b"1000 0 0 0 1000 1000 d1 0 0 1 rg 0 0 1000 1000 re f";
    let b = b"1000 0 d0 0 1 0 rg 0 0 500 1000 re f";
    assemble(vec![
        b"<</Type/Catalog/Pages 2 0 R>>".to_vec(),
        b"<</Type/Pages/Kids[3 0 R]/Count 1>>".to_vec(),
        b"<</Type/Page/Parent 2 0 R/MediaBox[0 0 595 842]/Resources<</Font<</T3 5 0 R>>>>\
          /Contents 4 0 R>>"
            .to_vec(),
        stream_object(&format!("<</Length {}>>", content.len()), content),
        b"<</Type/Font/Subtype/Type3/FontBBox[0 0 1000 1000]/FontMatrix[0.001 0 0 0.001 0 0]\
          /CharProcs<</a 6 0 R/b 7 0 R>>/Encoding<</Type/Encoding/Differences[97/a/b]>>\
          /FirstChar 97/LastChar 98/Widths[1000 1000]/Resources<<>>>>"
            .to_vec(),
        stream_object(&format!("<</Length {}>>", a.len()), a),
        stream_object(&format!("<</Length {}>>", b.len()), b),
    ])
}

#[test]
fn a_type3_font_s_glyphs_are_drawn_by_their_procedures() {
    let page = render(
        &type3_pdf(b"1 0 0 rg BT /T3 50 Tf 100 100 Td (ab) Tj (a) Tj ET 300 300 10 10 re f"),
        &PT,
    );
    // `a`: the current fill (red), not the blue its procedure sets.
    assert!(
        near(px(&page, 125, 842 - 125), RED),
        "{:?}",
        px(&page, 125, 717)
    );
    // `b`: its own green, half a glyph wide, after `a`'s 50 pt advance.
    assert!(
        near(px(&page, 160, 842 - 125), [0, 255, 0]),
        "{:?}",
        px(&page, 160, 717)
    );
    assert!(
        near(px(&page, 190, 842 - 125), WHITE),
        "{:?}",
        px(&page, 190, 717)
    );
    // The next `a` is red again: `b`'s color stayed inside its procedure.
    assert!(
        near(px(&page, 225, 842 - 125), RED),
        "{:?}",
        px(&page, 225, 717)
    );
    // So is what is painted after the text.
    assert!(
        near(px(&page, 305, 842 - 305), RED),
        "{:?}",
        px(&page, 305, 537)
    );
    assert!(page.gaps.is_empty(), "{:?}", page.gaps);
}

#[test]
fn a_code_with_no_glyph_procedure_is_reported() {
    let page = render(&type3_pdf(b"BT /T3 50 Tf 100 100 Td (z) Tj ET"), &PT);
    assert_eq!(page.gaps.text_runs, 1, "{:?}", page.gaps);
}

#[test]
fn a_jpeg_image_is_decoded() {
    let jpeg = include_bytes!("fixtures/solid_red_4x2.jpg");
    let image = stream_object(
        &format!(
            "<</Type/XObject/Subtype/Image/Width 4/Height 2/ColorSpace/DeviceRGB\
              /BitsPerComponent 8/Filter/DCTDecode/Length {}>>",
            jpeg.len()
        ),
        jpeg,
    );
    let page = render(
        &page_pdf_with_xobjects(A4, b"q 200 0 0 100 50 50 cm /J Do Q", vec![("J", image)]),
        &PT,
    );
    assert!(
        near(px(&page, 150, 842 - 100), [200, 30, 40]),
        "{:?}",
        px(&page, 150, 742)
    );
    assert!(near(px(&page, 20, 20), WHITE));
    assert!(page.gaps.is_empty(), "{:?}", page.gaps);
}

/// A page drawing a 16 x 8 CMYK JPEG over x 50..250, y 50..150, with `entries` in its dictionary.
fn cmyk_jpeg_page(jpeg: &[u8], entries: &str) -> RasteredPage {
    let image = stream_object(
        &format!(
            "<</Type/XObject/Subtype/Image/Width 16/Height 8/ColorSpace/DeviceCMYK              /BitsPerComponent 8/Filter/DCTDecode{entries}/Length {}>>",
            jpeg.len()
        ),
        jpeg,
    );
    render(
        &page_pdf_with_xobjects(A4, b"q 200 0 0 100 50 50 cm /J Do Q", vec![("J", image)]),
        &PT,
    )
}

#[test]
fn a_cmyk_jpeg_is_read_through_its_decode_array() {
    // Written the way Photoshop does: stored inverted (255 = no ink), left white, right cyan.
    let jpeg = include_bytes!("fixtures/halves_white_cyan_16x8_cmyk.jpg");
    let page = cmyk_jpeg_page(jpeg, "/Decode[1 0 1 0 1 0 1 0]");
    assert!(
        near(px(&page, 80, 842 - 100), WHITE),
        "{:?}",
        px(&page, 80, 742)
    );
    assert!(
        near(px(&page, 220, 842 - 100), [0, 255, 255]),
        "{:?}",
        px(&page, 220, 742)
    );
    assert!(page.gaps.is_empty(), "{:?}", page.gaps);
}

#[test]
fn a_ycck_jpeg_without_a_decode_array_keeps_its_light_and_dark() {
    // Adobe transform 2 (YCCK): left (Y 200, Cb 128, Cr 128, K 0), right (100, 128, 128, 128).
    // libjpeg's conversion gives C = M = Y' = 255 - Y, read as ink amounts: gray 200, and
    // 100 under half a K ink (about 50).
    let jpeg = include_bytes!("fixtures/halves_ycck_16x8.jpg");
    let page = cmyk_jpeg_page(jpeg, "");
    assert!(
        near(px(&page, 80, 842 - 100), [200, 200, 200]),
        "{:?}",
        px(&page, 80, 742)
    );
    assert!(
        near(px(&page, 220, 842 - 100), [50, 50, 50]),
        "{:?}",
        px(&page, 220, 742)
    );
}

/// A page drawing the 4 x 2 red JPEG over x 50..250, y 50..150 with `/SMask` object `mask`
/// (the second XObject, object 6).
fn masked_jpeg_page(mask: Vec<u8>) -> RasteredPage {
    let jpeg = include_bytes!("fixtures/solid_red_4x2.jpg");
    let image = stream_object(
        &format!(
            "<</Type/XObject/Subtype/Image/Width 4/Height 2/ColorSpace/DeviceRGB\
              /BitsPerComponent 8/Filter/DCTDecode/SMask 6 0 R/Length {}>>",
            jpeg.len()
        ),
        jpeg,
    );
    render(
        &page_pdf_with_xobjects(
            A4,
            b"q 200 0 0 100 50 50 cm /J Do Q",
            vec![("J", image), ("M", mask)],
        ),
        &PT,
    )
}

#[test]
fn an_image_soft_mask_coded_as_a_jpeg_is_applied() {
    // White on the left half, black on the right: the right half of the image is not drawn.
    let jpeg = include_bytes!("fixtures/halves_white_black_4x2_gray.jpg");
    let mask = stream_object(
        &format!(
            "<</Type/XObject/Subtype/Image/Width 4/Height 2/ColorSpace/DeviceGray\
              /BitsPerComponent 8/Filter/DCTDecode/Length {}>>",
            jpeg.len()
        ),
        jpeg,
    );
    let page = masked_jpeg_page(mask);
    assert!(
        near(px(&page, 100, 842 - 100), [200, 30, 40]),
        "{:?}",
        px(&page, 100, 742)
    );
    assert!(
        near(px(&page, 200, 842 - 100), WHITE),
        "{:?}",
        px(&page, 200, 742)
    );
    assert!(page.gaps.is_empty(), "{:?}", page.gaps);
}

#[test]
fn an_image_soft_mask_of_another_size_covers_the_same_square() {
    // 2 x 2 mask over a 4 x 2 image: opaque top-left and bottom-right, clear elsewhere.
    let mask = stream_object(
        "<</Type/XObject/Subtype/Image/Width 2/Height 2/ColorSpace/DeviceGray\
          /BitsPerComponent 8/Length 4>>",
        &[255, 0, 0, 255],
    );
    let page = masked_jpeg_page(mask);
    let at = |x: u32, y: u32| px(&page, x, 842 - y);
    assert!(
        near(at(100, 130), [200, 30, 40]),
        "top-left {:?}",
        at(100, 130)
    );
    assert!(near(at(200, 130), WHITE), "top-right {:?}", at(200, 130));
    assert!(near(at(100, 70), WHITE), "bottom-left {:?}", at(100, 70));
    assert!(
        near(at(200, 70), [200, 30, 40]),
        "bottom-right {:?}",
        at(200, 70)
    );
    assert!(page.gaps.is_empty(), "{:?}", page.gaps);
}

/// A spot color that is green at full tint and white at none, approximated in CMYK:
/// full tint is (1, 0, .71, .43), which is RGB (0, 145, 42).
const SPOT_GREEN: &str = "[/Separation/Spot/DeviceCMYK    <</FunctionType 2/Domain[0 1]/C0[0 0 0 0]/C1[1 0 .71 .43]/N 1>>]";
const GREEN: [u8; 3] = [0, 145, 42];

fn spot_resources(space: &str) -> String {
    format!("/Resources<</ColorSpace<</S0 {space}>>>>")
}

#[test]
fn a_separation_color_comes_out_in_its_alternate_space() {
    let page = render(
        &page_pdf(
            &spot_resources(SPOT_GREEN),
            A4,
            b"/S0 cs 1 scn 100 100 100 100 re f 0.5 scn 300 100 100 100 re f 0 scn 500 100 50 50 re f",
        ),
        &PT,
    );
    let at = |x: u32, y: u32| px(&page, x, 842 - y);
    assert!(near(at(150, 150), GREEN), "full {:?}", at(150, 150));
    // Half tint: CMYK (.5, 0, .355, .215).
    assert!(
        near(at(350, 150), [100, 200, 129]),
        "half {:?}",
        at(350, 150)
    );
    assert!(near(at(520, 120), WHITE), "none {:?}", at(520, 120));
    assert!(page.gaps.is_empty(), "{:?}", page.gaps);
}

#[test]
fn a_separation_space_starts_at_full_tint() {
    let page = render(
        &page_pdf(
            &spot_resources(SPOT_GREEN),
            A4,
            b"/S0 cs 100 100 100 100 re f /S0 CS 20 w 300 100 m 400 100 l S",
        ),
        &PT,
    );
    assert!(near(px(&page, 150, 842 - 150), GREEN));
    // The stroke has its own space, and it too starts at full tint.
    assert!(near(px(&page, 350, 842 - 100), GREEN));
}

#[test]
fn a_separation_over_gray_follows_a_stitched_transform() {
    // Gray falls from 1 to .5 over tints [0, .5] and from .5 to 0 over [.5, 1]: tint .25 is .75.
    let space = "[/Separation/Black/DeviceGray<</FunctionType 3/Domain[0 1]/Bounds[0.5]        /Encode[0 1 0 1]/Functions[        <</FunctionType 2/Domain[0 1]/C0[1]/C1[0.5]/N 1>>        <</FunctionType 2/Domain[0 1]/C0[0.5]/C1[0]/N 1>>]>>]";
    let page = render(
        &page_pdf(
            &spot_resources(space),
            A4,
            b"/S0 cs 0.25 scn 100 100 100 100 re f 1 scn 300 100 100 100 re f",
        ),
        &PT,
    );
    let at = |x: u32, y: u32| px(&page, x, 842 - y);
    assert!(near(at(150, 150), [191, 191, 191]), "{:?}", at(150, 150));
    assert!(near(at(350, 150), [0, 0, 0]), "{:?}", at(350, 150));
}

#[test]
fn a_shading_in_a_separation_space_runs_through_its_transform() {
    let shading = format!(
        "<</ShadingType 2/ColorSpace{SPOT_GREEN}/Coords[0 0 595 0]          /Function<</FunctionType 2/Domain[0 1]/C0[0]/C1[1]/N 1>>/Extend[true true]>>"
    );
    let page = render(&page_pdf(&with_shading(&shading), A4, b"/Sh sh"), &PT);
    assert!(near(px(&page, 2, 400), WHITE), "{:?}", px(&page, 2, 400));
    assert!(
        near(px(&page, 592, 400), GREEN),
        "{:?}",
        px(&page, 592, 400)
    );
    assert!(page.gaps.is_empty(), "{:?}", page.gaps);
}

#[test]
fn an_image_in_a_separation_space_is_converted_through_its_transform() {
    // Two samples: tint 0 (white) and tint 1 (the spot green).
    let image = stream_object(
        &format!(
            "<</Type/XObject/Subtype/Image/Width 2/Height 1/ColorSpace{SPOT_GREEN}              /BitsPerComponent 8/Length 2>>"
        ),
        &[0, 255],
    );
    let page = render(
        &page_pdf_with_xobjects(A4, b"q 200 0 0 100 50 50 cm /J Do Q", vec![("J", image)]),
        &PT,
    );
    assert!(
        near(px(&page, 70, 842 - 100), WHITE),
        "{:?}",
        px(&page, 70, 742)
    );
    assert!(
        near(px(&page, 230, 842 - 100), GREEN),
        "{:?}",
        px(&page, 230, 742)
    );
    assert!(page.gaps.is_empty(), "{:?}", page.gaps);
}

/// A 16 x 8 fax image, white on the left half and black on the right, as MuPDF renders it:
/// Group 4, and Group 3 one-dimensional with end-of-line codes (both written by libtiff).
const G4_HALVES: &[u8] = &[0x33, 0x17, 0xff, 0xf0, 0x01, 0x00, 0x10];
const G3_HALVES: &[u8] = &[
    0x00, 0x19, 0x8a, 0x00, 0x33, 0x14, 0x00, 0x66, 0x28, 0x00, 0xcc, 0x50, 0x01, 0x98, 0xa0, 0x03,
    0x31, 0x40, 0x06, 0x62, 0x80, 0x0c, 0xc5,
];

/// A page drawing a 16 x 8 fax image over x 100..260, y 100..180.
fn fax_page(entries: &str, data: &[u8]) -> RasteredPage {
    let image = stream_object(
        &format!(
            "<</Type/XObject/Subtype/Image/Width 16/Height 8{entries}\
              /Filter/CCITTFaxDecode/Length {}>>",
            data.len()
        ),
        data,
    );
    render(
        &page_pdf_with_xobjects(
            A4,
            b"1 0 0 rg q 160 0 0 80 100 100 cm /F Do Q",
            vec![("F", image)],
        ),
        &PT,
    )
}

#[test]
fn a_ccitt_image_is_decoded() {
    for (name, entries, data) in [
        (
            "Group 4",
            "/ColorSpace/DeviceGray/BitsPerComponent 1/DecodeParms<</K -1/Columns 16/Rows 8>>",
            G4_HALVES,
        ),
        (
            "Group 3",
            "/ColorSpace/DeviceGray/BitsPerComponent 1\
             /DecodeParms<</K 0/Columns 16/Rows 8/EndOfLine true>>",
            G3_HALVES,
        ),
    ] {
        let page = fax_page(entries, data);
        assert!(near(px(&page, 120, 842 - 140), WHITE), "{name}: left");
        assert!(near(px(&page, 240, 842 - 140), [0, 0, 0]), "{name}: right");
        assert!(page.gaps.is_empty(), "{name}: {:?}", page.gaps);
    }
}

#[test]
fn black_is_1_inverts_a_ccitt_image() {
    let page = fax_page(
        "/ColorSpace/DeviceGray/BitsPerComponent 1\
         /DecodeParms<</K -1/Columns 16/Rows 8/BlackIs1 true>>",
        G4_HALVES,
    );
    assert!(near(px(&page, 120, 842 - 140), [0, 0, 0]));
    assert!(near(px(&page, 240, 842 - 140), WHITE));
}

#[test]
fn a_ccitt_stencil_mask_paints_the_fill_color() {
    // A stencil marks where its samples are 0: the black half, in the fill color (red).
    let page = fax_page(
        "/ImageMask true/DecodeParms<</K -1/Columns 16/Rows 8>>",
        G4_HALVES,
    );
    assert!(near(px(&page, 120, 842 - 140), WHITE));
    assert!(near(px(&page, 240, 842 - 140), RED));
    assert!(page.gaps.is_empty(), "{:?}", page.gaps);
}

#[test]
fn a_ccitt_image_that_does_not_decode_is_reported() {
    let page = fax_page(
        "/ColorSpace/DeviceGray/BitsPerComponent 1/DecodeParms<</K -1/Columns 16/Rows 8>>",
        &[0x00, 0x00, 0x00],
    );
    assert_eq!(page.gaps.images, 1, "{:?}", page.gaps);
}

/// A page that sets the fill alpha to 0.5 (`/Half`) and paints the form `/Fm`, whose content
/// sets its own alpha back to 1 (`/Opaque`) and fills x 100..300, y 100..300 red. `group` is
/// the form's `/Group` entry, if any.
fn group_page(group: &str) -> Vec<u8> {
    let content = b"/Opaque gs 1 0 0 rg 100 100 200 200 re f";
    assemble(vec![
        b"<</Type/Catalog/Pages 2 0 R>>".to_vec(),
        b"<</Type/Pages/Kids[3 0 R]/Count 1>>".to_vec(),
        format!(
            "<</Type/Page/Parent 2 0 R{A4}/Resources<</XObject<</Fm 5 0 R>>\
             /ExtGState<</Half<</ca 0.5>>>>>>/Contents 4 0 R>>"
        )
        .into_bytes(),
        stream_object("<</Length 13>>", b"/Half gs /Fm Do"),
        stream_object(
            &format!(
                "<</Type/XObject/Subtype/Form/BBox[0 0 595 842]{group}\
                  /Resources<</ExtGState<</Opaque<</ca 1>>>>>>/Length {}>>",
                content.len()
            ),
            content,
        ),
    ])
}

#[test]
fn a_transparency_group_is_composited_with_the_alpha_it_is_painted_with() {
    // The alpha inside the group does not undo the one the group is painted with: half red.
    let page = render(&group_page("/Group<</S/Transparency>>"), &PT);
    let got = px(&page, 200, 842 - 200);
    assert!(near(got, [255, 128, 128]), "{got:?}");
}

#[test]
fn a_form_that_is_not_a_group_paints_with_the_alpha_its_content_sets() {
    let page = render(&group_page(""), &PT);
    let got = px(&page, 200, 842 - 200);
    assert!(near(got, RED), "{got:?}");
}

/// A 16 x 8 JBIG2 page (an immediate generic region, MMR-coded), white on the left half and
/// black on the right, as MuPDF renders it.
const JBIG2_HALVES: &[u8] = include_bytes!("fixtures/halves_white_black_16x8.jb2");

fn jbig2_page(entries: &str) -> RasteredPage {
    let image = stream_object(
        &format!(
            "<</Type/XObject/Subtype/Image/Width 16/Height 8{entries}\
              /Filter/JBIG2Decode/Length {}>>",
            JBIG2_HALVES.len()
        ),
        JBIG2_HALVES,
    );
    render(
        &page_pdf_with_xobjects(
            A4,
            b"1 0 0 rg q 160 0 0 80 100 100 cm /B Do Q",
            vec![("B", image)],
        ),
        &PT,
    )
}

#[test]
fn a_jbig2_image_is_decoded() {
    let page = jbig2_page("/ColorSpace/DeviceGray/BitsPerComponent 1");
    assert!(
        near(px(&page, 120, 842 - 140), WHITE),
        "{:?}",
        px(&page, 120, 702)
    );
    assert!(
        near(px(&page, 240, 842 - 140), [0, 0, 0]),
        "{:?}",
        px(&page, 240, 702)
    );
    assert!(page.gaps.is_empty(), "{:?}", page.gaps);
}

#[test]
fn a_jbig2_stencil_mask_paints_the_fill_color() {
    let page = jbig2_page("/ImageMask true");
    assert!(near(px(&page, 120, 842 - 140), WHITE));
    assert!(near(px(&page, 240, 842 - 140), RED));
    assert!(page.gaps.is_empty(), "{:?}", page.gaps);
}

/// A page drawing a JPEG 2000 image over x 100..260, y 100..180.
fn jpx_page(entries: &str, data: &[u8]) -> RasteredPage {
    let image = stream_object(
        &format!(
            "<</Type/XObject/Subtype/Image{entries}/BitsPerComponent 8/Filter/JPXDecode/Length {}>>",
            data.len()
        ),
        data,
    );
    render(
        &page_pdf_with_xobjects(A4, b"q 160 0 0 80 100 100 cm /J Do Q", vec![("J", image)]),
        &PT,
    )
}

#[test]
fn a_jpeg_2000_image_is_decoded_in_its_own_color_space() {
    // A JP2 file, red on the left half and blue on the right (as MuPDF renders it); the
    // dictionary gives no /ColorSpace, so the file's own (sRGB) is used.
    let page = jpx_page(
        "/Width 8/Height 4",
        include_bytes!("fixtures/halves_red_blue_8x4.jp2"),
    );
    assert!(
        near(px(&page, 120, 842 - 140), RED),
        "{:?}",
        px(&page, 120, 702)
    );
    assert!(
        near(px(&page, 240, 842 - 140), [0, 0, 255]),
        "{:?}",
        px(&page, 240, 702)
    );
    assert!(page.gaps.is_empty(), "{:?}", page.gaps);
}

#[test]
fn a_jpeg_2000_codestream_is_read_in_the_dictionary_s_color_space() {
    // A bare codestream, black on the left and white on the right, under /DeviceGray.
    let page = jpx_page(
        "/Width 8/Height 4/ColorSpace/DeviceGray",
        include_bytes!("fixtures/halves_black_white_8x4.j2k"),
    );
    assert!(near(px(&page, 120, 842 - 140), [0, 0, 0]));
    assert!(near(px(&page, 240, 842 - 140), WHITE));
    assert!(page.gaps.is_empty(), "{:?}", page.gaps);
}

#[test]
fn a_jpeg_2000_image_of_another_size_than_declared_is_reported() {
    let page = jpx_page(
        "/Width 16/Height 4",
        include_bytes!("fixtures/halves_red_blue_8x4.jp2"),
    );
    assert_eq!(page.gaps.images, 1, "{:?}", page.gaps);
}

#[test]
fn a_stencil_mask_paints_the_fill_color_where_it_marks() {
    // 2 x 1 stencil: bit 0 (left) marks, bit 1 (right) does not.
    let stencil = stream_object(
        "<</Type/XObject/Subtype/Image/Width 2/Height 1/ImageMask true/Length 1>>",
        &[0b0100_0000],
    );
    let page = render(
        &page_pdf_with_xobjects(
            A4,
            b"0 0 1 rg q 200 0 0 100 100 100 cm /S Do Q",
            vec![("S", stencil)],
        ),
        &PT,
    );
    assert!(
        near(px(&page, 150, 842 - 150), [0, 0, 255]),
        "left half marks"
    );
    assert!(
        near(px(&page, 250, 842 - 150), WHITE),
        "right half does not"
    );
}

#[test]
fn text_is_reported_as_not_painted_except_invisible_text() {
    // A symbolic font of unknown design, not embedded: nothing can stand in for it.
    let page = render(
        &font_page(
            "<</Type/Font/Subtype/TrueType/BaseFont/UnpdfPictographs/FontDescriptor 6 0 R>>",
            vec![b"<</Type/FontDescriptor/FontName/UnpdfPictographs/Flags 4/FontBBox[0 0 1000 800]\
                   /ItalicAngle 0/Ascent 800/Descent -200/CapHeight 800/StemV 80>>"
                .to_vec()],
            b"BT /F1 12 Tf 72 700 Td (Shown text) Tj ET BT 3 Tr /F1 12 Tf 72 680 Td (OCR layer) Tj ET",
        ),
        &PT,
    );
    assert_eq!(page.gaps.text_runs, 1);
    assert_eq!(page.substituted_text_runs, 0);
}

/// Whether any pixel of the box `x0..x1` × `y0..y1` (from the top-left) is dark.
fn inked(page: &RasteredPage, x0: u32, x1: u32, y0: u32, y1: u32) -> bool {
    (y0..y1).any(|y| (x0..x1).any(|x| px(page, x, y).iter().all(|&c| c < 128)))
}

/// `Hello` in Helvetica at 40 pt, its baseline at y 700: ink between x 72 and about 170.
const HELLO_HELVETICA: &[u8] = b"BT /F1 40 Tf 72 700 Td (Hello) Tj ET";

#[cfg(feature = "standard-fonts")]
#[test]
fn a_standard_font_that_is_not_embedded_is_drawn_in_a_stand_in_face() {
    let page = render(&page_pdf("", A4, HELLO_HELVETICA), &PT);
    assert!(
        inked(&page, 72, 175, 842 - 732, 842 - 700),
        "the word is painted"
    );
    assert!(
        !inked(&page, 180, 300, 842 - 732, 842 - 700),
        "and ends where its widths do"
    );
    assert!(page.gaps.is_empty(), "{:?}", page.gaps);
    assert_eq!(page.substituted_text_runs, 1);
}

#[cfg(feature = "standard-fonts")]
#[test]
fn a_stand_in_glyph_is_fitted_to_the_width_the_font_gives_it() {
    // `/Widths` makes every glyph 2000 units, nearly three times Helvetica's 722-unit `H`;
    // the fit is capped at twice the face's own width.
    let widths = vec!["2000"; 224].join(" ");
    let pdf = font_page(
        &format!(
            "<</Type/Font/Subtype/TrueType/BaseFont/Arial/FirstChar 32/LastChar 255\
              /Widths[{widths}]/Encoding/WinAnsiEncoding/FontDescriptor 6 0 R>>"
        ),
        vec![
            b"<</Type/FontDescriptor/FontName/Arial/Flags 32/FontBBox[0 0 1000 800]\
               /ItalicAngle 0/Ascent 800/Descent -200/CapHeight 700/StemV 80>>"
                .to_vec(),
        ],
        b"BT /F1 40 Tf 72 700 Td (H) Tj ET",
    );
    let page = render(&pdf, &PT);
    // Drawn at its own width the `H` would stop near x 98; stretched, its ink reaches on.
    assert!(
        inked(&page, 120, 150, 842 - 730, 842 - 700),
        "the glyph is stretched"
    );
    assert_eq!(page.substituted_text_runs, 1);
}

/// Whether some pixel in `x0..x1` × `y0..y1` is within `near` of `color`.
#[cfg(feature = "standard-fonts")]
fn has_color(page: &RasteredPage, x0: u32, x1: u32, y0: u32, y1: u32, color: [u8; 3]) -> bool {
    (y0..y1).any(|y| (x0..x1).any(|x| near(px(page, x, y), color)))
}

#[cfg(feature = "standard-fonts")]
#[test]
fn text_in_a_shading_pattern_is_painted_in_its_colors() {
    // Red up to x 90, blue from x 140, across `Hello`.
    let resources = format!(
        "/Resources<</Pattern<</P0<</PatternType 2/Shading<</ShadingType 2/ColorSpace/DeviceRGB\
          /Coords[90 0 140 0]/Function {RED_TO_BLUE}/Extend[true true]>>>>>>>>"
    );
    let page = render(
        &page_pdf(
            &resources,
            A4,
            b"/Pattern cs /P0 scn BT /F1 40 Tf 72 700 Td (Hello) Tj ET",
        ),
        &PT,
    );
    assert!(
        has_color(&page, 72, 90, 842 - 732, 842 - 700, RED),
        "red at the start"
    );
    assert!(
        has_color(&page, 140, 175, 842 - 732, 842 - 700, [0, 0, 255]),
        "blue at the end"
    );
    assert!(page.gaps.is_empty(), "{:?}", page.gaps);
}

/// A page whose resources hold the tiling pattern `/P0` (an indirect stream, as a pattern with a
/// content stream must be), the color space `/CS0` = `[/Pattern /DeviceRGB]`, and `/F1`
/// Helvetica.
fn tiling_pdf(pattern_entries: &str, cell: &[u8], content: &[u8]) -> Vec<u8> {
    assemble(vec![
        b"<</Type/Catalog/Pages 2 0 R>>".to_vec(),
        b"<</Type/Pages/Kids[3 0 R]/Count 1>>".to_vec(),
        format!(
            "<</Type/Page/Parent 2 0 R{A4}/Resources<</Pattern<</P0 5 0 R>>\
             /ColorSpace<</CS0[/Pattern/DeviceRGB]>>/Font<</F1 6 0 R>>>>/Contents 4 0 R>>"
        )
        .into_bytes(),
        stream_object(&format!("<</Length {}>>", content.len()), content),
        stream_object(
            &format!(
                "<</Type/Pattern/PatternType 1/TilingType 1{pattern_entries}/Length {}>>",
                cell.len()
            ),
            cell,
        ),
        b"<</Type/Font/Subtype/Type1/BaseFont/Helvetica>>".to_vec(),
    ])
}

/// A red cell 10 pt square, repeated every 20 pt: its content paints the whole 20 pt step,
/// but the `/BBox` clips it to its own 10.
const RED_CHECKS: &str = "/PaintType 1/BBox[0 0 10 10]/XStep 20/YStep 20/Resources<<>>";
const RED_CELL: &[u8] = b"1 0 0 rg 0 0 20 20 re f";

#[test]
fn a_tiling_pattern_repeats_its_cell_across_a_path() {
    let page = render(
        &tiling_pdf(
            RED_CHECKS,
            RED_CELL,
            b"/Pattern cs /P0 scn 100 100 200 200 re f",
        ),
        &PT,
    );
    // Pattern space is the page's: a cell at every multiple of 20, red over its first 10.
    for (x, y, expected) in [
        (105, 105, RED),
        (125, 105, RED),
        (245, 285, RED),
        (115, 105, WHITE), // past the cell's box, inside its step
        (105, 115, WHITE),
        (105, 50, WHITE), // outside the path
    ] {
        let got = px(&page, x, 842 - y - 1);
        assert!(near(got, expected), "({x}, {y}): {got:?}");
    }
    assert!(page.gaps.is_empty(), "{:?}", page.gaps);
}

#[test]
fn a_tiling_pattern_is_placed_by_its_matrix() {
    let page = render(
        &tiling_pdf(
            &format!("{RED_CHECKS}/Matrix[1 0 0 1 5 0]"),
            RED_CELL,
            b"/Pattern cs /P0 scn 100 100 200 200 re f",
        ),
        &PT,
    );
    // Moved 5 pt right: cells start at x = 5, 25, ...
    assert!(near(px(&page, 108, 842 - 106), RED));
    assert!(near(px(&page, 102, 842 - 106), WHITE));
    assert!(near(px(&page, 116, 842 - 106), WHITE));
}

#[test]
fn an_uncolored_tiling_pattern_paints_its_cell_in_the_color_given_with_it() {
    // The cell's own color (green) is ignored: an uncolored pattern is a shape.
    let page = render(
        &tiling_pdf(
            "/PaintType 2/BBox[0 0 10 10]/XStep 20/YStep 20/Resources<<>>",
            b"0 1 0 rg 0 0 10 10 re f",
            b"/CS0 cs 0 0 1 /P0 scn 100 100 200 200 re f",
        ),
        &PT,
    );
    assert!(
        near(px(&page, 105, 842 - 106), [0, 0, 255]),
        "{:?}",
        px(&page, 105, 736)
    );
    assert!(near(px(&page, 115, 842 - 106), WHITE));
    assert!(page.gaps.is_empty(), "{:?}", page.gaps);
}

#[test]
fn a_tiling_pattern_painting_itself_ends_and_is_reported() {
    // The cell fills with the pattern itself: the nesting stops, and what it left is a gap.
    let page = render(
        &tiling_pdf(
            "/PaintType 1/BBox[0 0 10 10]/XStep 10/YStep 10/Resources<<>>",
            b"/Pattern cs /P0 scn 0 0 10 10 re f",
            b"/Pattern cs /P0 scn 100 100 200 200 re f",
        ),
        &PT,
    );
    assert!(page.gaps.shadings > 0, "{:?}", page.gaps);
}

#[cfg(feature = "standard-fonts")]
#[test]
fn text_in_a_tiling_pattern_is_painted_in_its_cells() {
    let page = render(
        &tiling_pdf(
            "/PaintType 1/BBox[0 0 10 10]/XStep 10/YStep 10/Resources<<>>",
            b"1 0 0 rg 0 0 10 10 re f",
            b"/Pattern cs /P0 scn BT /F1 40 Tf 72 700 Td (Hello) Tj ET",
        ),
        &PT,
    );
    assert!(
        has_color(&page, 72, 175, 842 - 732, 842 - 700, RED),
        "the glyphs are red"
    );
    assert!(page.gaps.is_empty(), "{:?}", page.gaps);
}

#[cfg(not(feature = "standard-fonts"))]
#[test]
fn without_stand_in_faces_a_font_that_is_not_embedded_is_a_gap() {
    let page = render(&page_pdf("", A4, HELLO_HELVETICA), &PT);
    assert_eq!(page.gaps.text_runs, 1);
    assert_eq!(page.substituted_text_runs, 0);
}

#[test]
fn the_png_is_the_page() {
    let page = render(&page_pdf("", A4, b"1 0 0 rg 0 0 10 10 re f"), &PT);
    let png = page.to_png();
    assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
    assert_eq!(u32::from_be_bytes(png[16..20].try_into().unwrap()), 595);
    assert_eq!(u32::from_be_bytes(png[20..24].try_into().unwrap()), 842);
}

#[test]
fn a_page_out_of_range_is_an_error() {
    let parser = PdfParser::from_bytes(&page_pdf("", A4, b"")).unwrap();
    assert!(matches!(
        parser.render_page(2, &PT),
        Err(unpdf::Error::PageOutOfRange(2, 1))
    ));
}

// ---------------------------------------------------------------------------------------
// Text. The fixture fonts (tests/fixtures/make_square_fonts.py) draw `A` as a filled square
// over 100..900 x 0..800 of a 1000-unit em; `space` has no outline.

const SQUARE_TTF: &[u8] = include_bytes!("fixtures/square.ttf");
const SQUARE_CFF: &[u8] = include_bytes!("fixtures/square.cff");

/// One page drawing `content` with `/F1` = `font` (object 5); `extra` objects follow from 6.
fn font_page(font: &str, extra: Vec<Vec<u8>>, content: &[u8]) -> Vec<u8> {
    let mut objects = vec![
        b"<</Type/Catalog/Pages 2 0 R>>".to_vec(),
        b"<</Type/Pages/Kids[3 0 R]/Count 1>>".to_vec(),
        format!("<</Type/Page/Parent 2 0 R{A4}/Resources<</Font<</F1 5 0 R>>>>/Contents 4 0 R>>")
            .into_bytes(),
        stream_object(&format!("<</Length {}>>", content.len()), content),
        font.as_bytes().to_vec(),
    ];
    objects.extend(extra);
    common::assemble(objects)
}

fn font_file(subtype: &str, data: &[u8]) -> Vec<u8> {
    stream_object(&format!("<<{subtype}/Length {}>>", data.len()), data)
}

/// `A` at 100 pt, its square over x 110..190 and y 100..180 of the page.
const SHOW_A: &[u8] = b"BT /F1 100 Tf 100 100 Td (A) Tj ET";

fn assert_square_painted(page: &RasteredPage) {
    assert!(
        near(px(page, 150, 842 - 140), [0, 0, 0]),
        "inside the glyph: {:?}",
        px(page, 150, 702)
    );
    assert!(near(px(page, 105, 842 - 140), WHITE), "left of it");
    assert!(near(px(page, 150, 842 - 190), WHITE), "above it");
    assert_eq!(page.gaps.text_runs, 0, "{:?}", page.gaps);
}

#[test]
fn a_simple_truetype_font_is_drawn() {
    let pdf = font_page(
        "<</Type/Font/Subtype/TrueType/BaseFont/UnpdfSquare/FirstChar 32/LastChar 65\
          /Widths[250 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 1000]\
          /Encoding/WinAnsiEncoding/FontDescriptor 6 0 R>>",
        vec![
            b"<</Type/FontDescriptor/FontName/UnpdfSquare/Flags 32/FontBBox[0 0 1000 800]\
               /ItalicAngle 0/Ascent 800/Descent -200/CapHeight 800/StemV 80/FontFile2 7 0 R>>"
                .to_vec(),
            font_file("", SQUARE_TTF),
        ],
        SHOW_A,
    );
    assert_square_painted(&render(&pdf, &PT));
}

#[test]
fn a_bare_cff_font_is_drawn() {
    let pdf = font_page(
        "<</Type/Font/Subtype/Type1/BaseFont/UnpdfSquareCFF/FirstChar 65/LastChar 65\
          /Widths[1000]/FontDescriptor 6 0 R>>",
        vec![
            b"<</Type/FontDescriptor/FontName/UnpdfSquareCFF/Flags 32/FontBBox[0 0 1000 800]\
               /ItalicAngle 0/Ascent 800/Descent -200/CapHeight 800/StemV 80/FontFile3 7 0 R>>"
                .to_vec(),
            font_file("/Subtype/Type1C", SQUARE_CFF),
        ],
        SHOW_A,
    );
    assert_square_painted(&render(&pdf, &PT));
}

/// A Type 1 program whose `A` is the fixture square, built the way the format stores it:
/// charstrings encrypted with key 4330 inside a private part encrypted with key 55665.
fn square_type1(encoding: &str) -> Vec<u8> {
    fn encrypt(plain: &[u8], key: u16) -> Vec<u8> {
        let mut r = key;
        [0u8; 4]
            .iter()
            .chain(plain)
            .map(|&p| {
                let c = p ^ (r >> 8) as u8;
                r = u16::from(c)
                    .wrapping_add(r)
                    .wrapping_mul(52845)
                    .wrapping_add(22719);
                c
            })
            .collect()
    }
    // hsbw 0 1000 · rmoveto 100 0 · hlineto 800 · vlineto 800 · hlineto -800 · closepath ·
    // endchar — numbers in the charstring encoding.
    let a = [
        139, 250, 124, 13, 239, 139, 21, 249, 180, 6, 249, 180, 7, 253, 180, 6, 9, 14,
    ];
    let notdef = [139, 139, 13, 14];
    let mut private = b"dup /Private 5 dict dup begin /lenIV 4 def /Subrs 0 array\n".to_vec();
    private.extend(b"2 index /CharStrings 2 dict dup begin\n");
    for (name, cs) in [("/.notdef", &notdef[..]), ("/A", &a[..])] {
        let e = encrypt(cs, 4330);
        private.extend(format!("{name} {} RD ", e.len()).as_bytes());
        private.extend(e);
        private.extend(b" ND\n");
    }
    private.extend(b"end end\nmark currentfile closefile\n");
    let mut program = format!(
        "%!FontType1-1.0: UnpdfSquareT1\n/FontMatrix [0.001 0 0 0.001 0 0] readonly def\n\
         /Encoding {encoding} def\ncurrentfile eexec\n"
    )
    .into_bytes();
    program.extend(encrypt(&private, 55665));
    program
}

fn type1_page(font_encoding: &str, program_encoding: &str, content: &[u8]) -> Vec<u8> {
    let program = square_type1(program_encoding);
    font_page(
        &format!(
            "<</Type/Font/Subtype/Type1/BaseFont/UnpdfSquareT1/FirstChar 65/LastChar 66\
              /Widths[1000 1000]{font_encoding}/FontDescriptor 6 0 R>>"
        ),
        vec![
            b"<</Type/FontDescriptor/FontName/UnpdfSquareT1/Flags 32/FontBBox[0 0 1000 800]\
               /ItalicAngle 0/Ascent 800/Descent -200/CapHeight 800/StemV 80/FontFile 7 0 R>>"
                .to_vec(),
            stream_object(
                &format!(
                    "<</Length1 {} /Length2 0 /Length3 0/Length {}>>",
                    program.len(),
                    program.len()
                ),
                &program,
            ),
        ],
        content,
    )
}

#[test]
fn a_type1_font_is_drawn() {
    let pdf = type1_page("/Encoding/WinAnsiEncoding", "StandardEncoding", SHOW_A);
    assert_square_painted(&render(&pdf, &PT));
}

#[test]
fn a_type1_font_without_an_encoding_uses_its_own() {
    // The program's encoding puts `A` at code 66 (`B`); the PDF font names no encoding.
    let pdf = type1_page(
        "",
        "256 array 0 1 255 {1 index exch /.notdef put} for dup 66 /A put readonly",
        b"BT /F1 100 Tf 100 100 Td (B) Tj ET",
    );
    assert_square_painted(&render(&pdf, &PT));
}

#[test]
fn a_type1_glyph_is_found_by_its_differences_name() {
    let pdf = type1_page(
        "/Encoding<</Differences[66/A]>>",
        "StandardEncoding",
        b"BT /F1 100 Tf 100 100 Td (B) Tj ET",
    );
    assert_square_painted(&render(&pdf, &PT));
}

#[test]
fn a_composite_identity_font_is_drawn_by_glyph_index() {
    // Identity-H: the two-byte code is the CID, and CIDToGIDMap /Identity makes it the glyph
    // index — glyph 2 is `A`.
    let pdf = font_page(
        "<</Type/Font/Subtype/Type0/BaseFont/UnpdfSquare/Encoding/Identity-H\
          /DescendantFonts[6 0 R]>>",
        vec![
            b"<</Type/Font/Subtype/CIDFontType2/BaseFont/UnpdfSquare\
               /CIDSystemInfo<</Registry(Adobe)/Ordering(Identity)/Supplement 0>>\
               /FontDescriptor 7 0 R/DW 1000/CIDToGIDMap/Identity>>"
                .to_vec(),
            b"<</Type/FontDescriptor/FontName/UnpdfSquare/Flags 4/FontBBox[0 0 1000 800]\
               /ItalicAngle 0/Ascent 800/Descent -200/CapHeight 800/StemV 80/FontFile2 8 0 R>>"
                .to_vec(),
            font_file("", SQUARE_TTF),
        ],
        b"BT /F1 100 Tf 100 100 Td <0002> Tj ET",
    );
    assert_square_painted(&render(&pdf, &PT));
}

/// Glyphs advance by the widths extraction measures with: two `A`s 1000 units wide sit side
/// by side, the second starting 100 pt after the first.
#[test]
fn glyphs_advance_by_their_widths() {
    let pdf = font_page(
        "<</Type/Font/Subtype/TrueType/BaseFont/UnpdfSquare/FirstChar 65/LastChar 65\
          /Widths[1000]/Encoding/WinAnsiEncoding/FontDescriptor 6 0 R>>",
        vec![
            b"<</Type/FontDescriptor/FontName/UnpdfSquare/Flags 32/FontBBox[0 0 1000 800]\
               /ItalicAngle 0/Ascent 800/Descent -200/CapHeight 800/StemV 80/FontFile2 7 0 R>>"
                .to_vec(),
            font_file("", SQUARE_TTF),
        ],
        b"BT /F1 100 Tf 100 100 Td (AA) Tj ET",
    );
    let page = render(&pdf, &PT);
    assert!(near(px(&page, 150, 842 - 140), [0, 0, 0]));
    assert!(
        near(px(&page, 200, 842 - 140), WHITE),
        "the gap between the squares"
    );
    assert!(
        near(px(&page, 250, 842 - 140), [0, 0, 0]),
        "the second square"
    );
}

#[test]
fn invisible_text_paints_nothing_and_is_no_gap() {
    let pdf = font_page(
        "<</Type/Font/Subtype/TrueType/BaseFont/UnpdfSquare/FirstChar 65/LastChar 65\
          /Widths[1000]/Encoding/WinAnsiEncoding/FontDescriptor 6 0 R>>",
        vec![
            b"<</Type/FontDescriptor/FontName/UnpdfSquare/Flags 32/FontBBox[0 0 1000 800]\
               /ItalicAngle 0/Ascent 800/Descent -200/CapHeight 800/StemV 80/FontFile2 7 0 R>>"
                .to_vec(),
            font_file("", SQUARE_TTF),
        ],
        b"BT 3 Tr /F1 100 Tf 100 100 Td (A) Tj ET",
    );
    let page = render(&pdf, &PT);
    assert!(near(px(&page, 150, 842 - 140), WHITE));
    assert!(page.gaps.is_empty());
}

/// The crop box is what a viewer shows and the default region; the media box is the whole
/// sheet.
#[test]
fn the_crop_box_is_painted_unless_the_media_box_is_asked_for() {
    let pdf = page_pdf(
        "",
        &format!("{A4}/CropBox[100 100 300 400]"),
        b"1 0 0 rg 100 100 20 20 re f",
    );
    let crop = render(&pdf, &PT);
    assert_eq!((crop.width, crop.height), (200, 300));
    assert!(
        near(px(&crop, 10, 300 - 10), RED),
        "the crop box's corner is the raster's"
    );

    let media = render(
        &pdf,
        &RasterOptions {
            region: PageRegion::Media,
            ..PT
        },
    );
    assert_eq!((media.width, media.height), (595, 842));
}

/// A page whose `/ExtGState` resource `/M` carries `entry` (a `/BM` value) and whose content
/// is `content`; the form `/Fm` (when `content` paints it) fills x 100..200, y 100..200 yellow.
fn blend_page(entry: &str, content: &[u8]) -> Vec<u8> {
    let form = b"1 1 0 rg 100 100 100 100 re f";
    assemble(vec![
        b"<</Type/Catalog/Pages 2 0 R>>".to_vec(),
        b"<</Type/Pages/Kids[3 0 R]/Count 1>>".to_vec(),
        format!(
            "<</Type/Page/Parent 2 0 R{A4}/Resources<</XObject<</Fm 5 0 R>>\
             /ExtGState<</M<</BM{entry}>>>>>>/Contents 4 0 R>>"
        )
        .into_bytes(),
        stream_object(&format!("<</Length {}>>", content.len()), content),
        stream_object(
            &format!(
                "<</Type/XObject/Subtype/Form/BBox[0 0 595 842]/Group<</S/Transparency>>\
                  /Length {}>>",
                form.len()
            ),
            form,
        ),
    ])
}

/// Where the squares of the blend tests are painted.
fn at_square(page: &RasteredPage) -> [u8; 3] {
    px(page, 150, 842 - 150)
}

const YELLOW_OVER: &[u8] = b"/M gs 1 1 0 rg 100 100 100 100 re f";

#[test]
fn a_multiply_highlight_leaves_the_text_under_it_dark() {
    // Black underneath, a yellow rectangle multiplied over it: black stays black. Painted
    // as Normal it would cover it.
    let under = b"0 g 100 100 100 100 re f ";
    let content = [under.as_slice(), YELLOW_OVER].concat();
    let page = render(&blend_page("/Multiply", &content), &PT);
    assert!(near(at_square(&page), [0, 0, 0]), "{:?}", at_square(&page));
    let page = render(&blend_page("/Normal", &content), &PT);
    assert!(
        near(at_square(&page), [255, 255, 0]),
        "{:?}",
        at_square(&page)
    );
    // Over white, multiplying by yellow is yellow.
    let page = render(&blend_page("/Multiply", YELLOW_OVER), &PT);
    assert!(
        near(at_square(&page), [255, 255, 0]),
        "{:?}",
        at_square(&page)
    );
}

#[test]
fn the_separable_and_non_separable_modes_combine_as_the_standard_says() {
    // Red underneath, then a rectangle in a blend mode. The results are §11.3.5's formulas
    // on (1,0,0) and the source (1,1,0) / (0,0,1).
    let red = b"1 0 0 rg 100 100 100 100 re f ";
    let cases: [(&str, &[u8], [u8; 3]); 5] = [
        // Screen: 1 - (1-cb)(1-cs)
        ("/Screen", b"0 0 1 rg", [255, 0, 255]),
        // Darken: min
        ("/Darken", b"1 1 0 rg", [255, 0, 0]),
        // Lighten: max
        ("/Lighten", b"1 1 0 rg", [255, 255, 0]),
        // Difference: |cb - cs|
        ("/Difference", b"1 1 0 rg", [0, 255, 0]),
        // Exclusion: cb + cs - 2 cb cs
        ("/Exclusion", b"0 0 1 rg", [255, 0, 255]),
    ];
    for (mode, color, expected) in cases {
        let content = [red.as_slice(), b"/M gs ", color, b" 100 100 100 100 re f"].concat();
        let page = render(&blend_page(mode, &content), &PT);
        assert!(
            near(at_square(&page), expected),
            "{mode}: {:?}, want {expected:?}",
            at_square(&page)
        );
    }
}

#[test]
fn a_blend_mode_array_applies_its_first_known_name() {
    let under = b"0 g 100 100 100 100 re f ";
    let content = [under.as_slice(), YELLOW_OVER].concat();
    let page = render(&blend_page("[/NotAMode/Multiply/Screen]", &content), &PT);
    assert!(near(at_square(&page), [0, 0, 0]), "{:?}", at_square(&page));
    // A name no mode goes by leaves the paint Normal.
    let page = render(&blend_page("/NotAMode", &content), &PT);
    assert!(
        near(at_square(&page), [255, 255, 0]),
        "{:?}",
        at_square(&page)
    );
}

#[test]
fn a_blend_mode_is_part_of_the_graphics_state() {
    // Set inside q ... Q it ends with the Q: the second rectangle is Normal.
    let content = b"0 g 100 100 100 100 re f q /M gs 1 1 0 rg 100 100 50 100 re f Q \
                    1 1 0 rg 150 100 50 100 re f";
    let page = render(&blend_page("/Multiply", content), &PT);
    assert!(near(px(&page, 125, 842 - 150), [0, 0, 0]));
    assert!(near(px(&page, 175, 842 - 150), [255, 255, 0]));
}

#[test]
fn a_form_painted_with_a_blend_mode_is_blended_as_one_group() {
    // The form paints yellow in Normal (its own state starts Normal); the group is multiplied
    // onto the black under it.
    let content = b"0 g 100 100 100 100 re f /M gs /Fm Do";
    let page = render(&blend_page("/Multiply", content), &PT);
    assert!(near(at_square(&page), [0, 0, 0]), "{:?}", at_square(&page));
    assert!(page.gaps.is_empty(), "{:?}", page.gaps);
}

/// A page whose `/ExtGState` has `/Mk` (the soft mask `smask` describes, its group `/G` the
/// form that paints `mask_content`) and `/Off` (`/SMask /None`), painting `content`.
fn smask_page(smask: &str, mask_content: &[u8], content: &[u8]) -> Vec<u8> {
    smask_page_in(smask, "", mask_content, content)
}

/// [`smask_page`] with `group` added to the mask form's `/Group` dictionary (its `/CS`).
fn smask_page_in(smask: &str, group: &str, mask_content: &[u8], content: &[u8]) -> Vec<u8> {
    assemble(vec![
        b"<</Type/Catalog/Pages 2 0 R>>".to_vec(),
        b"<</Type/Pages/Kids[3 0 R]/Count 1>>".to_vec(),
        format!(
            "<</Type/Page/Parent 2 0 R{A4}/Resources<</ExtGState<</Mk<</SMask\
             <</Type/Mask{smask}/G 5 0 R>>>>/Off<</SMask/None>>>>>>/Contents 4 0 R>>"
        )
        .into_bytes(),
        stream_object(&format!("<</Length {}>>", content.len()), content),
        stream_object(
            &format!(
                "<</Type/XObject/Subtype/Form/BBox[0 0 595 842]/Group<</S/Transparency{group}>>\
                  /Length {}>>",
                mask_content.len()
            ),
            mask_content,
        ),
    ])
}

/// Red over the whole page, painted under the soft mask `/Mk`.
const RED_PAGE: &[u8] = b"/Mk gs 1 0 0 rg 0 0 595 842 re f";
/// The mask group's shape: x 100..200, y 100..300.
const IN_MASK: (u32, u32) = (150, 842 - 200);
const OUT_OF_MASK: (u32, u32) = (400, 842 - 400);

#[test]
fn a_luminosity_soft_mask_shows_the_paint_where_its_group_is_light() {
    let page = render(
        &smask_page("/S/Luminosity", b"1 g 100 100 100 200 re f", RED_PAGE),
        &PT,
    );
    assert!(near(px(&page, IN_MASK.0, IN_MASK.1), RED));
    assert!(near(px(&page, OUT_OF_MASK.0, OUT_OF_MASK.1), WHITE));
    assert!(page.gaps.is_empty(), "{:?}", page.gaps);
}

#[test]
fn a_luminosity_soft_mask_is_as_strong_as_its_groups_luminosity() {
    // Mid gray in the group: half of the red shows.
    let page = render(
        &smask_page("/S/Luminosity", b"0.5 g 100 100 100 200 re f", RED_PAGE),
        &PT,
    );
    let got = px(&page, IN_MASK.0, IN_MASK.1);
    assert!(
        near(got, [255, 128, 128]) || near(got, [255, 127, 127]),
        "{got:?}"
    );
}

#[test]
fn a_luminosity_soft_mask_is_painted_over_its_backdrop() {
    // The group is black where it paints and the backdrop (/BC) is white: the paint shows
    // everywhere but under the group.
    let page = render(
        &smask_page_in(
            "/S/Luminosity/BC[1]",
            "/CS/DeviceGray",
            b"0 g 100 100 100 200 re f",
            RED_PAGE,
        ),
        &PT,
    );
    assert!(near(px(&page, IN_MASK.0, IN_MASK.1), WHITE));
    assert!(near(px(&page, OUT_OF_MASK.0, OUT_OF_MASK.1), RED));
}

#[test]
fn a_backdrop_with_the_wrong_number_of_components_for_the_group_is_black() {
    // One component for a group that has the page's (RGB) space: not a color of that space.
    let page = render(
        &smask_page("/S/Luminosity/BC[1]", b"0 g 100 100 100 200 re f", RED_PAGE),
        &PT,
    );
    assert!(near(px(&page, OUT_OF_MASK.0, OUT_OF_MASK.1), WHITE));
}

#[test]
fn an_alpha_soft_mask_shows_the_paint_where_its_group_paints_anything() {
    // Black is opaque: the alpha mask does not care about the color.
    let page = render(
        &smask_page("/S/Alpha", b"0 g 100 100 100 200 re f", RED_PAGE),
        &PT,
    );
    assert!(near(px(&page, IN_MASK.0, IN_MASK.1), RED));
    assert!(near(px(&page, OUT_OF_MASK.0, OUT_OF_MASK.1), WHITE));
}

#[test]
fn a_soft_mask_ends_with_none_and_with_the_graphics_state_that_set_it() {
    let smask = smask_page(
        "/S/Luminosity",
        b"1 g 100 100 100 200 re f",
        // Masked red; none; then red again at the right; the mask set in q ... Q is gone.
        b"/Mk gs 1 0 0 rg 0 0 300 842 re f /Off gs 400 100 50 50 re f \
          q /Mk gs Q 0 0 1 rg 450 300 50 50 re f",
    );
    let page = render(&smask, &PT);
    assert!(near(px(&page, 425, 842 - 125), RED), "unmasked after /None");
    assert!(
        near(px(&page, 475, 842 - 325), [0, 0, 255]),
        "unmasked after Q"
    );
    assert!(
        near(px(&page, 250, 842 - 400), WHITE),
        "masked red outside the group"
    );
}

#[test]
fn a_soft_mask_whose_group_does_not_decode_is_reported_and_paints_unmasked() {
    let page = render(
        &assemble(vec![
            b"<</Type/Catalog/Pages 2 0 R>>".to_vec(),
            b"<</Type/Pages/Kids[3 0 R]/Count 1>>".to_vec(),
            format!(
                "<</Type/Page/Parent 2 0 R{A4}/Resources<</ExtGState<</Mk<</SMask\
                 <</S/Luminosity/G 5 0 R>>>>>>>>/Contents 4 0 R>>"
            )
            .into_bytes(),
            stream_object(&format!("<</Length {}>>", RED_PAGE.len()), RED_PAGE),
            stream_object(
                "<</Type/XObject/Subtype/Form/BBox[0 0 595 842]/Filter/FlateDecode/Length 4>>",
                b"nope",
            ),
        ]),
        &PT,
    );
    assert_eq!(page.gaps.soft_masks, 1, "{:?}", page.gaps);
    assert!(near(px(&page, OUT_OF_MASK.0, OUT_OF_MASK.1), RED));
}

#[test]
fn a_transfer_function_is_reported_and_the_mask_applied_without_it() {
    let page = render(
        &smask_page(
            "/S/Luminosity/TR/Custom",
            b"1 g 100 100 100 200 re f",
            RED_PAGE,
        ),
        &PT,
    );
    assert_eq!(page.gaps.soft_masks, 1, "{:?}", page.gaps);
    assert!(near(px(&page, IN_MASK.0, IN_MASK.1), RED));
    assert!(near(px(&page, OUT_OF_MASK.0, OUT_OF_MASK.1), WHITE));
}
