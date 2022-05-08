use crate::data::{DataSource, DropStats, GlobalStats, SearchParams, TopOrder, TopStats};
use askama::Template;
use axum::extract::{MatchedPath, Path, Query};
use axum::handler::Handler;
use axum::http::{Request, StatusCode};
use axum::middleware::Next;
use axum::response::{Html, IntoResponse, Response};
use axum::routing::get;
use axum::{middleware, Extension, Json, Router};
use main_error::MainError;
use metrics_exporter_prometheus::{Matcher, PrometheusBuilder, PrometheusHandle};
use sqlx::postgres::PgPool;
use std::borrow::Cow;
use std::fmt::Debug;
use std::future::ready;
use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::Arc;
use thiserror::Error;
use tokio::time::Instant;
use tower_http::trace::TraceLayer;
use tracing::{error, instrument};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::Layer;

mod data;
mod steam_id;

#[derive(Debug, Error)]
pub enum DropsError {
    #[error(transparent)]
    SteamId(#[from] steamid_ng::SteamIDError),
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    #[error(transparent)]
    DatabaseArc(#[from] Arc<sqlx::Error>),
    #[error("Error while resolving steam url")]
    Steam(#[from] steam_resolve_vanity::Error),
    #[error("Error while rendering template")]
    Template(#[from] askama::Error),
    #[error("404 - Page not found")]
    NotFound,
    #[error("User not found or no drops")]
    UserNotFound,
}

impl IntoResponse for DropsError {
    fn into_response(self) -> Response {
        let status = match &self {
            DropsError::SteamId(_) => StatusCode::BAD_REQUEST,
            DropsError::NotFound | DropsError::UserNotFound => StatusCode::NOT_FOUND,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        let template = ErrorTemplate {
            error: Cow::Owned(format!("{}", self)),
        };
        (
            status,
            Html(
                template
                    .render()
                    .unwrap_or_else(|_| "Error rendering error".into()),
            ),
        )
            .into_response()
    }
}

#[derive(Template)]
#[template(path = "index.html")]
struct IndexTemplate<'a> {
    top: &'a [TopStats],
    stats: GlobalStats,
}

#[derive(Template)]
#[template(path = "player.html")]
struct PlayerTemplate {
    stats: DropStats,
}

#[derive(Template)]
#[template(path = "error.html")]
struct ErrorTemplate {
    error: Cow<'static, str>,
}

#[instrument(skip(data_source))]
async fn page_top_stats(
    Extension(data_source): Extension<DataSource>,
    order: TopOrder,
) -> Result<impl IntoResponse, DropsError> {
    let top = data_source.top_stats(order).await?;
    let stats = data_source.global_stats().await?;
    let template = IndexTemplate {
        top: top.as_slice(),
        stats,
    };

    Ok(Html(template.render()?))
}

#[instrument(skip(data_source))]
async fn page_player(
    Extension(data_source): Extension<DataSource>,
    Path(steam_id): Path<String>,
) -> Result<impl IntoResponse, DropsError> {
    let steam_id = match steam_id.parse().map_err(DropsError::from) {
        Ok(steam_id) => steam_id,
        Err(e) => data_source
            .resolve_vanity_url(&steam_id)
            .await?
            .ok_or(e)
            .map_err(|e| {
                error!(steam_id = display(steam_id), "user not found");
                e
            })?,
    };
    let stats = data_source.stats_for_user(steam_id).await.map_err(|_| {
        error!(steam_id = u64::from(steam_id), "no logs found for user");
        DropsError::UserNotFound
    })?;

    metrics::increment_counter!(
        "player_stats",
        &[
            ("steam_id", format!("{}", u64::from(steam_id))),
            ("name", stats.name.clone())
        ]
    );

    let template = PlayerTemplate { stats };
    Ok(Html(template.render()?))
}

#[instrument(skip(data_source))]
pub async fn api_search(
    Extension(data_source): Extension<DataSource>,
    Query(query): Query<SearchParams>,
) -> Result<impl IntoResponse, DropsError> {
    let result = data_source.player_search(&query.search).await?;
    Ok(Json(result))
}

async fn handler_404() -> impl IntoResponse {
    DropsError::NotFound
}

#[tokio::main]
async fn main() -> Result<(), MainError> {
    if let Ok(tracing_endpoint) = dotenv::var("TRACING_ENDPOINT") {
        let tracer = opentelemetry_jaeger::new_pipeline()
            .with_agent_endpoint(tracing_endpoint)
            .with_service_name("drops.tf")
            .install_simple()?;
        let open_telemetry = tracing_opentelemetry::layer().with_tracer(tracer);
        tracing_subscriber::registry()
            .with(tracing_subscriber::EnvFilter::new(
                std::env::var("RUST_LOG")
                    .unwrap_or_else(|_| "dropstf=debug,tower_http=debug,sqlx=debug".into()),
            ))
            .with(open_telemetry)
            .with(
                tracing_subscriber::fmt::layer().with_filter(tracing_subscriber::EnvFilter::new(
                    std::env::var("RUST_LOG").unwrap_or_else(|_| "warn".into()),
                )),
            )
            .try_init()?;
    }

    let database_url = dotenv::var("DATABASE_URL")?;
    let api_key = dotenv::var("STEAM_API_KEY")?;
    let port = u16::from_str(&dotenv::var("PORT")?)?;

    let pool = PgPool::connect(&database_url).await?;
    let data_source = DataSource::new(pool, api_key);

    let recorder_handle = setup_metrics_recorder();

    let app = Router::new()
        .route(
            "/",
            get(|data_source| page_top_stats(data_source, TopOrder::Drops)),
        )
        .route(
            "/dpg",
            get(|data_source| page_top_stats(data_source, TopOrder::Dpg)),
        )
        .route(
            "/dph",
            get(|data_source| page_top_stats(data_source, TopOrder::Dps)),
        )
        .route(
            "/dpu",
            get(|data_source| page_top_stats(data_source, TopOrder::Dpu)),
        )
        .route("/profile/:steam_id", get(page_player))
        .route("/search", get(api_search))
        .route("/metrics", get(move || ready(recorder_handle.render())))
        .route_layer(middleware::from_fn(track_metrics))
        .layer(Extension(data_source))
        .layer(TraceLayer::new_for_http())
        .fallback(handler_404.into_service());

    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    tracing::debug!("listening on {}", addr);
    axum::Server::bind(&addr)
        .serve(app.into_make_service())
        .await?;

    Ok(())
}

fn setup_metrics_recorder() -> PrometheusHandle {
    const EXPONENTIAL_SECONDS: &[f64] = &[
        0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0,
    ];

    PrometheusBuilder::new()
        .set_buckets_for_metric(
            Matcher::Full("http_requests_duration_seconds".to_string()),
            EXPONENTIAL_SECONDS,
        )
        .unwrap()
        .install_recorder()
        .unwrap()
}

async fn track_metrics<B>(req: Request<B>, next: Next<B>) -> impl IntoResponse {
    let start = Instant::now();
    let path = if let Some(matched_path) = req.extensions().get::<MatchedPath>() {
        matched_path.as_str().to_owned()
    } else {
        req.uri().path().to_owned()
    };
    let method = req.method().clone();

    let response = next.run(req).await;

    let latency = start.elapsed().as_secs_f64();
    let status = response.status().as_u16().to_string();

    if path != "/metrics" {
        let labels = [
            ("method", method.to_string()),
            ("path", path),
            ("status", status),
        ];

        metrics::increment_counter!("http_requests_total", &labels);
        metrics::histogram!("http_requests_duration_seconds", latency, &labels);
    }

    response
}
