//! A one-page document around the fuzzed bytes, so a target reaches the code it is for without
//! first having to fuzz its way through a valid file structure.

/// `objects` numbered from 1, the first one the catalog, with a cross-reference table.
pub fn assemble(objects: &[Vec<u8>]) -> Vec<u8> {
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::with_capacity(objects.len());
    for (i, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
    for offset in offsets {
        out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!(
            "trailer\n<</Size {}/Root 1 0 R>>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    out
}

/// A stream object: `dict` (without `/Length`, which is added) and `data`.
pub fn stream(dict: &str, data: &[u8]) -> Vec<u8> {
    let mut out = format!("<<{dict}/Length {}>>\nstream\n", data.len()).into_bytes();
    out.extend_from_slice(data);
    out.extend_from_slice(b"\nendstream");
    out
}

/// Catalog, page tree, and one page drawing `content` with `/F1` = object 5 and `/Im0` a
/// one-pixel image; `font` is object 5 and `extra` follow it from object 6.
pub fn page(content: &[u8], font: Vec<u8>, extra: Vec<Vec<u8>>) -> Vec<u8> {
    let image_obj = 6 + extra.len();
    let mut objects = vec![
        b"<</Type/Catalog/Pages 2 0 R>>".to_vec(),
        b"<</Type/Pages/Kids[3 0 R]/Count 1>>".to_vec(),
        format!(
            "<</Type/Page/Parent 2 0 R/MediaBox[0 0 300 300]\
             /Resources<</Font<</F1 5 0 R>>/XObject<</Im0 {image_obj} 0 R>>>>/Contents 4 0 R>>"
        )
        .into_bytes(),
        stream("", content),
        font,
    ];
    objects.extend(extra);
    objects.push(stream(
        "/Type/XObject/Subtype/Image/Width 1/Height 1/ColorSpace/DeviceGray/BitsPerComponent 8",
        &[0x80],
    ));
    assemble(&objects)
}
