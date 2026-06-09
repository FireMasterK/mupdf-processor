use base64::Engine;
use mupdf::{Colorspace, Document, Matrix, TextPageFlags};
use tokio::sync::mpsc;

use crate::types::{
    PageResult, ProcessingFailure, ProcessingResponse, ResponseOptions, TextBlockResult,
    TextCharResult,
};
use crate::upload::UploadedPdf;
use crate::ws_protocol::{RenderScale, ServerEvent};

pub(crate) fn text_page_flags() -> TextPageFlags {
    TextPageFlags::PRESERVE_WHITESPACE | TextPageFlags::ACCURATE_BBOXES
}

pub(crate) fn extract_page_result(
    doc: &Document,
    page_index: u32,
    render_scale: RenderScale,
    options: ResponseOptions,
) -> Result<PageResult, ProcessingFailure> {
    let page_data = extract_page_data(doc, page_index, render_scale, options)?;
    Ok(PageResult {
        page_index,
        text: page_data.text.unwrap_or_default(),
        rendered_png_base64: page_data
            .rendered_png_bytes
            .map(|bytes| base64::engine::general_purpose::STANDARD.encode(bytes))
            .unwrap_or_default(),
        blocks: page_data.blocks.unwrap_or_default(),
    })
}

struct ExtractedPageData {
    text: Option<String>,
    rendered_png_bytes: Option<Vec<u8>>,
    blocks: Option<Vec<TextBlockResult>>,
}

fn extract_page_data(
    doc: &Document,
    page_index: u32,
    render_scale: RenderScale,
    options: ResponseOptions,
) -> Result<ExtractedPageData, ProcessingFailure> {
    let page = doc
        .load_page(page_index as i32)
        .map_err(ProcessingFailure::from)?;

    let want_text = options.want_text();
    let want_bbox = options.want_bbox();
    let want_render = options.want_render_image();

    let (text, blocks) = if want_text || want_bbox {
        let text_page = page
            .to_text_page(text_page_flags())
            .map_err(ProcessingFailure::from)?;

        let text = if want_text {
            Some(text_page.to_text().map_err(ProcessingFailure::from)?)
        } else {
            None
        };

        let blocks = if want_bbox {
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
            Some(blocks)
        } else {
            None
        };

        (text, blocks)
    } else {
        (None, None)
    };

    let rendered_png_bytes = if want_render {
        Some(render_page_to_png(&page, render_scale)?)
    } else {
        None
    };

    Ok(ExtractedPageData {
        text,
        rendered_png_bytes,
        blocks,
    })
}

fn render_page_to_png(
    page: &mupdf::Page,
    render_scale: RenderScale,
) -> Result<Vec<u8>, ProcessingFailure> {
    let colorspace = Colorspace::device_rgb();
    let scale = render_scale.value();
    let transform = Matrix::new_scale(scale, scale);
    let pixmap = page
        .to_pixmap(&transform, &colorspace, false, false)
        .map_err(ProcessingFailure::from)?;
    let mut png_bytes = Vec::new();
    pixmap
        .write_to(&mut png_bytes, mupdf::ImageFormat::PNG)
        .map_err(ProcessingFailure::from)?;
    Ok(png_bytes)
}

pub(crate) fn process_pdf_all(
    request_id: String,
    source: UploadedPdf,
    render_scale: RenderScale,
    options: ResponseOptions,
) -> Result<ProcessingResponse, ProcessingFailure> {
    let file_size_bytes = source.file_size()?;
    let bytes = source.bytes()?;
    let doc = Document::from_bytes(&bytes, "application/pdf").map_err(ProcessingFailure::from)?;
    let page_count = doc.page_count().map_err(ProcessingFailure::from)? as u32;

    let pages = if options.want_text() || options.want_bbox() || options.want_render_image() {
        let mut pages = Vec::with_capacity(page_count as usize);
        for page_index in 0..page_count {
            pages.push(extract_page_result(&doc, page_index, render_scale, options)?);
        }
        pages
    } else {
        Vec::new()
    };

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
    render_scale: RenderScale,
    options: ResponseOptions,
    event_tx: mpsc::UnboundedSender<ServerEvent>,
) -> Result<(), ProcessingFailure> {
    let file_size_bytes = source.file_size()?;
    let bytes = source.bytes()?;
    let doc = Document::from_bytes(&bytes, "application/pdf").map_err(ProcessingFailure::from)?;
    let page_count = doc.page_count().map_err(ProcessingFailure::from)? as u32;

    if options.want_text() || options.want_bbox() || options.want_render_image() {
        for page_index in 0..page_count {
            let page = extract_page_data(&doc, page_index, render_scale, options)?;
            let _ = event_tx.send(ServerEvent::Page {
                request_id: request_id.clone(),
                page_index,
                total_pages: page_count,
                text: page.text.unwrap_or_default(),
                rendered_png_bytes: page.rendered_png_bytes.unwrap_or_default(),
                blocks: page.blocks.unwrap_or_default(),
            });
        }
    }

    let _ = event_tx.send(ServerEvent::Complete {
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
        let response = process_pdf_all(
            "req-test".to_string(),
            UploadedPdf::Memory(bytes),
            RenderScale::default(),
            ResponseOptions::ALL,
        )
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
        let response = process_pdf_all(
            "req-test".to_string(),
            UploadedPdf::Memory(bytes),
            RenderScale::default(),
            ResponseOptions::ALL,
        )
        .expect("process fixture");

        assert_eq!(response.page_count, 2);
        assert_eq!(response.pages.len(), 2);
        assert!(response.pages[0].text.contains("First Page"));
        assert!(response.pages[1].text.contains("Second Page"));
    }
}
