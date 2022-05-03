use crate::data::{DataSource, DropStats, GlobalStats, SearchParams, TopOrder, TopStats};
use askama::Template;
use axum::extract::{Path, Query};
use axum::handler::Handler;
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Response};
use axum::routing::get;
use axum::{Extension, Json, Router};
use main_error::MainError;
use sqlx::postgres::PgPool;
use std::convert::TryInto;
use std::fmt::Debug;
use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::Arc;
use thiserror::Error;
use tower_http::trace::TraceLayer;
use tracing::instrument;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::Layer;

mod data;

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
}

impl IntoResponse for DropsError {
    fn into_response(self) -> Response {
        let status = match &self {
            DropsError::SteamId(_) => StatusCode::BAD_REQUEST,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        (status, format!("{}", self)).into_response()
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
#[template(path = "notfound.html")]
struct NotFoundTemplate;

#[derive(Template)]
#[template(path = "404.html")]
struct PageNotFoundTemplate;

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
    let steam_id = match steam_id.as_str().try_into().map_err(DropsError::from) {
        Ok(steam_id) => steam_id,
        Err(e) => data_source.resolve_vanity_url(&steam_id).await?.ok_or(e)?,
    };
    let stats = match data_source.stats_for_user(steam_id).await {
        Ok(stats) => stats,
        Err(_) => {
            let template = NotFoundTemplate;
            return Ok(Html(template.render()?));
        }
    };
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
    let template = PageNotFoundTemplate;
    (StatusCode::NOT_FOUND, Html(template.render().unwrap()))
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
        .layer(Extension(data_source))
        .layer(TraceLayer::new_for_http())
        .fallback(handler_404.into_service());

    // let not_found = warp::any().map(|| {
    //     return Ok(warp::reply::html(PageNotFoundTemplate.render().unwrap()));
    // });

    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    tracing::debug!("listening on {}", addr);
    axum::Server::bind(&addr)
        .serve(app.into_make_service())
        .await
        .unwrap();

    Ok(())
}
