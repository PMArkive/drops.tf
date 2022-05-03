use crate::data::{
    DataSource, DropStats, GlobalStats, SearchParams, SearchResult, TopOrder, TopStats,
};
use askama::Template;
use main_error::MainError;
use sqlx::postgres::PgPool;
use std::convert::TryInto;
use std::error::Error;
use std::fmt::{self, Debug, Display};
use std::str::FromStr;
use tracing::instrument;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use warp::reject::Reject;
use warp::{Filter, Rejection, Reply};

mod data;

pub struct DropsError(Box<dyn Error + Send + Sync + 'static>);

impl<E: Into<Box<dyn Error + Send + Sync + 'static>>> From<E> for DropsError {
    fn from(e: E) -> Self {
        DropsError(e.into())
    }
}

impl Reject for DropsError {}

impl Debug for DropsError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        Display::fmt(&self.0, f)?;
        let mut source = self.0.source();
        while let Some(error) = source {
            write!(f, "\ncaused by: {}", error)?;
            source = error.source();
        }
        Ok(())
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
async fn page_top_stats(data_source: DataSource, order: TopOrder) -> Result<impl Reply, Rejection> {
    let top = data_source.top_stats(order).await?;
    let stats = data_source.global_stats().await?;
    let template = IndexTemplate {
        top: top.as_slice(),
        stats,
    };
    Ok(warp::reply::html(template.render().unwrap()))
}

#[instrument(skip(data_source))]
async fn page_player(data_source: DataSource, steam_id: String) -> Result<impl Reply, Rejection> {
    let steam_id = match steam_id.as_str().try_into().map_err(DropsError::from) {
        Ok(steam_id) => steam_id,
        Err(e) => data_source.resolve_vanity_url(&steam_id).await?.ok_or(e)?,
    };
    let stats = match data_source.stats_for_user(steam_id).await {
        Ok(stats) => stats,
        Err(_) => {
            let template = NotFoundTemplate;
            return Ok(warp::reply::html(template.render().unwrap()));
        }
    };
    let template = PlayerTemplate { stats };
    Ok(warp::reply::html(template.render().unwrap()))
}

#[instrument(skip(data_source))]
pub async fn api_search(
    data_source: DataSource,
    query: SearchParams,
) -> Result<impl Reply, Rejection> {
    if let Ok(steam_id) = query.search.as_str().try_into() {
        if let Some(name) = data_source.get_user_name(steam_id).await? {
            return Ok(warp::reply::json(&vec![SearchResult {
                steam_id: steam_id.steam3(),
                name,
                count: 1,
                sim: 1.0,
            }]));
        }
    }
    let result = data_source.player_search(&query.search).await?;
    Ok(warp::reply::json(&result))
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
            .with(open_telemetry)
            .try_init()?;
    }

    let database_url = dotenv::var("DATABASE_URL")?;
    let api_key = dotenv::var("STEAM_API_KEY")?;
    let port = u16::from_str(&dotenv::var("PORT")?)?;

    let pool = PgPool::connect(&database_url).await?;
    let data_source = DataSource::new(pool, api_key);

    let data_source = warp::any().map(move || data_source.clone());

    let index = warp::path::end()
        .and(warp::get())
        .and(data_source.clone())
        .and_then(move |data_source| page_top_stats(data_source, TopOrder::Drops));

    let dpg = warp::path::path("dpg")
        .and(warp::get())
        .and(data_source.clone())
        .and_then(move |data_source| page_top_stats(data_source, TopOrder::Dpg));

    let dps = warp::path::path("dph")
        .and(warp::get())
        .and(data_source.clone())
        .and_then(move |data_source| page_top_stats(data_source, TopOrder::Dps));

    let dpu = warp::path::path("dpu")
        .and(warp::get())
        .and(data_source.clone())
        .and_then(move |data_source| page_top_stats(data_source, TopOrder::Dpu));

    let player = warp::path!("profile" / String)
        .and(warp::get())
        .and(data_source.clone())
        .and_then(|steam_id, data_source| page_player(data_source, steam_id));

    let search = warp::path!("search")
        .and(warp::get())
        .and(data_source.clone())
        .and(warp::query())
        .and_then(api_search);

    let not_found = warp::any().map(|| {
        return Ok(warp::reply::html(PageNotFoundTemplate.render().unwrap()));
    });

    warp::serve(
        index
            .or(dpg)
            .or(dpu)
            .or(dps)
            .or(player)
            .or(search)
            .or(not_found),
    )
    .run(([0, 0, 0, 0], port))
    .await;

    Ok(())
}
