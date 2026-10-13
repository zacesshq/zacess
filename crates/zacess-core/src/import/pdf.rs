//! Importing PDFs that carry a text layer.
//!
//! The content stream of every page is walked with the PDF graphics state tracked well enough to
//! know where each string lands on the page and how large it is. Those positioned strings go
//! through the shared [layout](super::layout) analysis. A page that draws no text at all has no
//! text layer and is reported for OCR.

use super::layout::{self, Fragment, PageSize};
use super::{ImportError, Imported, Tools, compose, ocr};
use crate::model::{Confidence, Page, Source, SourceFormat, Unit};
use lopdf::content::Content;
use lopdf::{Dictionary, Object, ObjectId};
use std::collections::{HashMap, HashSet};
use std::path::Path;

/// Form XObjects can nest, but not without end.
const MAX_FORM_DEPTH: usize = 8;

pub fn import_pdf(path: &Path) -> Result<Imported, ImportError> {
    import_pdf_with(path, &Tools::default())
}

/// Imports a PDF, running OCR on any page without a text layer when the tools for it are there.
pub fn import_pdf_with(path: &Path, tools: &Tools<'_>) -> Result<Imported, ImportError> {
    let bytes = std::fs::read(path)?;
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("document.pdf");
    let mut text = extract_text(&bytes, name)?;

    let mut recognised = Vec::new();
    if !text.pages_without_text.is_empty()
        && let (Some(engine), Some(rasteriser)) = (tools.ocr, tools.rasteriser)
    {
        let dir = tempfile::Builder::new().prefix("zacess-ocr-").tempdir()?;
        for &number in &text.pages_without_text {
            let image = rasteriser.render_page(path, number, dir.path())?;
            let page = engine.recognise(&image)?;
            let size = text
                .pages
                .iter()
                .find(|p| p.number == number)
                .expect("every page has a size");
            // pixels back to points, from the rendered width when the engine reports one
            let scale = if page.width > 0.0 {
                size.width / page.width
            } else {
                72.0 / rasteriser.dpi as f32
            };
            text.fragments.extend(ocr::fragments(&page, number, scale));
            recognised.push(number);
        }
    }

    let mut imported = text.into_imported();
    imported
        .pages_needing_ocr
        .retain(|p| !recognised.contains(p));
    imported.pages_recognised = recognised;
    Ok(imported)
}

/// Imports from memory. Pages without a text layer are reported rather than recognised, because
/// rendering them needs the file on disk.
pub fn import_pdf_bytes(bytes: &[u8], file_name: &str) -> Result<Imported, ImportError> {
    Ok(extract_text(bytes, file_name)?.into_imported())
}

/// The text layer of a PDF before layout analysis.
#[derive(Debug, Clone, PartialEq)]
pub struct PdfText {
    pub title: String,
    pub file_name: String,
    pub pages: Vec<Page>,
    pub fragments: Vec<Fragment>,
    /// Pages that draw no text at all, numbered from 1.
    pub pages_without_text: Vec<u32>,
}

impl PdfText {
    pub fn into_imported(self) -> Imported {
        let sizes: Vec<PageSize> = self
            .pages
            .iter()
            .map(|p| PageSize {
                number: p.number,
                width: p.width,
                height: p.height,
            })
            .collect();
        let layouts = layout::analyse(&sizes, &self.fragments);
        compose(
            self.title,
            Source {
                file_name: self.file_name,
                format: SourceFormat::Pdf,
            },
            self.pages,
            layouts,
        )
    }
}

/// Walks every page's content stream and collects its positioned text.
pub fn extract_text(bytes: &[u8], file_name: &str) -> Result<PdfText, ImportError> {
    let doc = lopdf::Document::load_mem(bytes).map_err(|e| ImportError::Pdf(e.to_string()))?;
    if doc.is_encrypted() {
        return Err(ImportError::Encrypted);
    }

    let title = doc
        .trailer
        .get(b"Info")
        .ok()
        .and_then(|info| doc.dereference(info).ok())
        .and_then(|(_, info)| info.as_dict().ok())
        .and_then(|info| info.get(b"Title").ok())
        .and_then(|t| lopdf::decode_text_string(t).ok())
        .filter(|t| !t.trim().is_empty())
        .unwrap_or_else(|| file_name.trim_end_matches(".pdf").to_string());

    let mut pages = Vec::new();
    let mut fragments = Vec::new();
    let mut fonts = FontCache::default();

    for (number, page_id) in doc.get_pages() {
        let page_dict = doc
            .get_dictionary(page_id)
            .map_err(|e| ImportError::Pdf(e.to_string()))?;
        let media_box = media_box(&doc, page_dict).unwrap_or([0.0, 0.0, 612.0, 792.0]);
        let (width, height) = (media_box[2] - media_box[0], media_box[3] - media_box[1]);
        pages.push(Page {
            number,
            width,
            height,
            unit: Unit::Pt,
        });

        let (resources, _) = doc
            .get_page_resources(page_id)
            .map_err(|e| ImportError::Pdf(e.to_string()))?;
        let content = doc.get_page_content(page_id);
        let mut walker = Walker {
            doc: &doc,
            fonts: &mut fonts,
            page: number,
            media_box,
            fragments: &mut fragments,
            visited_forms: HashSet::new(),
        };
        walker.walk(&content, resources, Matrix::IDENTITY, 0);
    }

    let pages_without_text = pages
        .iter()
        .map(|p| p.number)
        .filter(|n| {
            !fragments
                .iter()
                .any(|f| f.page == *n && !f.text.trim().is_empty())
        })
        .collect();
    Ok(PdfText {
        title,
        file_name: file_name.to_string(),
        pages,
        fragments,
        pages_without_text,
    })
}

fn media_box(doc: &lopdf::Document, page: &Dictionary) -> Option<[f32; 4]> {
    // MediaBox is inheritable, so walk up through Parent when the page itself has none
    let mut dict = page;
    for _ in 0..32 {
        if let Ok(array) = dict
            .get(b"MediaBox")
            .and_then(|m| doc.dereference(m))
            .and_then(|(_, m)| m.as_array())
        {
            let v: Vec<f32> = array.iter().filter_map(number).collect();
            if v.len() == 4 {
                return Some([
                    v[0].min(v[2]),
                    v[1].min(v[3]),
                    v[0].max(v[2]),
                    v[1].max(v[3]),
                ]);
            }
        }
        dict = dict
            .get(b"Parent")
            .ok()
            .and_then(|p| doc.dereference(p).ok())
            .and_then(|(_, p)| p.as_dict().ok())?;
    }
    None
}

fn number(obj: &Object) -> Option<f32> {
    match obj {
        Object::Integer(i) => Some(*i as f32),
        Object::Real(r) => Some(*r),
        _ => None,
    }
}

/// A PDF transformation matrix `[a b c d e f]`.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Matrix {
    a: f32,
    b: f32,
    c: f32,
    d: f32,
    e: f32,
    f: f32,
}

impl Matrix {
    const IDENTITY: Matrix = Matrix {
        a: 1.0,
        b: 0.0,
        c: 0.0,
        d: 1.0,
        e: 0.0,
        f: 0.0,
    };

    fn from_operands(ops: &[Object]) -> Option<Matrix> {
        let v: Vec<f32> = ops.iter().filter_map(number).collect();
        (v.len() == 6).then(|| Matrix {
            a: v[0],
            b: v[1],
            c: v[2],
            d: v[3],
            e: v[4],
            f: v[5],
        })
    }

    fn translation(tx: f32, ty: f32) -> Matrix {
        Matrix {
            e: tx,
            f: ty,
            ..Matrix::IDENTITY
        }
    }

    /// `self` then `other`, the PDF convention of `self × other`.
    fn then(&self, o: &Matrix) -> Matrix {
        Matrix {
            a: self.a * o.a + self.b * o.c,
            b: self.a * o.b + self.b * o.d,
            c: self.c * o.a + self.d * o.c,
            d: self.c * o.b + self.d * o.d,
            e: self.e * o.a + self.f * o.c + o.e,
            f: self.e * o.b + self.f * o.d + o.f,
        }
    }

    fn apply(&self, x: f32, y: f32) -> (f32, f32) {
        (
            self.a * x + self.c * y + self.e,
            self.b * x + self.d * y + self.f,
        )
    }

    fn x_scale(&self) -> f32 {
        self.a.hypot(self.b)
    }

    fn y_scale(&self) -> f32 {
        self.c.hypot(self.d)
    }
}

#[derive(Debug, Clone)]
struct TextState {
    font: Option<ObjectId>,
    size: f32,
    char_spacing: f32,
    word_spacing: f32,
    horizontal_scale: f32,
    leading: f32,
    rise: f32,
}

impl Default for TextState {
    fn default() -> Self {
        TextState {
            font: None,
            size: 0.0,
            char_spacing: 0.0,
            word_spacing: 0.0,
            horizontal_scale: 1.0,
            leading: 0.0,
            rise: 0.0,
        }
    }
}

/// What the importer needs to know about one font.
#[derive(Debug)]
struct FontInfo {
    bold: bool,
    /// Byte lengths a code can have, from the ToUnicode codespace or the font type.
    code_lengths: Vec<usize>,
    to_unicode: Option<HashMap<u32, String>>,
    simple_decoder: bool,
    widths: HashMap<u32, f32>,
    default_width: f32,
}

#[derive(Default)]
struct FontCache {
    fonts: HashMap<ObjectId, FontInfo>,
    /// Direct font dictionaries have no object id, so they are keyed by resource name per page.
    anonymous: HashMap<(u32, Vec<u8>), FontInfo>,
}

struct Walker<'a> {
    doc: &'a lopdf::Document,
    fonts: &'a mut FontCache,
    page: u32,
    media_box: [f32; 4],
    fragments: &'a mut Vec<Fragment>,
    visited_forms: HashSet<ObjectId>,
}

impl Walker<'_> {
    fn walk(
        &mut self,
        content: &[u8],
        resources: Option<&Dictionary>,
        base_ctm: Matrix,
        depth: usize,
    ) {
        let Ok(content) = Content::decode(content) else {
            return;
        };
        let mut ctm = base_ctm;
        let mut stack: Vec<(Matrix, TextState)> = Vec::new();
        let mut ts = TextState::default();
        let mut tm = Matrix::IDENTITY;
        let mut tlm = Matrix::IDENTITY;

        for op in &content.operations {
            let ops = &op.operands;
            match op.operator.as_str() {
                "q" => stack.push((ctm, ts.clone())),
                "Q" => {
                    if let Some((m, t)) = stack.pop() {
                        ctm = m;
                        ts = t;
                    }
                }
                "cm" => {
                    if let Some(m) = Matrix::from_operands(ops) {
                        ctm = m.then(&ctm);
                    }
                }
                "BT" => {
                    tm = Matrix::IDENTITY;
                    tlm = Matrix::IDENTITY;
                }
                "Tf" => {
                    if let (Some(name), Some(size)) = (
                        ops.first().and_then(|o| o.as_name().ok()),
                        ops.get(1).and_then(number),
                    ) {
                        ts.size = size;
                        ts.font = self.resolve_font(resources, name);
                    }
                }
                "Td" | "TD" => {
                    if let (Some(tx), Some(ty)) =
                        (ops.first().and_then(number), ops.get(1).and_then(number))
                    {
                        if op.operator == "TD" {
                            ts.leading = -ty;
                        }
                        tlm = Matrix::translation(tx, ty).then(&tlm);
                        tm = tlm;
                    }
                }
                "Tm" => {
                    if let Some(m) = Matrix::from_operands(ops) {
                        tlm = m;
                        tm = m;
                    }
                }
                "T*" => {
                    tlm = Matrix::translation(0.0, -ts.leading).then(&tlm);
                    tm = tlm;
                }
                "TL" => ts.leading = ops.first().and_then(number).unwrap_or(ts.leading),
                "Tc" => ts.char_spacing = ops.first().and_then(number).unwrap_or(ts.char_spacing),
                "Tw" => ts.word_spacing = ops.first().and_then(number).unwrap_or(ts.word_spacing),
                "Tz" => {
                    ts.horizontal_scale = ops
                        .first()
                        .and_then(number)
                        .map_or(ts.horizontal_scale, |z| z / 100.0)
                }
                "Ts" => ts.rise = ops.first().and_then(number).unwrap_or(ts.rise),
                "Tj" | "'" | "\"" => {
                    if op.operator != "Tj" {
                        if op.operator == "\"" {
                            ts.word_spacing =
                                ops.first().and_then(number).unwrap_or(ts.word_spacing);
                            ts.char_spacing =
                                ops.get(1).and_then(number).unwrap_or(ts.char_spacing);
                        }
                        tlm = Matrix::translation(0.0, -ts.leading).then(&tlm);
                        tm = tlm;
                    }
                    if let Some(Object::String(bytes, _)) = ops.last() {
                        self.show(bytes, &ts, &mut tm, &ctm, resources);
                    }
                }
                "TJ" => {
                    if let Some(Object::Array(items)) = ops.first() {
                        for item in items {
                            match item {
                                Object::String(bytes, _) => {
                                    self.show(bytes, &ts, &mut tm, &ctm, resources)
                                }
                                other => {
                                    if let Some(adjust) = number(other) {
                                        let tx = -adjust / 1000.0 * ts.size * ts.horizontal_scale;
                                        tm = Matrix::translation(tx, 0.0).then(&tm);
                                    }
                                }
                            }
                        }
                    }
                }
                "Do" => {
                    if depth < MAX_FORM_DEPTH
                        && let Some(name) = ops.first().and_then(|o| o.as_name().ok())
                    {
                        self.draw_form(resources, name, ctm, depth);
                    }
                }
                _ => {}
            }
        }
    }

    fn draw_form(
        &mut self,
        resources: Option<&Dictionary>,
        name: &[u8],
        ctm: Matrix,
        depth: usize,
    ) {
        let Some(resources) = resources else { return };
        let Ok(xobjects) = resources
            .get(b"XObject")
            .and_then(|x| self.doc.dereference(x))
            .and_then(|(_, x)| x.as_dict())
        else {
            return;
        };
        let Ok(reference) = xobjects.get(name).and_then(|x| x.as_reference()) else {
            return;
        };
        if !self.visited_forms.insert(reference) {
            return;
        }
        let Ok(stream) = self.doc.get_object(reference).and_then(|o| o.as_stream()) else {
            return;
        };
        if stream.dict.get(b"Subtype").and_then(|s| s.as_name()).ok() != Some(b"Form") {
            self.visited_forms.remove(&reference);
            return;
        }
        let matrix = stream
            .dict
            .get(b"Matrix")
            .ok()
            .and_then(|m| m.as_array().ok())
            .and_then(|m| Matrix::from_operands(m))
            .unwrap_or(Matrix::IDENTITY);
        let form_resources = stream
            .dict
            .get(b"Resources")
            .ok()
            .and_then(|r| self.doc.dereference(r).ok())
            .and_then(|(_, r)| r.as_dict().ok())
            .unwrap_or(resources);
        if let Ok(content) = stream.decompressed_content() {
            self.walk(&content, Some(form_resources), matrix.then(&ctm), depth + 1);
        }
        self.visited_forms.remove(&reference);
    }

    fn resolve_font(&mut self, resources: Option<&Dictionary>, name: &[u8]) -> Option<ObjectId> {
        let resources = resources?;
        let fonts = resources
            .get(b"Font")
            .ok()
            .and_then(|f| self.doc.dereference(f).ok())
            .and_then(|(_, f)| f.as_dict().ok())?;
        let entry = fonts.get(name).ok()?;
        match entry {
            Object::Reference(id) => {
                let id = *id;
                if !self.fonts.fonts.contains_key(&id)
                    && let Ok(dict) = self.doc.get_dictionary(id)
                {
                    let info = load_font(self.doc, dict);
                    self.fonts.fonts.insert(id, info);
                }
                Some(id)
            }
            Object::Dictionary(dict) => {
                // a direct dictionary gets a synthetic id in the page's namespace
                let key = (self.page, name.to_vec());
                let synthetic = (u32::MAX - self.page, self.fonts.anonymous.len() as u16);
                if !self.fonts.anonymous.contains_key(&key) {
                    let info = load_font(self.doc, dict);
                    self.fonts.anonymous.insert(key.clone(), info);
                }
                let position = self
                    .fonts
                    .anonymous
                    .keys()
                    .position(|k| *k == key)
                    .unwrap_or(0) as u16;
                let id = (synthetic.0, position);
                if !self.fonts.fonts.contains_key(&id) {
                    let info = load_font(self.doc, dict);
                    self.fonts.fonts.insert(id, info);
                }
                Some(id)
            }
            _ => None,
        }
    }

    fn show(
        &mut self,
        bytes: &[u8],
        ts: &TextState,
        tm: &mut Matrix,
        ctm: &Matrix,
        _resources: Option<&Dictionary>,
    ) {
        let font = ts.font.and_then(|id| self.fonts.fonts.get(&id));
        let (text, advance) = match font {
            Some(font) => decode(font, bytes, ts, self.doc),
            None => (
                String::from_utf8_lossy(bytes).into_owned(),
                bytes.len() as f32 * 0.5 * ts.size,
            ),
        };
        let bold = font.is_some_and(|f| f.bold);

        let trm = Matrix {
            a: ts.size * ts.horizontal_scale,
            b: 0.0,
            c: 0.0,
            d: ts.size,
            e: 0.0,
            f: ts.rise,
        }
        .then(tm)
        .then(ctm);
        let (x, y) = trm.apply(0.0, 0.0);
        let device_size = trm.y_scale();
        let device_width = advance * tm.then(ctm).x_scale();
        if !text.is_empty() && device_size > 0.0 {
            self.fragments.push(Fragment {
                page: self.page,
                x: x - self.media_box[0],
                baseline: self.media_box[3] - y,
                width: device_width,
                size: device_size,
                bold,
                text,
                confidence: Confidence::CERTAIN,
            });
        }
        *tm = Matrix::translation(advance, 0.0).then(tm);
    }
}

/// Decodes a string with its font and returns the text with the advance in text space units.
fn decode(font: &FontInfo, bytes: &[u8], ts: &TextState, _doc: &lopdf::Document) -> (String, f32) {
    let mut text = String::new();
    let mut advance = 0.0;
    let mut i = 0;
    let mut fallback = String::new();
    while i < bytes.len() {
        let len = font.code_lengths.iter().copied().find(|&len| {
            let end = i + len;
            end <= bytes.len()
                && font
                    .to_unicode
                    .as_ref()
                    .is_some_and(|map| map.contains_key(&code_at(bytes, i, len)))
        });
        let len = len
            .unwrap_or_else(|| *font.code_lengths.first().unwrap_or(&1))
            .min(bytes.len() - i);
        let code = code_at(bytes, i, len);
        let width = font
            .widths
            .get(&code)
            .copied()
            .unwrap_or(font.default_width)
            / 1000.0;
        let mut step = width * ts.size + ts.char_spacing;
        if len == 1 && code == 32 {
            step += ts.word_spacing;
        }
        advance += step * ts.horizontal_scale;
        match font.to_unicode.as_ref().and_then(|m| m.get(&code)) {
            Some(s) => text.push_str(s),
            None if font.simple_decoder => fallback.push(bytes[i] as char),
            None => text.push(
                char::from_u32(code)
                    .filter(|c| !c.is_control())
                    .unwrap_or('\u{fffd}'),
            ),
        }
        i += len;
    }
    if font.to_unicode.is_none() && font.simple_decoder {
        text = fallback;
    }
    (text, advance)
}

fn code_at(bytes: &[u8], start: usize, len: usize) -> u32 {
    bytes[start..start + len]
        .iter()
        .fold(0u32, |acc, b| (acc << 8) | *b as u32)
}

fn load_font(doc: &lopdf::Document, dict: &Dictionary) -> FontInfo {
    let deref = |o: &Object| doc.dereference(o).ok().map(|(_, o)| o.clone());
    let subtype = dict
        .get(b"Subtype")
        .ok()
        .and_then(|s| s.as_name().ok())
        .map(|s| s.to_vec())
        .unwrap_or_default();
    let base_font = dict
        .get(b"BaseFont")
        .ok()
        .and_then(|s| s.as_name().ok())
        .map(|s| String::from_utf8_lossy(s).to_string())
        .unwrap_or_default();
    let type0 = subtype == b"Type0";

    let descendant = if type0 {
        dict.get(b"DescendantFonts")
            .ok()
            .and_then(deref)
            .and_then(|d| d.as_array().ok().and_then(|a| a.first()).and_then(deref))
            .and_then(|d| d.as_dict().ok().cloned())
    } else {
        None
    };
    let descriptor_owner = descendant.as_ref().unwrap_or(dict);
    let descriptor = descriptor_owner
        .get(b"FontDescriptor")
        .ok()
        .and_then(deref)
        .and_then(|d| d.as_dict().ok().cloned());

    let flags = descriptor
        .as_ref()
        .and_then(|d| d.get(b"Flags").ok())
        .and_then(number)
        .unwrap_or(0.0) as u32;
    let stem_v = descriptor
        .as_ref()
        .and_then(|d| d.get(b"StemV").ok())
        .and_then(number)
        .unwrap_or(0.0);
    let name_lower = base_font.to_ascii_lowercase();
    let bold = flags & (1 << 18) != 0
        || stem_v >= 120.0
        || [
            "bold",
            "black",
            "heavy",
            "semibold",
            "demibold",
            "extrabold",
        ]
        .iter()
        .any(|w| name_lower.contains(w));

    let to_unicode = dict
        .get(b"ToUnicode")
        .ok()
        .and_then(|t| doc.dereference(t).ok())
        .and_then(|(_, t)| t.as_stream().ok())
        .and_then(|s| s.decompressed_content().ok())
        .map(|bytes| parse_to_unicode(&bytes));

    let mut code_lengths: Vec<usize> = to_unicode
        .as_ref()
        .map(|(_, lens)| lens.clone())
        .unwrap_or_default();
    if code_lengths.is_empty() {
        code_lengths = vec![if type0 { 2 } else { 1 }];
    }
    let mut to_unicode_map = to_unicode.map(|(map, _)| map);

    // a simple font with a named encoding and no ToUnicode is decoded through lopdf's tables
    let mut simple_decoder = false;
    if to_unicode_map.is_none() && !type0 {
        if let Ok(encoding) = dict.get_font_encoding(doc) {
            let mut map = HashMap::new();
            for code in 0u32..256 {
                if let Ok(s) = lopdf::Document::decode_text(&encoding, &[code as u8])
                    && !s.contains('\u{fffd}')
                    && !s.is_empty()
                {
                    map.insert(code, s);
                }
            }
            if !map.is_empty() {
                to_unicode_map = Some(map);
            }
        }
        simple_decoder = to_unicode_map.is_none();
    }

    let mut widths = HashMap::new();
    let mut default_width = 500.0;
    if let Some(desc) = &descendant {
        default_width = desc.get(b"DW").ok().and_then(number).unwrap_or(1000.0);
        if let Some(w) = desc
            .get(b"W")
            .ok()
            .and_then(deref)
            .and_then(|w| w.as_array().ok().cloned())
        {
            let items: Vec<Object> = w.iter().filter_map(deref).collect();
            let mut i = 0;
            while i < items.len() {
                let Some(first) = number(&items[i]) else {
                    break;
                };
                match items.get(i + 1) {
                    Some(Object::Array(list)) => {
                        for (k, item) in list.iter().enumerate() {
                            if let Some(width) = number(item) {
                                widths.insert(first as u32 + k as u32, width);
                            }
                        }
                        i += 2;
                    }
                    Some(last) => {
                        let (Some(last), Some(width)) =
                            (number(last), items.get(i + 2).and_then(number))
                        else {
                            break;
                        };
                        for code in first as u32..=(last as u32).min(first as u32 + 65_535) {
                            widths.insert(code, width);
                        }
                        i += 3;
                    }
                    None => break,
                }
            }
        }
    } else {
        let first_char = dict.get(b"FirstChar").ok().and_then(number).unwrap_or(0.0) as u32;
        if let Some(array) = dict
            .get(b"Widths")
            .ok()
            .and_then(deref)
            .and_then(|w| w.as_array().ok().cloned())
        {
            for (k, item) in array.iter().enumerate() {
                if let Some(width) = deref(item).as_ref().and_then(number) {
                    widths.insert(first_char + k as u32, width);
                }
            }
        }
        if let Some(missing) = descriptor
            .as_ref()
            .and_then(|d| d.get(b"MissingWidth").ok())
            .and_then(number)
        {
            default_width = missing;
        }
        if widths.is_empty() {
            // the standard 14 fonts carry no widths; a space is narrower than a letter
            widths.insert(32, 278.0);
        }
    }

    FontInfo {
        bold,
        code_lengths,
        to_unicode: to_unicode_map,
        simple_decoder,
        widths,
        default_width,
    }
}

/// Parses a ToUnicode CMap into a code to text map plus the byte lengths codes can have.
fn parse_to_unicode(bytes: &[u8]) -> (HashMap<u32, String>, Vec<usize>) {
    #[derive(Debug, Clone, PartialEq)]
    enum Token {
        Hex(Vec<u8>),
        Number(f32),
        Keyword(String),
        Open,
        Close,
    }
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        match c {
            b'%' => {
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
            }
            b'<' => {
                if bytes.get(i + 1) == Some(&b'<') {
                    i += 2;
                    continue;
                }
                let start = i + 1;
                while i < bytes.len() && bytes[i] != b'>' {
                    i += 1;
                }
                let hex: Vec<u8> = bytes[start..i.min(bytes.len())]
                    .iter()
                    .copied()
                    .filter(u8::is_ascii_hexdigit)
                    .collect();
                let value = hex
                    .chunks(2)
                    .filter_map(|pair| std::str::from_utf8(pair).ok())
                    .filter_map(|s| u8::from_str_radix(&format!("{s:0<2}"), 16).ok())
                    .collect();
                tokens.push(Token::Hex(value));
                i += 1;
            }
            b'>' => i += 1,
            b'[' => {
                tokens.push(Token::Open);
                i += 1;
            }
            b']' => {
                tokens.push(Token::Close);
                i += 1;
            }
            b'(' => {
                // a literal string, rare in a CMap; skipped
                let mut depth = 0;
                while i < bytes.len() {
                    match bytes[i] {
                        b'\\' => i += 1,
                        b'(' => depth += 1,
                        b')' => {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        _ => {}
                    }
                    i += 1;
                }
                i += 1;
            }
            c if c.is_ascii_whitespace() => i += 1,
            _ => {
                let start = i;
                while i < bytes.len()
                    && !bytes[i].is_ascii_whitespace()
                    && !b"<>[]()/%".contains(&bytes[i])
                {
                    i += 1;
                }
                if i == start {
                    i += 1;
                    continue;
                }
                let word = String::from_utf8_lossy(&bytes[start..i]).to_string();
                match word.parse::<f32>() {
                    Ok(n) => tokens.push(Token::Number(n)),
                    Err(_) => tokens.push(Token::Keyword(word)),
                }
            }
        }
    }

    let utf16 = |b: &[u8]| -> String {
        let units: Vec<u16> = b
            .chunks(2)
            .map(|p| u16::from_be_bytes([p[0], *p.get(1).unwrap_or(&0)]))
            .collect();
        String::from_utf16_lossy(&units)
    };
    let code_of = |b: &[u8]| b.iter().fold(0u32, |acc, x| (acc << 8) | *x as u32);

    let mut map = HashMap::new();
    let mut lengths: Vec<usize> = Vec::new();
    let mut note_length = |len: usize| {
        if len > 0 && len <= 4 && !lengths.contains(&len) {
            lengths.push(len);
        }
    };
    let mut i = 0;
    while i < tokens.len() {
        match &tokens[i] {
            Token::Keyword(k) if k == "begincodespacerange" => {
                i += 1;
                while let (Some(Token::Hex(lo)), Some(Token::Hex(_))) =
                    (tokens.get(i), tokens.get(i + 1))
                {
                    note_length(lo.len());
                    i += 2;
                }
            }
            Token::Keyword(k) if k == "beginbfchar" => {
                i += 1;
                while let (Some(Token::Hex(src)), Some(Token::Hex(dst))) =
                    (tokens.get(i), tokens.get(i + 1))
                {
                    note_length(src.len());
                    map.insert(code_of(src), utf16(dst));
                    i += 2;
                }
            }
            Token::Keyword(k) if k == "beginbfrange" => {
                i += 1;
                while let (Some(Token::Hex(lo)), Some(Token::Hex(hi))) =
                    (tokens.get(i), tokens.get(i + 1))
                {
                    note_length(lo.len());
                    let (lo_code, hi_code) = (code_of(lo), code_of(hi).min(code_of(lo) + 65_535));
                    match tokens.get(i + 2) {
                        Some(Token::Hex(dst)) => {
                            let base = utf16(dst);
                            for (k, code) in (lo_code..=hi_code).enumerate() {
                                // the last code unit counts up through the range
                                let mut units: Vec<u16> = base.encode_utf16().collect();
                                if let Some(last) = units.last_mut() {
                                    *last = last.wrapping_add(k as u16);
                                }
                                map.insert(code, String::from_utf16_lossy(&units));
                            }
                            i += 3;
                        }
                        Some(Token::Open) => {
                            i += 3;
                            let mut code = lo_code;
                            while let Some(Token::Hex(dst)) = tokens.get(i) {
                                map.insert(code, utf16(dst));
                                code += 1;
                                i += 1;
                            }
                            i += 1;
                        }
                        _ => break,
                    }
                }
            }
            _ => i += 1,
        }
    }
    lengths.sort_unstable();
    (map, lengths)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{BlockKind, Document, join_runs};
    use std::path::PathBuf;

    fn fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures")
            .join(name)
    }

    fn outline(doc: &Document) -> Vec<String> {
        doc.blocks
            .iter()
            .filter_map(|b| match &b.kind {
                BlockKind::Heading { level, runs } => Some(format!("h{level}:{}", join_runs(runs))),
                BlockKind::Paragraph { runs, .. } => Some(format!("p:{}", join_runs(runs))),
                BlockKind::List { ordered, items } => Some(format!(
                    "{}:{}",
                    if *ordered { "ol" } else { "ul" },
                    items
                        .iter()
                        .map(|i| join_runs(&i.runs))
                        .collect::<Vec<_>>()
                        .join("|")
                )),
                BlockKind::PageBreak => Some("---".into()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn to_unicode_cmaps_parse_bfchar_and_bfrange() {
        let cmap = b"1 begincodespacerange <00> <FF> endcodespacerange\n\
            2 beginbfchar <01> <0041> <02> <03A9> endbfchar\n\
            2 beginbfrange <10> <12> <0061> <20> <21> [<0078> <00790079>] endbfrange";
        let (map, lengths) = parse_to_unicode(cmap);
        assert_eq!(lengths, [1]);
        assert_eq!(map[&1], "A");
        assert_eq!(map[&2], "\u{3a9}");
        assert_eq!(map[&0x10], "a");
        assert_eq!(map[&0x12], "c");
        assert_eq!(map[&0x20], "x");
        assert_eq!(map[&0x21], "yy");
    }

    #[test]
    fn slides_keep_their_headings_lists_and_page_order() {
        let imported = import_pdf(&fixture("slides-signals.pdf")).unwrap();
        assert!(imported.pages_needing_ocr.is_empty());
        let doc = &imported.document;
        assert_eq!(doc.pages.len(), 4);
        assert_eq!(doc.title, "Signals and Systems week 3");
        assert_eq!(
            outline(doc),
            [
                "h1:Signals and Systems",
                "p:Week 3: Fourier series",
                "p:Lecture slides for EE2001",
                "---",
                "h2:Why Fourier series?",
                "ul:Any periodic signal is a sum of sinusoids|Each harmonic has its own amplitude and phase|A filter acts on each harmonic separately",
                "---",
                "h2:Worked example",
                "p:A square wave at 50 Hz has only odd harmonics.",
                "ul:Third harmonic: 150 Hz|Fifth harmonic: 250 Hz|Seventh harmonic: 350 Hz",
                "---",
                "h2:Summary",
                "p:Periodic signals can be analysed one harmonic at a time.",
                "p:Next week: the Fourier transform for signals that do not repeat.",
            ]
        );
        // every block links to the page it was drawn on
        for block in &doc.blocks {
            for run in block.runs() {
                assert_eq!(run.origin.unwrap().page, block.origin.page);
            }
        }
        assert_eq!(
            doc.blocks_on_page(3)
                .filter(|b| !matches!(b.kind, BlockKind::PageBreak))
                .count(),
            3
        );
    }

    #[test]
    fn two_column_textbook_reads_down_each_column() {
        let imported = import_pdf(&fixture("textbook-two-column.pdf")).unwrap();
        let text = outline(&imported.document);
        let position = |needle: &str| {
            text.iter()
                .position(|t| t.contains(needle))
                .unwrap_or_else(|| panic!("{needle} missing in {text:#?}"))
        };
        assert_eq!(position("h1:Chapter 4"), 0);
        assert!(position("4.1 The ideal op-amp") < position("4.2 The inverting amplifier"));
        assert!(position("4.2 The inverting amplifier") < position("Because the inverting input"));
        assert!(
            position("Because the inverting input") < position("4.3 The non-inverting amplifier")
        );
        assert!(
            !text.iter().any(|t| t == "p:47"),
            "the page number is furniture"
        );
        assert_eq!(text.iter().filter(|t| t.starts_with("h2:")).count(), 3);
    }

    #[test]
    fn numbers_and_symbols_survive_the_text_layer() {
        let imported = import_pdf(&fixture("notes-numbers.pdf")).unwrap();
        let text = imported.document.plain_text();
        for needed in [
            "4.7 k\u{3a9}",
            "12 \u{d7} 10 / 14.7 = 8.16 V",
            "\u{2212}0.35 V",
            "45\u{b0} (\u{3c0}/4 rad)",
            "\u{b1} 5%",
        ] {
            assert!(text.contains(needed), "{needed} missing in {text}");
        }
    }

    #[test]
    fn a_scanned_pdf_is_handed_to_ocr() {
        let imported = import_pdf(&fixture("handout-scan.pdf")).unwrap();
        assert_eq!(imported.pages_needing_ocr, [1]);
        assert_eq!(imported.document.pages.len(), 1);
        assert!(imported.document.blocks.is_empty());
    }

    #[test]
    fn rejects_bytes_that_are_not_a_pdf() {
        assert!(matches!(
            import_pdf_bytes(b"hello", "x.pdf"),
            Err(ImportError::Pdf(_))
        ));
    }
}
