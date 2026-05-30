use std::io;
use std::sync::Arc;
use std::time::Duration;

use actix_multipart::Multipart;
use actix_web::error::ErrorInternalServerError;
use actix_web::{App, Error, HttpRequest, HttpResponse, HttpServer, Responder, get, post, web};
use crossfire::mpmc;
use tokio::sync::oneshot;

use mupdf_processor::codec::WsCodec;
use mupdf_processor::config::AppConfig;
use mupdf_processor::upload::collect_pdf_upload;
use mupdf_processor::websocket::{WsState, run_websocket_connection};
use mupdf_processor::worker::{JOB_QUEUE_CAPACITY, Job, JobResultTarget, spawn_workers};
use mupdf_processor::ws_protocol::RenderScale;
use mupdf_processor::{
    AppState, RequestIdGenerator, allocate_request_id, error_response, to_http_error,
};

#[get("/health")]
async fn health() -> impl Responder {
    HttpResponse::Ok().json(serde_json::json!({ "status": "ok" }))
}

#[post("/process/json")]
async fn process_pdf_json(
    mut multipart: Multipart,
    state: web::Data<AppState>,
) -> Result<HttpResponse, Error> {
    let request_id = allocate_request_id(&state);
    let upload = collect_pdf_upload(&mut multipart, &state.config)
        .await
        .map_err(to_http_error)?;
    let (tx, rx) = oneshot::channel();

    let job = Job {
        request_id: request_id.clone(),
        file_name: upload.file_name.clone(),
        source: upload.source,
        render_scale: RenderScale::default(),
        result_target: JobResultTarget::Aggregate(tx),
    };

    submit_json_job(state.get_ref().clone(), job).await?;

    match rx.await {
        Ok(Ok(response)) => Ok(HttpResponse::Ok().json(response)),
        Ok(Err(error)) => Ok(error_response(error)),
        Err(error) => Err(ErrorInternalServerError(format!(
            "worker dropped response channel: {error}"
        ))),
    }
}

#[get("/process/ws")]
async fn websocket_route(
    req: HttpRequest,
    stream: web::Payload,
    state: web::Data<AppState>,
) -> Result<HttpResponse, Error> {
    let (response, session, msg_stream) = actix_ws::handle(&req, stream)?;
    let state = WsState {
        config: state.config.clone(),
        job_sender: state.job_sender.clone(),
        next_request_id: state.next_request_id.clone(),
        ws_codec: state.ws_codec.clone(),
    };

    actix_web::rt::spawn(async move {
        run_websocket_connection(state, session, msg_stream).await;
    });

    Ok(response)
}

async fn submit_json_job(state: AppState, mut job: Job) -> Result<(), Error> {
    match state.job_sender.try_send(job) {
        Ok(()) => Ok(()),
        Err(crossfire::TrySendError::Full(full_job)) => {
            job = full_job;
            job.source = job
                .source
                .into_temp_file(job.file_name.as_deref())
                .map_err(to_http_error)?;
            state.job_sender.send(job).await.map_err(|error| {
                ErrorInternalServerError(format!("failed to enqueue queued job: {error}"))
            })
        }
        Err(crossfire::TrySendError::Disconnected(_)) => {
            Err(ErrorInternalServerError("worker queue is unavailable"))
        }
    }
}

#[actix_web::main]
async fn main() -> io::Result<()> {
    crossfire::detect_backoff_cfg();
    let config = AppConfig::from_env()?;

    let ws_codec = Arc::new(
        WsCodec::new().map_err(|error| io::Error::other(format!("fory setup failed: {error}")))?,
    );

    let (job_tx, job_rx) = mpmc::bounded_async::<Job>(JOB_QUEUE_CAPACITY);
    spawn_workers(job_rx, config.worker_count);

    let state = web::Data::new(AppState {
        config: config.clone(),
        job_sender: job_tx,
        next_request_id: Arc::new(RequestIdGenerator),
        ws_codec,
    });

    HttpServer::new(move || {
        App::new()
            .app_data(state.clone())
            .service(health)
            .service(process_pdf_json)
            .service(websocket_route)
    })
    .keep_alive(Duration::from_secs(30))
    .bind(config.bind_addr)?
    .run()
    .await
}
