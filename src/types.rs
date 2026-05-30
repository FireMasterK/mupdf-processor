use fory::ForyStruct;
use serde::Serialize;

#[derive(Debug, Clone, Default, Serialize, ForyStruct)]
pub struct ProcessingResponse {
    pub request_id: String,
    pub file_size_bytes: u64,
    pub page_count: u32,
    pub pages: Vec<PageResult>,
}

#[derive(Debug, Clone, Default, Serialize, ForyStruct)]
pub struct PageResult {
    pub page_index: u32,
    pub text: String,
    pub rendered_png_base64: String,
    pub blocks: Vec<TextBlockResult>,
}

#[derive(Debug, Clone, Default, Serialize, ForyStruct)]
pub struct TextBlockResult {
    pub text: String,
    pub bbox: RectDto,
    pub chars: Vec<TextCharResult>,
}

#[derive(Debug, Clone, Default, Serialize, ForyStruct)]
pub struct TextCharResult {
    pub value: String,
    pub quad: QuadDto,
    pub bbox: RectDto,
}

#[derive(Debug, Clone, Copy, Default, Serialize, ForyStruct)]
pub struct RectDto {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
}

#[derive(Debug, Clone, Copy, Default, Serialize, ForyStruct)]
pub struct PointDto {
    pub x: f32,
    pub y: f32,
}

#[derive(Debug, Clone, Copy, Default, Serialize, ForyStruct)]
pub struct QuadDto {
    pub ul: PointDto,
    pub ur: PointDto,
    pub ll: PointDto,
    pub lr: PointDto,
}

#[derive(Debug)]
pub struct ProcessingFailure {
    pub request_id: Option<String>,
    pub message: String,
}

impl std::fmt::Display for ProcessingFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for ProcessingFailure {}

impl From<mupdf::Error> for ProcessingFailure {
    fn from(error: mupdf::Error) -> Self {
        Self {
            request_id: None,
            message: format!("mupdf error: {error}"),
        }
    }
}

impl From<mupdf::Rect> for RectDto {
    fn from(value: mupdf::Rect) -> Self {
        Self {
            x0: value.x0,
            y0: value.y0,
            x1: value.x1,
            y1: value.y1,
        }
    }
}

impl From<mupdf::Point> for PointDto {
    fn from(value: mupdf::Point) -> Self {
        Self {
            x: value.x,
            y: value.y,
        }
    }
}

impl From<mupdf::Quad> for QuadDto {
    fn from(value: mupdf::Quad) -> Self {
        Self {
            ul: value.ul.into(),
            ur: value.ur.into(),
            ll: value.ll.into(),
            lr: value.lr.into(),
        }
    }
}
