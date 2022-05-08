use crate::steam_id::SteamId;
use crate::DropsError;
use moka::future::Cache;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::collections::HashSet;
use std::fmt;
use std::fmt::Display;
use std::sync::Arc;
use std::time::Duration;
use tracing::instrument;

#[derive(Clone)]
pub struct DataSource {
    global_cache: Cache<(), GlobalStats>,
    top_cache: Cache<TopOrder, Arc<Vec<TopStats>>>,
    player_cache: Cache<SteamId, DropStats>,
    database: PgPool,
    api_key: String,
}

impl DataSource {
    pub fn new(database: PgPool, api_key: String) -> Self {
        DataSource {
            global_cache: Cache::builder()
                .time_to_live(Duration::from_secs(15 * 60))
                .time_to_idle(Duration::from_secs(5 * 60))
                .build(),
            top_cache: Cache::builder()
                .time_to_live(Duration::from_secs(15 * 60))
                .time_to_idle(Duration::from_secs(5 * 60))
                .build(),
            player_cache: Cache::builder()
                .time_to_live(Duration::from_secs(15 * 60))
                .time_to_idle(Duration::from_secs(5 * 60))
                .max_capacity(1024)
                .build(),
            database,
            api_key,
        }
    }

    #[instrument(skip(self))]
    pub async fn player_search(&self, search: &str) -> Result<Vec<SearchResult>, DropsError> {
        if let Ok(steam_id) = search.parse() {
            if let Some(name) = self.get_user_name(steam_id).await? {
                return Ok(vec![SearchResult {
                    steam_id,
                    name,
                    count: 1,
                    sim: 1.0,
                }]);
            }
        }
        self.player_wildcard_search(search).await
    }

    #[instrument(skip(self))]
    async fn get_user_name(&self, steam_id: SteamId) -> Result<Option<String>, DropsError> {
        let result = sqlx::query!(
            r#"SELECT name FROM user_names WHERE steam_id=$1"#,
            steam_id.steam3()
        )
        .fetch_one(&self.database)
        .await?;

        Ok(result.name)
    }

    #[instrument(skip(self))]
    async fn player_wildcard_search(&self, search: &str) -> Result<Vec<SearchResult>, DropsError> {
        let mut players: Vec<SearchResult> = sqlx::query_as!(
            SearchResult,
            r#"SELECT steam_id as "steam_id!: _", name as "name!", count as "count!", (1 - (name  <-> $1)) AS "sim!" 
            FROM medic_names
            WHERE name ~* $1
            ORDER BY count DESC
            LIMIT 50"#,
            search
        )
            .fetch_all(&self.database)
            .await?;

        players.sort_by(|a, b| b.weight().partial_cmp(&a.weight()).unwrap());

        let mut found = HashSet::new();

        Ok(players
            .into_iter()
            .filter(|player| {
                if found.contains(&player.steam_id) {
                    false
                } else {
                    found.insert(player.steam_id);
                    true
                }
            })
            .collect())
    }

    #[instrument(skip(self))]
    pub async fn stats_for_user(&self, steam_id: SteamId) -> Result<DropStats, DropsError> {
        let result = self.player_cache.try_get_with(steam_id, async {
            // for medics with more than 100 drops we have cached info
            if let Ok(result) = sqlx::query_as!(
                DropStats,
                r#"SELECT steam_id as "steam_id!: _", name as "name!", games as "games!", ubers as "ubers!", drops as "drops!",
                medic_time as "medic_time!", drops_rank as "drops_rank!", dpu_rank as "dpu_rank!", dps_rank as "dps_rank!", dpg_rank as "dpg_rank!"
                FROM ranked_medic_stats
                WHERE steam_id=$1"#,
                steam_id.steam3()
            )
                .fetch_one(&self.database)
                .await {
                Ok(result)
            } else {
                // for other we need to recalculate
                sqlx::query_as!(
                    DropStats,
                    r#"SELECT user_names.steam_id as "steam_id!: _", name as "name!", games as "games!", ubers as "ubers!", drops as "drops!", medic_time as "medic_time!",
                    (SELECT COUNT(*) FROM ranked_medic_stats m2 WHERE m2.drops > medic_stats.drops AND m2.drops > 100) + 1 AS "drops_rank!",
                    (SELECT COUNT(*) FROM ranked_medic_stats m2 WHERE m2.dpu > medic_stats.dpu AND m2.drops > 100) + 1 AS "dpu_rank!",
                    (SELECT COUNT(*) FROM ranked_medic_stats m2 WHERE m2.dps > medic_stats.dps AND m2.drops > 100) + 1 AS "dps_rank!",
                    (SELECT COUNT(*) FROM ranked_medic_stats m2 WHERE m2.dpg > medic_stats.dpg AND m2.drops > 100) + 1 AS "dpg_rank!"
                    FROM medic_stats
                    INNER JOIN user_names ON user_names.steam_id = medic_stats.steam_id
                    WHERE medic_stats.steam_id=$1"#,
                    steam_id.steam3()
                )
                    .fetch_one(&self.database)
                    .await
            }
        }).await?;
        Ok(result)
    }

    #[instrument(skip(self))]
    pub async fn top_stats(&self, order: TopOrder) -> Result<Arc<Vec<TopStats>>, DropsError> {
        let result = self.top_cache.try_get_with::<_, sqlx::Error>(order, async {
            let result = match order {
                TopOrder::Drops => {
                    sqlx::query_as!(
                        TopStats,
                        r#"SELECT steam_id as "steam_id!: _", games as "games!", ubers as "ubers!", drops as "drops!", medic_time as "medic_time!", name as "name!"
                        FROM ranked_medic_stats
                        ORDER BY drops DESC LIMIT 25"#
                    )
                        .fetch_all(&self.database)
                        .await?
                }
                TopOrder::Dps => {
                    sqlx::query_as!(
                        TopStats,
                        r#"SELECT steam_id as "steam_id!: _", games as "games!", ubers as "ubers!", drops as "drops!", medic_time as "medic_time!", name as "name!"
                        FROM ranked_medic_stats
                        ORDER BY dps DESC LIMIT 25"#
                    )
                        .fetch_all(&self.database)
                        .await?
                }
                TopOrder::Dpu => {
                    sqlx::query_as!(
                        TopStats,
                        r#"SELECT steam_id as "steam_id!: _", games as "games!", ubers as "ubers!", drops as "drops!", medic_time as "medic_time!", name as "name!"
                        FROM ranked_medic_stats
                        ORDER BY dpu DESC LIMIT 25"#
                    )
                        .fetch_all(&self.database)
                        .await?
                }
                TopOrder::Dpg => {
                    sqlx::query_as!(
                        TopStats,
                        r#"SELECT steam_id as "steam_id!: _", games as "games!", ubers as "ubers!", drops as "drops!", medic_time as "medic_time!", name as "name!"
                        FROM ranked_medic_stats
                        ORDER BY dpg DESC LIMIT 25"#
                    )
                        .fetch_all(&self.database)
                        .await?
                }
            };
            Ok(Arc::new(result))
        }).await?;

        Ok(result)
    }

    #[instrument(skip(self))]
    pub async fn global_stats(&self) -> Result<GlobalStats, DropsError> {
        let result = self.global_cache
            .try_get_with(
                (),
                sqlx::query_as!(
                        GlobalStats,
                        r#"SELECT drops as "drops!", ubers as "ubers!", games as "games!" FROM global_stats"#
                    )
                    .fetch_one(&self.database),
            )
            .await?;

        Ok(result)
    }

    #[instrument(skip(self))]
    pub async fn resolve_vanity_url(&self, url: &str) -> Result<Option<SteamId>, DropsError> {
        if let Ok(row) = sqlx::query!(
            r#"SELECT steam_id as "steam_id!: SteamId" FROM vanity_urls WHERE url=$1"#,
            url
        )
        .fetch_one(&self.database)
        .await
        {
            Ok(Some(row.steam_id))
        } else if let Some(steam_id) =
            steam_resolve_vanity::resolve_vanity_url(url, &self.api_key).await?
        {
            sqlx::query!(
                r#"INSERT INTO vanity_urls(url, steam_id) VALUES($1, $2)"#,
                url,
                steam_id.steam3()
            )
            .execute(&self.database)
            .await?;

            Ok(Some(SteamId::from(steam_id)))
        } else {
            Ok(None)
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct SearchParams {
    pub search: String,
}

#[derive(Debug, Serialize)]
pub struct SearchResult {
    pub steam_id: SteamId,
    pub name: String,
    pub count: i64,
    pub sim: f64,
}

impl SearchResult {
    pub fn weight(&self) -> f64 {
        self.sim * 5.0 + self.count as f64
    }
}

#[derive(Debug, Clone)]
pub struct DropStats {
    pub steam_id: SteamId,
    pub name: String,
    pub drops: i64,
    pub ubers: i64,
    pub games: i64,
    pub medic_time: i64,
    pub drops_rank: i64,
    pub dpu_rank: i64,
    pub dps_rank: i64,
    pub dpg_rank: i64,
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
        format!(
            "https://steamcommunity.com/profiles/{}",
            u64::from(self.steam_id)
        )
    }

    pub fn etf2l_link(&self) -> String {
        format!("http://etf2l.org/search/{}", u64::from(self.steam_id))
    }

    pub fn ugc_link(&self) -> String {
        format!(
            "https://www.ugcleague.com/players_page.cfm?player_id={}",
            u64::from(self.steam_id)
        )
    }

    pub fn logs_link(&self) -> String {
        format!("http://logs.tf/profile/{}", u64::from(self.steam_id))
    }

    pub fn demos_link(&self) -> String {
        format!("http://demos.tf/profiles/{}", u64::from(self.steam_id))
    }

    pub fn rgl_link(&self) -> String {
        format!(
            "https://rgl.gg/Public/PlayerProfile.aspx?p={}",
            u64::from(self.steam_id)
        )
    }
}

#[derive(Debug, Clone)]
pub struct TopStats {
    pub steam_id: SteamId,
    pub name: String,
    pub drops: i64,
    pub ubers: i64,
    pub games: i64,
    pub medic_time: i64,
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

    pub fn steam_id64(&self) -> u64 {
        self.steam_id.into()
    }
}

#[derive(Debug, Clone)]
pub struct GlobalStats {
    pub drops: i64,
    pub ubers: i64,
    pub games: i64,
}

#[derive(Debug, Hash, Eq, PartialEq, Clone, Copy)]
pub enum TopOrder {
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
