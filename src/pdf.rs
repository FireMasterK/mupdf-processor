use std::fs;

use base64::Engine;
use mupdf::{Colorspace, Document, Matrix, TextPageFlags};
use tokio::sync::mpsc;

use crate::types::{
    PageResult, ProcessingFailure, ProcessingResponse, TextBlockResult, TextCharResult,
    WsServerEvent,
};
use crate::upload::UploadedPdf;

pub(crate) fn text_page_flags() -> TextPageFlags {
    TextPageFlags::PRESERVE_WHITESPACE | TextPageFlags::ACCURATE_BBOXES
}

pub(crate) fn extract_page_result(
    doc: &Document,
    page_index: u32,
) -> Result<PageResult, ProcessingFailure> {
    let page_data = extract_page_data(doc, page_index)?;
    Ok(PageResult {
        page_index,
        text: page_data.text,
        rendered_png_base64: base64::engine::general_purpose::STANDARD.encode(page_data.rendered_png_bytes),
        blocks: page_data.blocks,
    })
}

struct ExtractedPageData {
    text: String,
    rendered_png_bytes: Vec<u8>,
    blocks: Vec<TextBlockResult>,
}

fn extract_page_data(doc: &Document, page_index: u32) -> Result<ExtractedPageData, ProcessingFailure> {
    let page = doc
        .load_page(page_index as i32)
        .map_err(ProcessingFailure::from)?;
    let text_page = page
        .to_text_page(text_page_flags())
        .map_err(ProcessingFailure::from)?;
    let text = text_page.to_text().map_err(ProcessingFailure::from)?;

    let mut blocks = Vec::new();
    for block in text_page.blocks() {
        let mut block_text = String::new();
        let mut chars = Vec::new();

        for line in block.lines() {
            for ch in line.chars() {
                if let Some(value) = ch.char() {
                    block_text.push(value);
                    let quad = ch.quad();
                    let bbox = mupdf::Rect::from(quad.clone());
                    chars.push(TextCharResult {
                        value: value.to_string(),
                        quad: quad.into(),
                        bbox: bbox.into(),
                    });
                }
            }
            if !block_text.is_empty() {
                block_text.push('\n');
            }
        }

        if !block_text.trim().is_empty() || !chars.is_empty() {
            blocks.push(TextBlockResult {
                text: block_text.trim_end().to_string(),
                bbox: block.bounds().into(),
                chars,
            });
        }
    }

    let colorspace = Colorspace::device_rgb();
    let transform = Matrix::new_scale(1.0, 1.0);
    let pixmap = page
        .to_pixmap(&transform, &colorspace, false, false)
        .map_err(ProcessingFailure::from)?;
    let temp_png = tempfile::Builder::new()
        .prefix("mupdf-render-")
        .suffix(".png")
        .tempfile()
        .map_err(|error| ProcessingFailure {
            request_id: None,
            message: error.to_string(),
        })?;
    let temp_path = temp_png.path().to_string_lossy().into_owned();

    pixmap
        .save_as(&temp_path, mupdf::ImageFormat::PNG)
        .map_err(ProcessingFailure::from)?;
    let png_bytes = fs::read(temp_png.path()).map_err(|error| ProcessingFailure {
        request_id: None,
        message: error.to_string(),
    })?;

    Ok(ExtractedPageData {
        text,
        rendered_png_bytes: png_bytes,
        blocks,
    })
}

pub(crate) fn process_pdf_all(
    request_id: String,
    source: UploadedPdf,
) -> Result<ProcessingResponse, ProcessingFailure> {
    let file_size_bytes = source.file_size()?;
    let bytes = source.bytes()?;
    let doc = Document::from_bytes(&bytes, "application/pdf").map_err(ProcessingFailure::from)?;
    let page_count = doc.page_count().map_err(ProcessingFailure::from)? as u32;
    let mut pages = Vec::with_capacity(page_count as usize);

    for page_index in 0..page_count {
        pages.push(extract_page_result(&doc, page_index)?);
    }

    Ok(ProcessingResponse {
        request_id,
        file_size_bytes,
        page_count,
        pages,
    })
}

pub(crate) fn process_pdf_streaming(
    request_id: String,
    source: UploadedPdf,
    event_tx: mpsc::UnboundedSender<WsServerEvent>,
) -> Result<(), ProcessingFailure> {
    let file_size_bytes = source.file_size()?;
    let bytes = source.bytes()?;
    let doc = Document::from_bytes(&bytes, "application/pdf").map_err(ProcessingFailure::from)?;
    let page_count = doc.page_count().map_err(ProcessingFailure::from)? as u32;

    for page_index in 0..page_count {
        let page = extract_page_data(&doc, page_index)?;
        let _ = event_tx.send(WsServerEvent::Page {
            request_id: request_id.clone(),
            page_index,
            total_pages: page_count,
            text: page.text,
            rendered_png_bytes: page.rendered_png_bytes,
            blocks: page.blocks,
        });
    }

    let _ = event_tx.send(WsServerEvent::Complete {
        request_id,
        page_count,
        file_size_bytes,
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_text_and_png_from_single_page_fixture() {
        let bytes = std::fs::read("testdata/hello.pdf").expect("fixture");
        let response = process_pdf_all("req-test".to_string(), UploadedPdf::Memory(bytes))
            .expect("process fixture");

        assert_eq!(response.page_count, 1);
        assert_eq!(response.pages.len(), 1);
        assert!(response.pages[0].text.contains("Hello PDF"));
        assert!(!response.pages[0].rendered_png_base64.is_empty());
        assert!(!response.pages[0].blocks.is_empty());
    }

    #[test]
    fn extracts_all_pages_from_multi_page_fixture() {
        let bytes = std::fs::read("testdata/two-pages.pdf").expect("fixture");
        let response = process_pdf_all("req-test".to_string(), UploadedPdf::Memory(bytes))
            .expect("process fixture");

        assert_eq!(response.page_count, 2);
        assert_eq!(response.pages.len(), 2);
        assert!(response.pages[0].text.contains("First Page"));
        assert!(response.pages[1].text.contains("Second Page"));
    }
}

