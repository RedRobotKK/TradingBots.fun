use anyhow::Result;
use sqlx::postgres::PgPool;
use crate::config::Config;
use crate::decision::Decision;

#[derive(Clone)]
pub struct Database {
    pool: PgPool,
}

impl Database {
    pub async fn new(config: &Config) -> Result<Self> {
        let pool = PgPool::connect(&config.database_url).await?;
        
        // Run migrations
        sqlx::raw_sql(include_str!("../migrations/init.sql"))
            .execute(&pool)
            .await
            .ok();

        Ok(Database { pool })
    }

    pub async fn log_trade(&self, decision: &Decision, order_id: &str) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO trades (
                trade_id, action, confidence, position_size, leverage,
                entry_price, stop_loss, take_profit, strategy, rationale
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
            "#
        )
        .bind(order_id)
        .bind(&decision.action)
        .bind(decision.confidence)
        .bind(decision.position_size)
        .bind(decision.leverage)
        .bind(decision.entry_price)
        .bind(decision.stop_loss)
        .bind(decision.take_profit)
        .bind(&decision.strategy)
        .bind(&decision.rationale)
        .execute(&self.pool)
        .await
        .ok();

        Ok(())
    }

    pub async fn get_recent_trades(&self, limit: i32) -> Result<Vec<serde_json::Value>> {
        let trades = sqlx::query_as::<_, serde_json::Value>(
            "SELECT * FROM trades ORDER BY created_at DESC LIMIT $1"
        )
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await?;

        Ok(trades)
    }
}
