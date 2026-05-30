use std::sync::Arc;

use actix_ws::Session;
use fory::Fory;

use crate::types::{
    PageResult, PointDto, ProcessingFailure, ProcessingResponse, QuadDto, RectDto,
    TextBlockResult, TextCharResult, WsClientCommand, WsServerEvent,
};

const TYPE_ID_RECT_DTO: u32 = 100;
const TYPE_ID_POINT_DTO: u32 = 101;
const TYPE_ID_QUAD_DTO: u32 = 102;
const TYPE_ID_TEXT_CHAR_RESULT: u32 = 103;
const TYPE_ID_TEXT_BLOCK_RESULT: u32 = 104;
const TYPE_ID_PAGE_RESULT: u32 = 105;
const TYPE_ID_PROCESSING_RESPONSE: u32 = 106;
const TYPE_ID_WS_CLIENT_COMMAND: u32 = 107;
const TYPE_ID_WS_SERVER_EVENT: u32 = 108;

pub struct WsCodec {
    fory: Arc<Fory>,
}

impl WsCodec {
    pub fn new() -> Result<Self, fory::Error> {
        let mut fory = Fory::builder().xlang(false).compatible(true).build();
        fory.register::<RectDto>(TYPE_ID_RECT_DTO)?;
        fory.register::<PointDto>(TYPE_ID_POINT_DTO)?;
        fory.register::<QuadDto>(TYPE_ID_QUAD_DTO)?;
        fory.register::<TextCharResult>(TYPE_ID_TEXT_CHAR_RESULT)?;
        fory.register::<TextBlockResult>(TYPE_ID_TEXT_BLOCK_RESULT)?;
        fory.register::<PageResult>(TYPE_ID_PAGE_RESULT)?;
        fory.register::<ProcessingResponse>(TYPE_ID_PROCESSING_RESPONSE)?;
        fory.register::<WsClientCommand>(TYPE_ID_WS_CLIENT_COMMAND)?;
        fory.register::<WsServerEvent>(TYPE_ID_WS_SERVER_EVENT)?;
        Ok(Self {
            fory: Arc::new(fory),
        })
    }

    pub fn encode_event(
        &self,
        event: &WsServerEvent,
    ) -> Result<Vec<u8>, ProcessingFailure> {
        self.fory.serialize(event).map_err(|error| ProcessingFailure {
            request_id: None,
            message: format!("failed to serialize websocket event: {error}"),
        })
    }

    pub fn decode_command(
        &self,
        bytes: &[u8],
    ) -> Result<WsClientCommand, ProcessingFailure> {
        self.fory.deserialize(bytes).map_err(|error| ProcessingFailure {
            request_id: None,
            message: format!("failed to deserialize websocket command: {error}"),
        })
    }
}

pub async fn send_ws_event(
    codec: &WsCodec,
    session: &mut Session,
    event: &WsServerEvent,
) -> Result<(), actix_ws::Closed> {
    match codec.encode_event(event) {
        Ok(payload) => session.binary(payload).await,
        Err(_) => session.clone().close(None).await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::WsClientCommand;

    #[test]
    fn codec_round_trips_binary_messages() {
        let codec = WsCodec::new().expect("codec");
        let command = WsClientCommand::Upload {
            file_name: Some("sample.pdf".to_string()),
            pdf_bytes: b"foo".to_vec(),
        };

        let bytes = codec.fory.serialize(&command).expect("serialize");
        let decoded = codec.decode_command(&bytes).expect("decode");

        match decoded {
            WsClientCommand::Upload {
                file_name,
                pdf_bytes,
            } => {
                assert_eq!(file_name.as_deref(), Some("sample.pdf"));
                assert_eq!(pdf_bytes, b"foo");
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn codec_encodes_events() {
        let codec = WsCodec::new().expect("codec");
        let event = WsServerEvent::Accepted {
            request_id: "req-1".to_string(),
        };

        let bytes = codec.encode_event(&event).expect("encode");
        assert!(!bytes.is_empty());
    }
}
