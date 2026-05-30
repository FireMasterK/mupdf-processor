use std::sync::Arc;

use actix_ws::Session;
use fory::Fory;

use crate::types::{
    PageResult, PointDto, ProcessingFailure, ProcessingResponse, QuadDto, RectDto, TextBlockResult,
    TextCharResult,
};
use crate::ws_protocol::{
    AcceptedEventMeta, ClientCommand, ClientUpload, CompleteEventMeta, ErrorEventMeta, MessageKind,
    PageEventMeta, RenderScale, ServerEvent, UploadCommandMeta,
};

const TYPE_ID_RECT_DTO: u32 = 100;
const TYPE_ID_POINT_DTO: u32 = 101;
const TYPE_ID_QUAD_DTO: u32 = 102;
const TYPE_ID_TEXT_CHAR_RESULT: u32 = 103;
const TYPE_ID_TEXT_BLOCK_RESULT: u32 = 104;
const TYPE_ID_PAGE_RESULT: u32 = 105;
const TYPE_ID_PROCESSING_RESPONSE: u32 = 106;
const TYPE_ID_WS_UPLOAD_COMMAND: u32 = 107;
const TYPE_ID_WS_ACCEPTED_EVENT: u32 = 108;
const TYPE_ID_WS_PAGE_EVENT: u32 = 109;
const TYPE_ID_WS_COMPLETE_EVENT: u32 = 110;
const TYPE_ID_WS_ERROR_EVENT: u32 = 111;

pub struct WsCodec {
    fory: Arc<Fory>,
}

impl WsCodec {
    pub fn new() -> Result<Self, fory::Error> {
        let mut fory = Fory::builder().xlang(true).compatible(false).build();
        fory.register::<RectDto>(TYPE_ID_RECT_DTO)?;
        fory.register::<PointDto>(TYPE_ID_POINT_DTO)?;
        fory.register::<QuadDto>(TYPE_ID_QUAD_DTO)?;
        fory.register::<TextCharResult>(TYPE_ID_TEXT_CHAR_RESULT)?;
        fory.register::<TextBlockResult>(TYPE_ID_TEXT_BLOCK_RESULT)?;
        fory.register::<PageResult>(TYPE_ID_PAGE_RESULT)?;
        fory.register::<ProcessingResponse>(TYPE_ID_PROCESSING_RESPONSE)?;
        fory.register::<UploadCommandMeta>(TYPE_ID_WS_UPLOAD_COMMAND)?;
        fory.register::<AcceptedEventMeta>(TYPE_ID_WS_ACCEPTED_EVENT)?;
        fory.register::<PageEventMeta>(TYPE_ID_WS_PAGE_EVENT)?;
        fory.register::<CompleteEventMeta>(TYPE_ID_WS_COMPLETE_EVENT)?;
        fory.register::<ErrorEventMeta>(TYPE_ID_WS_ERROR_EVENT)?;
        Ok(Self {
            fory: Arc::new(fory),
        })
    }

    pub fn encode_event(&self, event: &ServerEvent) -> Result<Vec<u8>, ProcessingFailure> {
        let (kind, payload, binary_tail) = match event {
            ServerEvent::Accepted { request_id } => (
                MessageKind::AcceptedEvent,
                self.fory
                    .serialize(&AcceptedEventMeta {
                        request_id: request_id.clone(),
                    })
                    .map_err(|error| ProcessingFailure {
                        request_id: None,
                        message: format!("failed to serialize accepted event: {error}"),
                    })?,
                Vec::new(),
            ),
            ServerEvent::Page {
                request_id,
                page_index,
                total_pages,
                text,
                rendered_png_bytes,
                blocks,
            } => (
                MessageKind::PageEvent,
                self.fory
                    .serialize(&PageEventMeta {
                        request_id: request_id.clone(),
                        page_index: *page_index,
                        total_pages: *total_pages,
                        text: text.clone(),
                        blocks: blocks.clone(),
                    })
                    .map_err(|error| ProcessingFailure {
                        request_id: None,
                        message: format!("failed to serialize page event: {error}"),
                    })?,
                rendered_png_bytes.clone(),
            ),
            ServerEvent::Complete {
                request_id,
                page_count,
                file_size_bytes,
            } => (
                MessageKind::CompleteEvent,
                self.fory
                    .serialize(&CompleteEventMeta {
                        request_id: request_id.clone(),
                        page_count: *page_count,
                        file_size_bytes: *file_size_bytes,
                    })
                    .map_err(|error| ProcessingFailure {
                        request_id: None,
                        message: format!("failed to serialize complete event: {error}"),
                    })?,
                Vec::new(),
            ),
            ServerEvent::Error {
                request_id,
                message,
            } => (
                MessageKind::ErrorEvent,
                self.fory
                    .serialize(&ErrorEventMeta {
                        request_id: request_id.clone(),
                        message: message.clone(),
                    })
                    .map_err(|error| ProcessingFailure {
                        request_id: None,
                        message: format!("failed to serialize error event: {error}"),
                    })?,
                Vec::new(),
            ),
        };

        let payload_len = u32::try_from(payload.len()).map_err(|_| ProcessingFailure {
            request_id: None,
            message: "websocket payload too large".to_string(),
        })?;
        let mut framed = Vec::with_capacity(1 + 4 + payload.len() + binary_tail.len());
        framed.push(kind as u8);
        framed.extend_from_slice(&payload_len.to_le_bytes());
        framed.extend_from_slice(&payload);
        framed.extend_from_slice(&binary_tail);
        Ok(framed)
    }

    pub fn decode_command(&self, bytes: &[u8]) -> Result<ClientCommand, ProcessingFailure> {
        let Some((&kind, rest)) = bytes.split_first() else {
            return Err(ProcessingFailure {
                request_id: None,
                message: "empty websocket payload".to_string(),
            });
        };
        if rest.len() < 4 {
            return Err(ProcessingFailure {
                request_id: None,
                message: "missing websocket payload length".to_string(),
            });
        }
        let payload_len = u32::from_le_bytes(
            rest[..4]
                .try_into()
                .expect("slice with exact payload length bytes"),
        ) as usize;
        if rest.len() < 4 + payload_len {
            return Err(ProcessingFailure {
                request_id: None,
                message: "truncated websocket payload".to_string(),
            });
        }
        let payload = &rest[4..4 + payload_len];
        let binary_tail = &rest[4 + payload_len..];

        match MessageKind::from_u8(kind) {
            Some(MessageKind::UploadCommand) => {
                let command: UploadCommandMeta =
                    self.fory
                        .deserialize(payload)
                        .map_err(|error| ProcessingFailure {
                            request_id: None,
                            message: format!("failed to deserialize upload command: {error}"),
                        })?;
                Ok(ClientCommand::Upload(ClientUpload {
                    file_name: command.file_name,
                    pdf_bytes: binary_tail.to_vec(),
                    render_scale: RenderScale::resolve(command.render_scale)?,
                }))
            }
            Some(other) => Err(ProcessingFailure {
                request_id: None,
                message: format!("unexpected websocket command kind: {:?}", other as u8),
            }),
            None => Err(ProcessingFailure {
                request_id: None,
                message: format!("unknown websocket message kind: {kind}"),
            }),
        }
    }
}

pub async fn send_ws_event(
    codec: &WsCodec,
    session: &mut Session,
    event: &ServerEvent,
) -> Result<(), actix_ws::Closed> {
    match codec.encode_event(event) {
        Ok(payload) => session.binary(payload).await,
        Err(_) => session.clone().close(None).await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ws_protocol::ServerEvent;

    #[test]
    fn codec_round_trips_binary_messages() {
        let codec = WsCodec::new().expect("codec");
        let bytes = {
            let payload = codec
                .fory
                .serialize(&UploadCommandMeta {
                    file_name: Some("sample.pdf".to_string()),
                    render_scale: Some(2.0),
                })
                .expect("serialize");
            let mut framed = vec![MessageKind::UploadCommand as u8];
            framed.extend_from_slice(&(payload.len() as u32).to_le_bytes());
            framed.extend_from_slice(&payload);
            framed.extend_from_slice(b"foo");
            framed
        };
        let decoded = codec.decode_command(&bytes).expect("decode");

        match decoded {
            ClientCommand::Upload(ClientUpload {
                file_name,
                pdf_bytes,
                render_scale,
            }) => {
                assert_eq!(file_name.as_deref(), Some("sample.pdf"));
                assert_eq!(pdf_bytes, b"foo");
                assert_eq!(
                    render_scale,
                    RenderScale::try_new(2.0).expect("valid scale")
                );
            }
        }
    }

    #[test]
    fn codec_encodes_events() {
        let codec = WsCodec::new().expect("codec");
        let event = ServerEvent::Accepted {
            request_id: "req-1".to_string(),
        };

        let bytes = codec.encode_event(&event).expect("encode");
        assert!(!bytes.is_empty());
    }
}
