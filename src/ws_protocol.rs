use fory::ForyStruct;

use crate::types::{ProcessingFailure, TextBlockResult};

pub const DEFAULT_RENDER_SCALE: f32 = 1.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RenderScale(f32);

impl RenderScale {
    pub fn try_new(value: f32) -> Result<Self, ProcessingFailure> {
        if value.is_finite() && value > 0.0 {
            Ok(Self(value))
        } else {
            Err(ProcessingFailure {
                request_id: None,
                message: format!("render_scale must be a finite positive number, got {value}"),
            })
        }
    }

    pub fn resolve(value: Option<f32>) -> Result<Self, ProcessingFailure> {
        match value {
            Some(value) => Self::try_new(value),
            None => Ok(Self::default()),
        }
    }

    pub fn value(self) -> f32 {
        self.0
    }
}

impl Default for RenderScale {
    fn default() -> Self {
        Self(DEFAULT_RENDER_SCALE)
    }
}

#[derive(Debug, Clone)]
pub struct ClientUpload {
    pub file_name: Option<String>,
    pub pdf_bytes: Vec<u8>,
    pub render_scale: RenderScale,
}

#[derive(Debug, Clone)]
pub enum ClientCommand {
    Upload(ClientUpload),
}

#[derive(Debug, Clone)]
pub enum ServerEvent {
    Accepted {
        request_id: String,
    },
    Page {
        request_id: String,
        page_index: u32,
        total_pages: u32,
        text: String,
        rendered_png_bytes: Vec<u8>,
        blocks: Vec<TextBlockResult>,
    },
    Complete {
        request_id: String,
        page_count: u32,
        file_size_bytes: u64,
    },
    Error {
        request_id: Option<String>,
        message: String,
    },
}

#[derive(Debug, Clone, Copy)]
pub enum MessageKind {
    UploadCommand = 1,
    AcceptedEvent = 2,
    PageEvent = 3,
    CompleteEvent = 4,
    ErrorEvent = 5,
}

impl MessageKind {
    pub fn from_u8(value: u8) -> Option<Self> {
        match value {
            1 => Some(Self::UploadCommand),
            2 => Some(Self::AcceptedEvent),
            3 => Some(Self::PageEvent),
            4 => Some(Self::CompleteEvent),
            5 => Some(Self::ErrorEvent),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Default, ForyStruct)]
pub struct UploadCommandMeta {
    pub file_name: Option<String>,
    pub render_scale: Option<f32>,
}

#[derive(Debug, Clone, Default, ForyStruct)]
pub struct AcceptedEventMeta {
    pub request_id: String,
}

#[derive(Debug, Clone, Default, ForyStruct)]
pub struct PageEventMeta {
    pub request_id: String,
    pub page_index: u32,
    pub total_pages: u32,
    pub text: String,
    pub blocks: Vec<TextBlockResult>,
}

#[derive(Debug, Clone, Default, ForyStruct)]
pub struct CompleteEventMeta {
    pub request_id: String,
    pub page_count: u32,
    pub file_size_bytes: u64,
}

#[derive(Debug, Clone, Default, ForyStruct)]
pub struct ErrorEventMeta {
    pub request_id: Option<String>,
    pub message: String,
}
