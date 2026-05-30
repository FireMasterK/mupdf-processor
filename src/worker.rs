use std::thread;

use crossfire::{MAsyncRx, mpmc};
use tokio::sync::{mpsc, oneshot};

use crate::pdf::{process_pdf_all, process_pdf_streaming};
use crate::types::{ProcessingFailure, ProcessingResponse, WsServerEvent};
use crate::upload::UploadedPdf;

pub const JOB_QUEUE_CAPACITY: usize = 16;
pub struct Job {
    pub request_id: String,
    pub file_name: Option<String>,
    pub source: UploadedPdf,
    pub result_target: JobResultTarget,
}

pub enum JobResultTarget {
    Aggregate(oneshot::Sender<Result<ProcessingResponse, ProcessingFailure>>),
    WebSocket(mpsc::UnboundedSender<WsServerEvent>),
}

pub fn spawn_workers(rx: MAsyncRx<mpmc::Array<Job>>, worker_count: usize) {
    for worker_id in 0..worker_count {
        let worker_rx = rx.clone().into_blocking();
        thread::Builder::new()
            .name(format!("pdf-worker-{worker_id}"))
            .spawn(move || {
                while let Ok(job) = worker_rx.recv() {
                    let request_id = job.request_id.clone();

                    match job.result_target {
                        JobResultTarget::Aggregate(responder) => {
                            let result = process_pdf_all(request_id.clone(), job.source)
                                .map_err(|mut error| {
                                    error.request_id = error.request_id.take().or(Some(request_id));
                                    error
                                });
                            let _ = responder.send(result);
                        }
                        JobResultTarget::WebSocket(event_tx) => {
                            if let Err(mut error) =
                                process_pdf_streaming(request_id.clone(), job.source, event_tx.clone())
                            {
                                error.request_id = error.request_id.take().or(Some(request_id));
                                let _ = event_tx.send(WsServerEvent::Error {
                                    request_id: error.request_id,
                                    message: error.message,
                                });
                            }
                        }
                    }
                }
            })
            .expect("failed to spawn worker");
    }
}
