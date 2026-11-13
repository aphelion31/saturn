//! Port of Plane's `recent_visited_task`. Records a visit in `user_recent_visits`, deduping on
//! (user, workspace, entity, identifier, project). Keeps at most 20 rows per user and workspace by
//! evicting the oldest `created_at`. Callers spawn it, so GET latency is unaffected.

use sqlx::PgPool;
use uuid::Uuid;

const CAP: i64 = 20;

pub struct Visit<'a> {
    pub workspace_id: Uuid,
    pub user_id: Uuid,
    pub entity_name: &'a str,
    pub entity_identifier: Uuid,
    pub project_id: Option<Uuid>,
}

/// Fire-and-forget. Errors are logged, never returned to the request.
pub fn record(db: &PgPool, v: Visit<'static>) {
    let db = db.clone();
    tokio::spawn(async move {
        if let Err(e) = upsert(&db, &v).await {
            tracing::warn!(error = %e, "recent visit upsert failed");
        }
    });
}

async fn upsert(db: &PgPool, v: &Visit<'_>) -> Result<(), sqlx::Error> {
    let mut tx = db.begin().await?;
    // Serialise per user+workspace so the cap check cannot race.
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended('recent_visits:' || $1::text, 0))")
        .bind(v.user_id)
        .execute(&mut *tx)
        .await?;

    let updated = sqlx::query(
        "UPDATE user_recent_visits SET visited_at = now(), updated_at = now()
         WHERE workspace_id = $1 AND user_id = $2 AND entity_name = $3 AND entity_identifier = $4
           AND project_id IS NOT DISTINCT FROM $5",
    )
    .bind(v.workspace_id)
    .bind(v.user_id)
    .bind(v.entity_name)
    .bind(v.entity_identifier)
    .bind(v.project_id)
    .execute(&mut *tx)
    .await?;

    if updated.rows_affected() == 0 {
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM user_recent_visits WHERE user_id = $1 AND workspace_id = $2")
            .bind(v.user_id)
            .bind(v.workspace_id)
            .fetch_one(&mut *tx)
            .await?;
        if count >= CAP {
            sqlx::query(
                "DELETE FROM user_recent_visits WHERE id = (
                    SELECT id FROM user_recent_visits WHERE user_id = $1 AND workspace_id = $2
                    ORDER BY created_at ASC LIMIT 1)",
            )
            .bind(v.user_id)
            .bind(v.workspace_id)
            .execute(&mut *tx)
            .await?;
        }
        sqlx::query(
            "INSERT INTO user_recent_visits (entity_name, entity_identifier, user_id, visited_at, project_id,
                                             workspace_id, created_by_id, updated_by_id)
             VALUES ($1, $2, $3, now(), $4, $5, $3, $3)",
        )
        .bind(v.entity_name)
        .bind(v.entity_identifier)
        .bind(v.user_id)
        .bind(v.project_id)
        .bind(v.workspace_id)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await
}
