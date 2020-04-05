use askama::Template;
use main_error::MainError;
use serde::{Deserialize, Serialize};
use sqlx::postgres::PgPool;
use std::collections::HashSet;
use std::error::Error;
use std::fmt::{self, Debug, Display};
use warp::reject::Reject;
use warp::Filter;

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
    drops: i32,
    ubers: i32,
    games: i32,
    medic_time: i32,
}

impl DropStats {
    pub fn dpm(&self) -> String {
        format!(
            "{:.2}",
            self.drops as f64 / (self.medic_time as f64 / 3600.0)
        )
    }
}

async fn stats_for_user(steam_id: &str, database: &PgPool) -> Result<DropStats, DropsError> {
    let result = sqlx::query_as!(
        DropStats,
        r#"SELECT user_names.steam_id, name, games, ubers, drops, medic_time
        FROM medic_stats
        INNER JOIN user_names ON user_names.steam_id = medic_stats.steam_id
        WHERE medic_stats.steam_id=$1"#,
        steam_id
    )
    .fetch_one(database)
    .await?;

    Ok(result)
}

async fn top_stats(database: &PgPool) -> Result<Vec<DropStats>, DropsError> {
    let result = sqlx::query_as!(
        DropStats,
        r#"SELECT user_names.steam_id, games, ubers, drops, medic_time, name
        FROM medic_stats
        INNER JOIN user_names ON user_names.steam_id = medic_stats.steam_id
        ORDER BY drops DESC LIMIT 25"#
    )
    .fetch_all(database)
    .await?;

    Ok(result)
}

#[derive(Debug)]
struct GlobalStats {
    drops: i64,
    ubers: i64,
    games: i64,
}

async fn global_stats(database: &PgPool) -> Result<GlobalStats, DropsError> {
    let result = sqlx::query_as!(
        GlobalStats,
        r#"SELECT drops, ubers, games
        FROM global_stats"#
    )
    .fetch_one(database)
    .await?;

    Ok(result)
}

#[derive(Debug, Serialize)]
struct SearchResult {
    steam_id: String,
    name: String,
    count: i32,
    sim: f64,
}

impl SearchResult {
    pub fn weight(&self) -> f64 {
        self.sim * 5.0 + self.count as f64 * 1.0
    }
}

async fn player_search(search: &str, database: &PgPool) -> Result<Vec<SearchResult>, DropsError> {
    let mut players: Vec<SearchResult> = sqlx::query_as!(
        SearchResult,
        r#"SELECT steam_id, name, count, (1 - (name  <-> $1)) AS sim 
        FROM player_names
        WHERE name % $1 OR name ~* $1
        ORDER BY count DESC
        LIMIT 100"#,
        search
    )
    .fetch_all(database)
    .await?;

    players.sort_by(|a, b| b.weight().partial_cmp(&a.weight()).unwrap());

    let mut found = HashSet::new();

    Ok(players
        .into_iter()
        .filter(|player| {
            if !found.contains(&player.steam_id) {
                found.insert(player.steam_id.clone());
                true
            } else {
                false
            }
        })
        .collect())
}

#[derive(Template)]
#[template(path = "index.html")]
struct IndexTemplate {
    top: Vec<DropStats>,
    stats: GlobalStats,
}

#[derive(Template)]
#[template(path = "player.html")]
struct PlayerTemplate {
    stats: DropStats,
}

#[derive(Deserialize)]
struct SearchParams {
    search: String,
}

#[tokio::main]
async fn main() -> Result<(), MainError> {
    let database_url = dotenv::var("DATABASE_URL")?;
    let pool = PgPool::builder().max_size(2).build(&database_url).await?;

    let database = warp::any().map(move || pool.clone());

    let index = warp::path::end()
        .and(warp::get())
        .and(database.clone())
        .and_then(move |pool| async move {
            let top = match top_stats(&pool).await {
                Ok(stats) => stats,
                Err(e) => return Err(warp::reject::custom(e)),
            };
            let stats = match global_stats(&pool).await {
                Ok(stats) => stats,
                Err(e) => return Err(warp::reject::custom(e)),
            };
            let template = IndexTemplate { top, stats };
            Ok(warp::reply::html(template.render().unwrap()))
        });

    let player = warp::path!("profile" / String)
        .and(warp::get())
        .and(database.clone())
        .and_then(move |steam_id: String, pool| async move {
            let stats = match stats_for_user(&steam_id, &pool).await {
                Ok(stats) => stats,
                Err(e) => return Err(warp::reject::custom(e)),
            };
            let template = PlayerTemplate { stats };
            Ok(warp::reply::html(template.render().unwrap()))
        });

    let search = warp::path!("search")
        .and(warp::get())
        .and(warp::query())
        .and(database.clone())
        .and_then(move |query: SearchParams, pool| async move {
            let result = match player_search(&query.search, &pool).await {
                Ok(stats) => stats,
                Err(e) => return Err(warp::reject::custom(e)),
            };
            Ok(warp::reply::json(&result))
        });

    warp::serve(index.or(player).or(search))
        .run(([0, 0, 0, 0], 12345))
        .await;

    Ok(())
}
