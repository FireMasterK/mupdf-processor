use std::sync::Arc;

use actix_ws::{AggregatedMessage, MessageStream, Session};
use crossfire::{MAsyncTx, TrySendError, mpmc};
use tokio::sync::mpsc;

use crate::codec::{WsCodec, send_ws_event};
use crate::config::AppConfig;
use crate::types::WsServerEvent;
use crate::upload::{CollectedUpload, spill_bytes_if_needed};
use crate::worker::{Job, JobResultTarget};

pub const WS_MAX_FRAME_BYTES: usize = 192 * 1024 * 1024;

#[derive(Clone)]
pub struct WsState {
    pub config: AppConfig,
    pub job_sender: MAsyncTx<mpmc::Array<Job>>,
    pub next_request_id: Arc<crate::RequestIdGenerator>,
    pub ws_codec: Arc<WsCodec>,
}

pub async fn run_websocket_connection(
    state: WsState,
    session: Session,
    msg_stream: MessageStream,
) {
    let (event_tx, mut event_rx) = mpsc::unbounded_channel::<WsServerEvent>();
    let codec = state.ws_codec.clone();

    let mut outbound = session.clone();
    let outbound_codec = codec.clone();
    let outbound_task = actix_web::rt::spawn(async move {
        while let Some(event) = event_rx.recv().await {
            if send_ws_event(&outbound_codec, &mut outbound, &event)
                .await
                .is_err()
            {
                break;
            }
        }
        let _ = outbound.close(None).await;
    });

    let mut session = session;
    let mut aggregated = msg_stream
        .aggregate_continuations()
        .max_continuation_size(WS_MAX_FRAME_BYTES);

    while let Some(result) = aggregated.recv().await {
        match result {
            Ok(AggregatedMessage::Ping(bytes)) => {
                if session.pong(&bytes).await.is_err() {
                    break;
                }
            }
            Ok(AggregatedMessage::Pong(_)) => {}
            Ok(AggregatedMessage::Binary(bytes)) => {
                handle_ws_binary_command(&state, event_tx.clone(), bytes.to_vec()).await;
            }
            Ok(AggregatedMessage::Text(_)) => {
                let _ = event_tx.send(WsServerEvent::Error {
                    request_id: None,
                    message: "text websocket frames are not supported; send Apache Fory binary messages"
                        .to_string(),
                });
            }
            Ok(AggregatedMessage::Close(reason)) => {
                let _ = session.close(reason).await;
                break;
            }
            Err(error) => {
                let _ = event_tx.send(WsServerEvent::Error {
                    request_id: None,
                    message: format!("websocket protocol error: {error}"),
                });
                break;
            }
        }
    }

    drop(event_tx);
    let _ = outbound_task.await;
}

pub async fn handle_ws_binary_command(
    state: &WsState,
    event_tx: mpsc::UnboundedSender<WsServerEvent>,
    payload: Vec<u8>,
) {
    let command = match state.ws_codec.decode_command(&payload) {
        Ok(command) => command,
        Err(error) => {
            let _ = event_tx.send(WsServerEvent::Error {
                request_id: None,
                message: error.message,
            });
            return;
        }
    };

    match command {
        crate::types::WsClientCommand::Unknown => {
            let _ = event_tx.send(WsServerEvent::Error {
                request_id: None,
                message: "unknown websocket command".to_string(),
            });
        }
        crate::types::WsClientCommand::Upload {
            file_name,
            pdf_bytes,
        } => {
            let request_id =
                crate::allocate_request_id_from_counter(&state.next_request_id);
            let upload = match spill_bytes_if_needed(pdf_bytes, file_name.clone(), &state.config) {
                Ok(upload) => upload,
                Err(error) => {
                    let _ = event_tx.send(WsServerEvent::Error {
                        request_id: Some(request_id),
                        message: error.message,
                    });
                    return;
                }
            };

            submit_ws_job(state, event_tx, request_id, upload).await;
        }
    }
}

async fn submit_ws_job(
    state: &WsState,
    event_tx: mpsc::UnboundedSender<WsServerEvent>,
    request_id: String,
    upload: CollectedUpload,
) {
    let mut job = Job {
        request_id: request_id.clone(),
        file_name: upload.file_name,
        source: upload.source,
        result_target: JobResultTarget::WebSocket(event_tx.clone()),
    };

    match state.job_sender.try_send(job) {
        Ok(()) => {
            let _ = event_tx.send(WsServerEvent::Accepted { request_id });
        }
        Err(TrySendError::Full(full_job)) => {
            job = full_job;
            match job.source.into_temp_file(job.file_name.as_deref()) {
                Ok(spooled_source) => {
                    job.source = spooled_source;
                    let _ = event_tx.send(WsServerEvent::Accepted {
                        request_id: request_id.clone(),
                    });
                    let sender = state.job_sender.clone();
                    let error_tx = event_tx.clone();
                    actix_web::rt::spawn(async move {
                        if let Err(error) = sender.send(job).await {
                            let _ = error_tx.send(WsServerEvent::Error {
                                request_id: Some(request_id),
                                message: format!(
                                    "failed to enqueue queued websocket job: {error}"
                                ),
                            });
                        }
                    });
                }
                Err(error) => {
                    let _ = event_tx.send(WsServerEvent::Error {
                        request_id: Some(request_id),
                        message: error.message,
                    });
                }
            }
        }
        Err(TrySendError::Disconnected(disconnected_job)) => {
            let _ = event_tx.send(WsServerEvent::Error {
                request_id: Some(disconnected_job.request_id),
                message: "worker queue is unavailable".to_string(),
            });
        }
    }
}
