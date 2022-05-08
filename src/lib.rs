pub use crate::data::{DataSource, DropStats, GlobalStats, SearchParams, TopOrder, TopStats};
use askama::Template;
use axum::extract::{Path, Query};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Response};
use axum::{Extension, Json};
use std::borrow::Cow;
use std::fmt::Debug;
use std::sync::Arc;
use thiserror::Error;
use tracing::{error, instrument};

mod data;
mod steam_id;
mod str;

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
pub struct IndexTemplate<'a> {
    pub top: &'a [TopStats],
    pub stats: GlobalStats,
}

#[derive(Template)]
#[template(path = "player.html")]
pub struct PlayerTemplate {
    pub stats: DropStats,
}

#[derive(Template)]
#[template(path = "error.html")]
pub struct ErrorTemplate {
    pub error: Cow<'static, str>,
}

#[instrument(skip(data_source))]
pub async fn page_top_stats(
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
pub async fn page_player(
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
            ("name", stats.name.to_string())
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

pub async fn handler_404() -> impl IntoResponse {
    DropsError::NotFound
}
