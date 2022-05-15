use axum::extract::Path;
use axum::http::Method;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::routing::post;
use axum::Extension;
use axum::Json;
use axum::Router;
use clap::Parser;
use http::StatusCode;
use rand::prelude::*;
use serde::Serialize;
use serde_json::json;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;
use uuid::Uuid;
// use tracing::warn;

#[derive(Parser, Debug)]
#[clap(
    version, author = env!("CARGO_PKG_HOMEPAGE"), about,
)]
pub struct Settings {
    /// Listening port of http server
    #[clap(long, env("APP_PORT"), default_value("8080"))]
    pub port: u16,
    /// Listening host of http server
    #[clap(long, env("APP_HOST"), default_value("0.0.0.0"))]
    pub host: String,
    /// Minimal log level (same syntax than RUST_LOG)
    #[clap(long, env("APP_LOG_LEVEL"), default_value("info"))]
    pub log_level: String,
}

fn init_tracing(log_level: String) {
    use tracing_subscriber::filter::EnvFilter;
    use tracing_subscriber::fmt::format::FmtSpan;
    use tracing_subscriber::layer::SubscriberExt;
    // std::env::set_var("RUST_LOG", "info,kube=trace");
    std::env::set_var("RUST_LOG", std::env::var("RUST_LOG").unwrap_or(log_level));

    let fmt_layer = tracing_subscriber::fmt::layer()
        .json()
        .with_timer(tracing_subscriber::fmt::time::uptime())
        .with_span_events(FmtSpan::NEW | FmtSpan::CLOSE)
        // .with_filter(EnvFilter::from_default_env())
        ;

    // Build a subscriber that combines the access log and stdout log
    // layers.
    let subscriber = tracing_subscriber::registry()
        .with(fmt_layer)
        // .with(access_log)
        .with(EnvFilter::from_default_env());
    tracing::subscriber::set_global_default(subscriber).unwrap();
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let settings = Settings::parse();
    init_tracing(settings.log_level);
    let app = app();
    // run it
    let addr = format!("{}:{}", settings.host, settings.port).parse::<SocketAddr>()?;
    tracing::warn!("listening on {}", addr);
    axum::Server::bind(&addr)
        .serve(app.into_make_service())
        .await?;
    Ok(())
}

fn app() -> Router {
    let works: WorkDb = Arc::new(Mutex::new(HashMap::<Uuid, Work>::new()));

    // build our application with a route
    Router::new()
        .route("/health", get(health))
        .route("/start_work", post(start_work))
        .route("/work/:work_id", get(work))
        .layer(Extension(works))
        .layer(
            // see https://docs.rs/tower-http/latest/tower_http/cors/index.html
            // for more details
            CorsLayer::new()
                .allow_methods(vec![Method::GET, Method::POST])
                // allow requests from any origin
                .allow_origin(Any),
        )
        // It provides good defaults but is also very customizable.
        //
        // See https://docs.rs/tower-http/0.1.1/tower_http/trace/index.html for more details.
        .layer(TraceLayer::new_for_http())
}

async fn health() -> impl IntoResponse {
    axum::Json(json!({ "status" : "UP" }))
}

#[derive(Debug, Clone, Serialize)]
struct Work {
    work_id: Uuid,
    #[serde(skip)]
    end_at: Instant,
    duration: Duration,
    // count the number of get on this work
    nb_get_call: u16,
}

type WorkDb = Arc<Mutex<HashMap<Uuid, Work>>>;

async fn start_work(Extension(works): Extension<WorkDb>) -> impl IntoResponse {
    let mut rng: StdRng = SeedableRng::from_entropy();
    let work_id = Uuid::new_v4();
    let duration = Duration::from_secs(rng.gen_range(1..=20));
    let end_at = Instant::now() + duration;

    let get_url = format!("/work/{}", work_id);
    let next_try = duration.as_secs() / 2;

    let mut works = works.lock().expect("acquire works lock to start_work");
    works.insert(
        work_id,
        Work {
            work_id,
            end_at,
            duration,
            nb_get_call: 0,
        },
    );
    (
        StatusCode::SEE_OTHER,
        [
            (http::header::LOCATION, get_url),
            (http::header::RETRY_AFTER, format!("{}", next_try)),
        ],
    )
}

async fn work(Path(work_id): Path<Uuid>, Extension(works): Extension<WorkDb>) -> impl IntoResponse {
    let mut works = works.lock().expect("acquire works lock to get_work");
    tracing::info!(?work_id, "request work result");
    match works.get_mut(&work_id) {
        None => (StatusCode::NOT_FOUND).into_response(),
        Some(work) => {
            if work.end_at > Instant::now() {
                work.nb_get_call += 1;

                let get_url = format!("/work/{}", work.work_id);
                let next_try = 1;
                (
                    StatusCode::SEE_OTHER,
                    [
                        (http::header::LOCATION, get_url),
                        (http::header::RETRY_AFTER, format!("{}", next_try)),
                    ],
                )
                    .into_response()
            } else {
                (StatusCode::OK, Json(work.clone())).into_response()
            }
        }
    }
}

// #[cfg(test)]
// mod tests {
//     // see https://github.com/tokio-rs/axum/blob/main/examples/testing/src/main.rs
//     use super::*;
//     use assert2::{assert, check};
//     use axum::{
//         body::Body,
//         http::{Request, StatusCode},
//     };
//     use serde_json::{json, Value};
//     use std::net::{SocketAddr, TcpListener};
//     use tower::ServiceExt; // for `app.oneshot()`
//     #[tokio::test]
//     async fn simulation_with_duration() {
//         let listener = TcpListener::bind("0.0.0.0:0".parse::<SocketAddr>().unwrap()).unwrap();
//         let addr = listener.local_addr().unwrap();
//         let remote_url = format!("http://{}", addr);
//         tokio::spawn(async move {
//             axum::Server::from_tcp(listener)
//                 .unwrap()
//                 .serve(app(&remote_url).into_make_service())
//                 .await
//                 .unwrap();
//         });

//         let client = hyper::Client::new();

//         let response = client
//             .request(
//                 Request::builder()
//                     .uri(format!("http://{}/?duration_level_max=0.01&depth=1", addr))
//                     .body(Body::empty())
//                     .unwrap(),
//             )
//             .await
//             .unwrap();

//         check!(response.status() == StatusCode::OK);
//         let body = hyper::body::to_bytes(response.into_body()).await.unwrap();
//         let body: Value = serde_json::from_slice(&body).unwrap();
//         check!(body == json!({ "depth": 1, "response": { "simulation": "DONE" }}));
//     }
// }
