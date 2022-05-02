use crate::search::api_search;
use askama::Template;
use main_error::MainError;
use sqlx::postgres::PgPool;
use std::convert::TryInto;
use std::error::Error;
use std::fmt::{self, Debug, Display};
use std::str::FromStr;
use steamid_ng::SteamID;
use tracing::instrument;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use warp::reject::Reject;
use warp::{Filter, Rejection, Reply};

mod search;

macro_rules! named {
    ($name:expr) => {
        warp::trace(|info| tracing::info_span!(
            $name,
            method = %info.method(),
            path = %info.path(),
        ))
    };
}

struct DropsError(Box<dyn Error + Send + Sync + 'static>);

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

#[derive(Debug)]
struct DropStats {
    steam_id: String,
    name: String,
    drops: i64,
    ubers: i64,
    games: i64,
    medic_time: i64,
    drops_rank: i64,
    dpu_rank: i64,
    dps_rank: i64,
    dpg_rank: i64,
}

impl DropStats {
    pub fn dpm(&self) -> String {
        format!(
            "{:.2}",
            self.drops as f64 / (self.medic_time as f64 / 3600.0)
        )
    }

    pub fn dpu(&self) -> String {
        format!("{:.2}", self.drops as f64 / self.ubers as f64)
    }

    pub fn dpg(&self) -> String {
        format!("{:.2}", self.drops as f64 / self.games as f64)
    }

    pub fn steam_link(&self) -> String {
        format!("https://steamcommunity.com/profiles/{}", self.steam_id)
    }

    pub fn etf2l_link(&self) -> String {
        format!(
            "http://etf2l.org/search/{}",
            &self.steam_id[1..self.steam_id.len() - 1]
        )
    }

    pub fn ugc_link(&self) -> String {
        let steam_id_64 = u64::from(SteamID::from_steam3(&self.steam_id).unwrap());

        format!(
            "https://www.ugcleague.com/players_page.cfm?player_id={}",
            steam_id_64
        )
    }

    pub fn logs_link(&self) -> String {
        let steam_id_64 = u64::from(SteamID::from_steam3(&self.steam_id).unwrap());

        format!("http://logs.tf/profile/{}", steam_id_64)
    }

    pub fn demos_link(&self) -> String {
        let steam_id_64 = u64::from(SteamID::from_steam3(&self.steam_id).unwrap());

        format!("http://demos.tf/profiles/{}", steam_id_64)
    }

    pub fn rgl_link(&self) -> String {
        let steam_id_64 = u64::from(SteamID::from_steam3(&self.steam_id).unwrap());

        format!("https://rgl.gg/Public/PlayerProfile.aspx?p={}", steam_id_64)
    }
}

#[instrument(skip(database))]
async fn stats_for_user(steam_id: SteamID, database: &PgPool) -> Result<DropStats, DropsError> {
    // for medics with more than 100 drops we have cached info
    if let Ok(result) = sqlx::query_as!(
        DropStats,
        r#"SELECT steam_id as "steam_id!", name as "name!", games as "games!", ubers as "ubers!", drops as "drops!",
        medic_time as "medic_time!", drops_rank as "drops_rank!", dpu_rank as "dpu_rank!", dps_rank as "dps_rank!", dpg_rank as "dpg_rank!"
        FROM ranked_medic_stats
        WHERE steam_id=$1"#,
        steam_id.steam3()
    )
        .fetch_one(database)
        .await {
        return Ok(result);
    }

    // for other we need to recalculate
    let result = sqlx::query_as!(
        DropStats,
        r#"SELECT user_names.steam_id as "steam_id!", name as "name!", games as "games!", ubers as "ubers!", drops as "drops!", medic_time as "medic_time!",
        (SELECT COUNT(*) FROM ranked_medic_stats m2 WHERE m2.drops > medic_stats.drops AND m2.drops > 100) + 1 AS "drops_rank!",
        (SELECT COUNT(*) FROM ranked_medic_stats m2 WHERE m2.dpu > medic_stats.dpu AND m2.drops > 100) + 1 AS "dpu_rank!",
        (SELECT COUNT(*) FROM ranked_medic_stats m2 WHERE m2.dps > medic_stats.dps AND m2.drops > 100) + 1 AS "dps_rank!",
        (SELECT COUNT(*) FROM ranked_medic_stats m2 WHERE m2.dpg > medic_stats.dpg AND m2.drops > 100) + 1 AS "dpg_rank!"
        FROM medic_stats
        INNER JOIN user_names ON user_names.steam_id = medic_stats.steam_id
        WHERE medic_stats.steam_id=$1"#,
        steam_id.steam3()
    )
        .fetch_one(database)
        .await?;

    Ok(result)
}

#[derive(Debug)]
struct TopStats {
    steam_id: String,
    name: String,
    drops: i64,
    ubers: i64,
    games: i64,
    medic_time: i64,
}

impl TopStats {
    pub fn dpm(&self) -> String {
        format!(
            "{:.2}",
            self.drops as f64 / (self.medic_time as f64 / 3600.0)
        )
    }

    pub fn dpu(&self) -> String {
        format!("{:.2}", self.drops as f64 / self.ubers as f64)
    }

    pub fn dpg(&self) -> String {
        format!("{:.2}", self.drops as f64 / self.games as f64)
    }
}

#[instrument(skip(database))]
async fn top_stats(database: &PgPool, order: TopOrder) -> Result<Vec<TopStats>, DropsError> {
    let result = match order {
        TopOrder::Drops => {
            sqlx::query_as!(
                TopStats,
                r#"SELECT steam_id as "steam_id!", games as "games!", ubers as "ubers!", drops as "drops!", medic_time as "medic_time!", name as "name!"
                FROM ranked_medic_stats
                ORDER BY drops DESC LIMIT 25"#
            )
                .fetch_all(database)
                .await?
        }
        TopOrder::Dps => {
            sqlx::query_as!(
                TopStats,
                r#"SELECT steam_id as "steam_id!", games as "games!", ubers as "ubers!", drops as "drops!", medic_time as "medic_time!", name as "name!"
                FROM ranked_medic_stats
                ORDER BY dps DESC LIMIT 25"#
            )
                .fetch_all(database)
                .await?
        }
        TopOrder::Dpu => {
            sqlx::query_as!(
                TopStats,
                r#"SELECT steam_id as "steam_id!", games as "games!", ubers as "ubers!", drops as "drops!", medic_time as "medic_time!", name as "name!"
                FROM ranked_medic_stats
                ORDER BY dpu DESC LIMIT 25"#
            )
                .fetch_all(database)
                .await?
        }
        TopOrder::Dpg => {
            sqlx::query_as!(
                TopStats,
                r#"SELECT steam_id as "steam_id!", games as "games!", ubers as "ubers!", drops as "drops!", medic_time as "medic_time!", name as "name!"
                FROM ranked_medic_stats
                ORDER BY dpg DESC LIMIT 25"#
            )
                .fetch_all(database)
                .await?
        }
    };

    Ok(result)
}

#[derive(Debug)]
struct GlobalStats {
    drops: i64,
    ubers: i64,
    games: i64,
}

#[instrument(skip(database))]
async fn global_stats(database: &PgPool) -> Result<GlobalStats, DropsError> {
    let result = sqlx::query_as!(
        GlobalStats,
        r#"SELECT drops as "drops!", ubers as "ubers!", games as "games!"
        FROM global_stats"#
    )
    .fetch_one(database)
    .await?;

    Ok(result)
}

#[derive(Template)]
#[template(path = "index.html")]
struct IndexTemplate {
    top: Vec<TopStats>,
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

#[derive(Debug)]
enum TopOrder {
    Drops,
    Dps,
    Dpg,
    Dpu,
}

impl Display for TopOrder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}",
            match self {
                TopOrder::Drops => "drops",
                TopOrder::Dps => "dps",
                TopOrder::Dpg => "dpg",
                TopOrder::Dpu => "dpu",
            }
        )
    }
}

#[instrument(skip(pool))]
async fn page_top_stats(pool: PgPool, order: TopOrder) -> Result<impl Reply, Rejection> {
    let top = top_stats(&pool, order).await?;
    let stats = global_stats(&pool).await?;
    let template = IndexTemplate { top, stats };
    Ok(warp::reply::html(template.render().unwrap()))
}

#[instrument(skip(database, api_key))]
async fn resolve_vanity_url(
    database: &PgPool,
    url: &str,
    api_key: &str,
) -> Result<Option<SteamID>, DropsError> {
    if let Ok(row) = sqlx::query!(r#"SELECT steam_id FROM vanity_urls WHERE url=$1"#, url)
        .fetch_one(database)
        .await
    {
        let steam_id: String = row.steam_id;
        Ok(Some(SteamID::from_steam3(&steam_id)?))
    } else {
        if let Some(steam_id) = steam_resolve_vanity::resolve_vanity_url(url, api_key).await? {
            sqlx::query!(
                r#"INSERT INTO vanity_urls(url, steam_id) VALUES($1, $2)"#,
                url,
                steam_id.steam3()
            )
            .execute(database)
            .await?;

            Ok(Some(steam_id))
        } else {
            Ok(None)
        }
    }
}

#[instrument(skip(pool, api_key))]
async fn page_player(
    steam_id: String,
    pool: PgPool,
    api_key: String,
) -> Result<impl Reply, Rejection> {
    let steam_id = match steam_id.as_str().try_into().map_err(DropsError::from) {
        Ok(steam_id) => steam_id,
        Err(e) => resolve_vanity_url(&pool, &steam_id, &api_key)
            .await?
            .ok_or(e)?,
    };
    let stats = match stats_for_user(steam_id, &pool).await {
        Ok(stats) => stats,
        Err(_) => {
            let template = NotFoundTemplate;
            return Ok(warp::reply::html(template.render().unwrap()));
        }
    };
    let template = PlayerTemplate { stats };
    Ok(warp::reply::html(template.render().unwrap()))
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

    let database = warp::any().map(move || pool.clone());

    let api_key = warp::any().map(move || api_key.clone());

    let index = warp::path::end()
        .and(warp::get())
        .and(database.clone())
        .and_then(move |pool| page_top_stats(pool, TopOrder::Drops))
        .with(named!("index"));

    let dpg = warp::path::path("dpg")
        .and(warp::get())
        .and(database.clone())
        .and_then(move |pool| page_top_stats(pool, TopOrder::Dpg))
        .with(named!("dpg"));

    let dps = warp::path::path("dph")
        .and(warp::get())
        .and(database.clone())
        .and_then(move |pool| page_top_stats(pool, TopOrder::Dps))
        .with(named!("dph"));

    let dpu = warp::path::path("dpu")
        .and(warp::get())
        .and(database.clone())
        .and_then(move |pool| page_top_stats(pool, TopOrder::Dpu))
        .with(named!("dpu"));

    let player = warp::path!("profile" / String)
        .and(warp::get())
        .and(database.clone())
        .and(api_key.clone())
        .and_then(page_player)
        .with(named!("profile"));

    let search = warp::path!("search")
        .and(warp::get())
        .and(warp::query())
        .and(database.clone())
        .and_then(api_search)
        .with(named!("search"));

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
