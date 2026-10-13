//! The functions the page calls. Each one only converts errors, so the behaviour lives in
//! `lib.rs` where it is tested natively.

use wasm_bindgen::prelude::*;

fn js(result: crate::Result<String>) -> Result<String, JsError> {
    result.map_err(|e| JsError::new(&e))
}

#[wasm_bindgen(js_name = defaultProfile)]
pub fn default_profile() -> String {
    crate::default_profile()
}

#[wasm_bindgen]
pub fn presets() -> String {
    crate::presets()
}

#[wasm_bindgen(js_name = checkProfile)]
pub fn check_profile(json: &str) -> Result<String, JsError> {
    js(crate::check_profile(json))
}

#[wasm_bindgen(js_name = readingCss)]
pub fn reading_css(profile: &str, scope: &str) -> Result<String, JsError> {
    js(crate::reading_css(profile, scope))
}

#[wasm_bindgen(js_name = documentFromText)]
pub fn document_from_text(text: &str, file_name: &str) -> Result<String, JsError> {
    js(crate::document_from_text(text, file_name))
}

#[wasm_bindgen(js_name = documentFromHtml)]
pub fn document_from_html(
    html: &str,
    file_name: &str,
    format: &str,
    fragment: bool,
) -> Result<String, JsError> {
    js(crate::document_from_html(html, file_name, format, fragment))
}

#[wasm_bindgen(js_name = documentFromPages)]
pub fn document_from_pages(json: &str) -> Result<String, JsError> {
    js(crate::document_from_pages(json))
}

#[wasm_bindgen]
pub fn summary(document: &str) -> Result<String, JsError> {
    js(crate::summary(document))
}

#[wasm_bindgen(js_name = readingHtml)]
pub fn reading_html(document: &str, profile: &str) -> Result<String, JsError> {
    js(crate::reading_html(document, profile))
}

#[wasm_bindgen(js_name = exportHtml)]
pub fn export_html(document: &str, profile: &str, language: &str) -> Result<String, JsError> {
    js(crate::export_html(document, profile, language))
}

#[wasm_bindgen(js_name = exportEpub)]
pub fn export_epub(
    document: &str,
    profile: &str,
    language: &str,
    modified: f64,
) -> Result<Vec<u8>, JsError> {
    // a clock before 1970 or a value that is not a time becomes 0 rather than wrapping
    let seconds = if modified.is_finite() && modified > 0.0 {
        modified as u64
    } else {
        0
    };
    crate::export_epub(document, profile, language, seconds).map_err(|e| JsError::new(&e))
}
