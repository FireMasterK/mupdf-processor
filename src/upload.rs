use std::fs;
use std::io::{self, Write};
use std::path::Path;

use actix_multipart::Multipart;
use futures_util::StreamExt;
use tempfile::{NamedTempFile, TempPath};

use crate::config::AppConfig;
use crate::types::ProcessingFailure;

pub const MAX_IN_MEMORY_UPLOAD_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_ACCEPTED_UPLOAD_BYTES: usize = 512 * 1024 * 1024;

#[derive(Debug)]
pub struct CollectedUpload {
    pub file_name: Option<String>,
    pub source: UploadedPdf,
}

#[derive(Debug)]
pub enum UploadedPdf {
    Memory(Vec<u8>),
    TempFile(TempPath),
}

impl UploadedPdf {
    pub fn bytes(&self) -> Result<Vec<u8>, ProcessingFailure> {
        match self {
            Self::Memory(bytes) => Ok(bytes.clone()),
            Self::TempFile(path) => {
                fs::read(path.as_ref() as &Path).map_err(|error| ProcessingFailure {
                    request_id: None,
                    message: format!("failed to read temporary upload: {error}"),
                })
            }
        }
    }

    pub fn file_size(&self) -> Result<u64, ProcessingFailure> {
        match self {
            Self::Memory(bytes) => Ok(bytes.len() as u64),
            Self::TempFile(path) => fs::metadata(path.as_ref() as &Path)
                .map(|metadata| metadata.len())
                .map_err(|error| ProcessingFailure {
                    request_id: None,
                    message: format!("failed to stat temporary upload: {error}"),
                }),
        }
    }

    pub fn into_temp_file(self, file_name: Option<&str>) -> Result<Self, ProcessingFailure> {
        match self {
            Self::Memory(bytes) => {
                let mut temp = create_temp_pdf(file_name)
                    .map_err(|error| processing_error(error.to_string()))?;
                temp.write_all(&bytes)
                    .map_err(|error| processing_error(error.to_string()))?;
                Ok(Self::TempFile(temp.into_temp_path()))
            }
            Self::TempFile(path) => Ok(Self::TempFile(path)),
        }
    }
}

pub async fn collect_pdf_upload(
    multipart: &mut Multipart,
    config: &AppConfig,
) -> Result<CollectedUpload, ProcessingFailure> {
    let mut maybe_pdf = None;

    while let Some(item) = multipart.next().await {
        let mut field = item.map_err(|error| ProcessingFailure {
            request_id: None,
            message: format!("multipart read error: {error}"),
        })?;

        let content_disposition = field.content_disposition().cloned();
        let field_name = content_disposition
            .as_ref()
            .and_then(|cd| cd.get_name())
            .unwrap_or_default()
            .to_owned();

        if field_name != "file" && field_name != "pdf" {
            while let Some(chunk) = field.next().await {
                chunk.map_err(|error| ProcessingFailure {
                    request_id: None,
                    message: format!("multipart read error: {error}"),
                })?;
            }
            continue;
        }

        let file_name = content_disposition
            .as_ref()
            .and_then(|cd| cd.get_filename())
            .map(str::to_owned);

        let mut total = 0usize;
        let mut memory = Vec::new();
        let mut temp = None::<NamedTempFile>;

        while let Some(chunk) = field.next().await {
            let chunk = chunk.map_err(|error| ProcessingFailure {
                request_id: None,
                message: format!("multipart read error: {error}"),
            })?;
            total = total.saturating_add(chunk.len());

            if total > config.max_accepted_upload_bytes {
                return Err(ProcessingFailure {
                    request_id: None,
                    message: format!(
                        "uploaded PDF is too large; limit is {} bytes",
                        config.max_accepted_upload_bytes
                    ),
                });
            }

            if temp.is_none() && total <= config.max_in_memory_upload_bytes {
                memory.extend_from_slice(&chunk);
            } else {
                if temp.is_none() {
                    temp = Some(
                        create_temp_pdf(file_name.as_deref())
                            .map_err(|error| processing_error(error.to_string()))?,
                    );
                    if let Some(temp_file) = temp.as_mut() {
                        temp_file
                            .write_all(&memory)
                            .map_err(|error| processing_error(error.to_string()))?;
                    }
                    memory.clear();
                }

                if let Some(temp_file) = temp.as_mut() {
                    temp_file
                        .write_all(&chunk)
                        .map_err(|error| processing_error(error.to_string()))?;
                }
            }
        }

        maybe_pdf = Some(CollectedUpload {
            file_name,
            source: match temp {
                Some(temp_file) => UploadedPdf::TempFile(temp_file.into_temp_path()),
                None => UploadedPdf::Memory(memory),
            },
        });
        break;
    }

    maybe_pdf.ok_or_else(|| processing_error("missing multipart file field named `file` or `pdf`"))
}

pub fn spill_bytes_if_needed(
    bytes: Vec<u8>,
    file_name: Option<String>,
    config: &AppConfig,
) -> Result<CollectedUpload, ProcessingFailure> {
    if bytes.len() > config.max_accepted_upload_bytes {
        return Err(processing_error(format!(
            "uploaded PDF is too large; limit is {} bytes",
            config.max_accepted_upload_bytes
        )));
    }

    let source = if bytes.len() <= config.max_in_memory_upload_bytes {
        UploadedPdf::Memory(bytes)
    } else {
        UploadedPdf::Memory(bytes).into_temp_file(file_name.as_deref())?
    };

    Ok(CollectedUpload { file_name, source })
}

pub fn create_temp_pdf(file_name: Option<&str>) -> io::Result<NamedTempFile> {
    let suffix = file_name
        .and_then(|name| Path::new(name).extension())
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .map(|value| format!(".{value}"))
        .unwrap_or_else(|| ".pdf".to_string());

    tempfile::Builder::new()
        .prefix("mupdf-processor-")
        .suffix(&suffix)
        .tempfile()
}

pub fn processing_error(message: impl Into<String>) -> ProcessingFailure {
    ProcessingFailure {
        request_id: None,
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::SocketAddr;

    fn test_config() -> AppConfig {
        AppConfig {
            bind_addr: "127.0.0.1:0".parse::<SocketAddr>().expect("addr"),
            worker_count: 1,
            max_in_memory_upload_bytes: MAX_IN_MEMORY_UPLOAD_BYTES,
            max_accepted_upload_bytes: MAX_ACCEPTED_UPLOAD_BYTES,
        }
    }

    #[test]
    fn spill_small_upload_stays_in_memory() {
        let bytes = vec![1, 2, 3];
        let upload = spill_bytes_if_needed(
            bytes.clone(),
            Some("a.pdf".to_string()),
            &test_config(),
        )
        .expect("spill");

        match upload.source {
            UploadedPdf::Memory(stored) => assert_eq!(stored, bytes),
            UploadedPdf::TempFile(_) => panic!("expected in-memory upload"),
        }
    }

    #[test]
    fn spill_large_upload_moves_to_temp_file() {
        let bytes = vec![7_u8; MAX_IN_MEMORY_UPLOAD_BYTES + 1];
        let upload = spill_bytes_if_needed(
            bytes.clone(),
            Some("big.pdf".to_string()),
            &test_config(),
        )
        .expect("spill");

        match upload.source {
            UploadedPdf::TempFile(path) => {
                let on_disk = fs::read(path.as_ref() as &Path).expect("read temp");
                assert_eq!(on_disk.len(), bytes.len());
                assert_eq!(on_disk[0], 7);
            }
            UploadedPdf::Memory(_) => panic!("expected temp-file upload"),
        }
    }

    #[test]
    fn temp_file_keeps_original_extension() {
        let temp = create_temp_pdf(Some("report.custom.pdf")).expect("temp");
        let suffix = temp
            .path()
            .extension()
            .and_then(|value| value.to_str())
            .expect("extension");
        assert_eq!(suffix, "pdf");
    }

    #[test]
    fn oversized_upload_is_rejected() {
        let bytes = vec![0_u8; MAX_ACCEPTED_UPLOAD_BYTES + 1];
        let error = spill_bytes_if_needed(bytes, None, &test_config()).expect_err("should fail");
        assert!(error.message.contains("too large"));
    }
}
