pub mod codec;
pub mod config;
pub mod pdf;
pub mod types;
pub mod upload;
pub mod websocket;
pub mod worker;
pub mod ws_protocol;

use std::sync::Arc;

use actix_web::Error;
use actix_web::error::{ErrorBadRequest, ErrorPayloadTooLarge};
use uuid::Uuid;

use crate::types::ProcessingFailure;

#[derive(Clone)]
pub struct AppState {
    pub config: config::AppConfig,
    pub job_sender: crossfire::MAsyncTx<crossfire::mpmc::Array<worker::Job>>,
    pub next_request_id: Arc<RequestIdGenerator>,
    pub ws_codec: Arc<codec::WsCodec>,
}

#[derive(Default)]
pub struct RequestIdGenerator;

pub fn allocate_request_id(state: &AppState) -> String {
    allocate_request_id_from_counter(&state.next_request_id)
}

pub fn allocate_request_id_from_counter(_counter: &Arc<RequestIdGenerator>) -> String {
    Uuid::new_v4().to_string()
}

pub fn to_http_error(error: ProcessingFailure) -> Error {
    if error.message.contains("too large") {
        ErrorPayloadTooLarge(error.message)
    } else {
        ErrorBadRequest(error.message)
    }
}

pub fn error_response(error: ProcessingFailure) -> actix_web::HttpResponse {
    let status = if error.message.contains("too large") {
        actix_web::http::StatusCode::PAYLOAD_TOO_LARGE
    } else {
        actix_web::http::StatusCode::BAD_REQUEST
    };

    actix_web::HttpResponse::build(status).json(serde_json::json!({
        "request_id": error.request_id,
        "error": error.message
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_ids_are_uuids() {
        let counter = Arc::new(RequestIdGenerator);
        let first = allocate_request_id_from_counter(&counter);
        let second = allocate_request_id_from_counter(&counter);

        assert_ne!(first, second);
        assert!(uuid::Uuid::parse_str(&first).is_ok());
        assert!(uuid::Uuid::parse_str(&second).is_ok());
    }
}
