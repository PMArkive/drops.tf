use askama::Template;
use main_error::MainError;
use serde::{Deserialize, Serialize};
use sqlx::postgres::PgPool;
use std::collections::HashSet;
use std::error::Error;
use std::fmt::{self, Debug, Display};
use std::str::FromStr;
use warp::reject::Reject;
use warp::{Filter, Rejection, Reply};

fn normalize_steam_id(id: &str) -> Result<String, DropsError> {
    if id.starts_with("STEAM") {
        let first = u64::from_str(&id[8..9])?;
        let second = u64::from_str(&id[10..])?;

        Ok(format!("[U:1:{}]", first + (second * 2)))
    } else if id.starts_with("[U:") {
        Ok(id.to_string())
    } else if id.starts_with("765") {
        let base = 76561197960265728u64;
        let id = u64::from_str(&id)?;
        if id > base {
            Ok(format!("[U:1:{}]", id - base))
        } else {
            Err(InvalidStreamIdFormat.into())
        }
    } else {
        Err(InvalidStreamIdFormat.into())
    }
}

#[derive(Debug)]
struct InvalidStreamIdFormat;

impl Error for InvalidStreamIdFormat {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        None
    }
}

impl Display for InvalidStreamIdFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Invalid steamid fomat")
    }
}

#[test]
fn test_steam_id() {
    assert_eq!(
        "[U:1:64229260]".to_string(),
        normalize_steam_id("76561198024494988".to_string()).unwrap()
    );
    assert_eq!(
        "[U:1:64229260]".to_string(),
        normalize_steam_id("STEAM_1:0:32114630".to_string()).unwrap()
    );
    assert_eq!(
        "[U:1:64229260]".to_string(),
        normalize_steam_id("[U:1:64229260]".to_string()).unwrap()
    );
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

impl From<DropsError> for Rejection {
    fn from(from: DropsError) -> Self {
        warp::reject::custom(from)
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
}

async fn stats_for_user(steam_id: &str, database: &PgPool) -> Result<DropStats, DropsError> {
    let result = sqlx::query_as!(
        DropStats,
        r#"SELECT user_names.steam_id, name, games, ubers, drops, medic_time,
        (SELECT COUNT(*) FROM medic_stats m2 WHERE m2.drops > medic_stats.drops AND m2.drops > 100) + 1 AS drops_rank,
        (SELECT COUNT(*) FROM medic_stats m2 WHERE m2.dpu > medic_stats.dpu AND m2.drops > 100) + 1 AS dpu_rank,
        (SELECT COUNT(*) FROM medic_stats m2 WHERE m2.dps > medic_stats.dps AND m2.drops > 100) + 1 AS dps_rank,
        (SELECT COUNT(*) FROM medic_stats m2 WHERE m2.dpg > medic_stats.dpg AND m2.drops > 100) + 1 AS dpg_rank
        FROM medic_stats
        INNER JOIN user_names ON user_names.steam_id = medic_stats.steam_id
        WHERE medic_stats.steam_id=$1"#,
        steam_id
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

async fn top_stats(database: &PgPool, order: TopOrder) -> Result<Vec<TopStats>, DropsError> {
    let result = match order {
        TopOrder::Drops => {
            sqlx::query_as!(
                TopStats,
                r#"SELECT user_names.steam_id, games, ubers, drops, medic_time, name
                FROM medic_stats
                INNER JOIN user_names ON user_names.steam_id = medic_stats.steam_id
                WHERE drops > 100 AND medic_stats.steam_id != 'BOT'
                ORDER BY drops DESC LIMIT 25"#
            )
            .fetch_all(database)
            .await?
        }
        TopOrder::Dps => {
            sqlx::query_as!(
                TopStats,
                r#"SELECT user_names.steam_id, games, ubers, drops, medic_time, name
                FROM medic_stats
                INNER JOIN user_names ON user_names.steam_id = medic_stats.steam_id
                WHERE drops > 100 AND medic_stats.steam_id != 'BOT'
                ORDER BY dps DESC LIMIT 25"#
            )
            .fetch_all(database)
            .await?
        }
        TopOrder::Dpu => {
            sqlx::query_as!(
                TopStats,
                r#"SELECT user_names.steam_id, games, ubers, drops, medic_time, name
                FROM medic_stats
                INNER JOIN user_names ON user_names.steam_id = medic_stats.steam_id
                WHERE drops > 100 AND medic_stats.steam_id != 'BOT'
                ORDER BY dpu DESC LIMIT 25"#
            )
            .fetch_all(database)
            .await?
        }
        TopOrder::Dpg => {
            sqlx::query_as!(
                TopStats,
                r#"SELECT user_names.steam_id, games, ubers, drops, medic_time, name
                FROM medic_stats
                INNER JOIN user_names ON user_names.steam_id = medic_stats.steam_id
                WHERE drops > 100 AND medic_stats.steam_id != 'BOT'
                ORDER BY dpg DESC LIMIT 25"#
            )
            .fetch_all(database)
            .await?
        }
    };

    Ok(result)
}

async fn get_user_name(steam_id: &str, database: &PgPool) -> Result<Option<String>, DropsError> {
    let result = sqlx::query!(
        r#"SELECT name
        FROM user_names
        WHERE steam_id=$1"#,
        steam_id
    )
    .fetch_one(database)
    .await?;

    Ok(result.name)
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
    count: i64,
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
        WHERE name ~* $1
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
    top: Vec<TopStats>,
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

async fn page_top_stats(pool: PgPool, order: TopOrder) -> Result<impl Reply, Rejection> {
    let top = top_stats(&pool, order).await?;
    let stats = global_stats(&pool).await?;
    let template = IndexTemplate { top, stats };
    Ok(warp::reply::html(template.render().unwrap()))
}

async fn page_player(steam_id: String, pool: PgPool) -> Result<impl Reply, Rejection> {
    let stats = stats_for_user(&normalize_steam_id(&steam_id)?, &pool).await?;
    let template = PlayerTemplate { stats };
    Ok(warp::reply::html(template.render().unwrap()))
}

async fn api_search(query: SearchParams, pool: PgPool) -> Result<impl Reply, Rejection> {
    if let Ok(steam_id) = normalize_steam_id(&query.search) {
        if let Some(name) = get_user_name(&steam_id, &pool).await? {
            return Ok(warp::reply::json(&vec![SearchResult {
                steam_id,
                name,
                count: 1,
                sim: 1.0,
            }]));
        }
    }
    let result = player_search(&query.search, &pool).await?;
    Ok(warp::reply::json(&result))
}

#[tokio::main]
async fn main() -> Result<(), MainError> {
    let database_url = dotenv::var("DATABASE_URL")?;
    let port = u16::from_str(&dotenv::var("PORT")?)?;

    let pool = PgPool::builder().max_size(2).build(&database_url).await?;

    let database = warp::any().map(move || pool.clone());

    let index = warp::path::end()
        .and(warp::get())
        .and(database.clone())
        .and_then(move |pool| page_top_stats(pool, TopOrder::Drops));

    let dpg = warp::path::path("dpg")
        .and(warp::get())
        .and(database.clone())
        .and_then(move |pool| page_top_stats(pool, TopOrder::Dpg));

    let dps = warp::path::path("dph")
        .and(warp::get())
        .and(database.clone())
        .and_then(move |pool| page_top_stats(pool, TopOrder::Dps));

    let dpu = warp::path::path("dpu")
        .and(warp::get())
        .and(database.clone())
        .and_then(move |pool| page_top_stats(pool, TopOrder::Dpu));

    let player = warp::path!("profile" / String)
        .and(warp::get())
        .and(database.clone())
        .and_then(page_player);

    let search = warp::path!("search")
        .and(warp::get())
        .and(warp::query())
        .and(database.clone())
        .and_then(api_search);

    warp::serve(index.or(dpg).or(dpu).or(dps).or(player).or(search))
        .run(([0, 0, 0, 0], port))
        .await;

    Ok(())
}
