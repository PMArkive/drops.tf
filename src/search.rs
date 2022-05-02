use crate::DropsError;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::collections::HashSet;
use std::convert::TryInto;
use steamid_ng::SteamID;
use tracing::instrument;
use warp::{Rejection, Reply};

#[derive(Debug, Deserialize)]
pub struct SearchParams {
    search: String,
}

#[instrument(skip(database))]
async fn get_user_name(steam_id: SteamID, database: &PgPool) -> Result<Option<String>, DropsError> {
    let result = sqlx::query!(
        r#"SELECT name
        FROM user_names
        WHERE steam_id=$1"#,
        steam_id.steam3()
    )
    .fetch_one(database)
    .await?;

    Ok(result.name)
}

#[instrument(skip(pool))]
pub async fn api_search(query: SearchParams, pool: PgPool) -> Result<impl Reply, Rejection> {
    if let Ok(steam_id) = query.search.as_str().try_into() {
        if let Some(name) = get_user_name(steam_id, &pool).await? {
            return Ok(warp::reply::json(&vec![SearchResult {
                steam_id: steam_id.steam3(),
                name,
                count: 1,
                sim: 1.0,
            }]));
        }
    }
    let result = player_search(&query.search, &pool).await?;
    Ok(warp::reply::json(&result))
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

#[instrument(skip(database))]
async fn player_search(search: &str, database: &PgPool) -> Result<Vec<SearchResult>, DropsError> {
    let mut players: Vec<SearchResult> = sqlx::query_as!(
        SearchResult,
        r#"SELECT steam_id as "steam_id!", name as "name!", count as "count!", (1 - (name  <-> $1)) AS "sim!" 
        FROM medic_names
        WHERE name ~* $1
        ORDER BY count DESC
        LIMIT 50"#,
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
