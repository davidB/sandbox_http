use axum::extract::Path;
use axum::http::Method;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::routing::post;
use axum::Extension;
use axum::Json;
use axum::Router;
use http::StatusCode;
use rand::prelude::*;
use serde::Serialize;
use serde_json::json;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;
use uuid::Uuid;
// use tracing::warn;

pub fn app() -> Router {
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
