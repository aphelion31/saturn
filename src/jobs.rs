//! Scheduled background tasks ported from Plane's Celery beat schedule:
//! - `archive_and_close_old_issues` (daily 01:00 UTC)
//! - `unsnooze_expired_intake_issues` (every 5 min)
//! - `hard_delete` (daily 00:00 UTC)
//! - `delete_unuploaded_file_asset` (daily 02:00 UTC)
//! - `delete_api_logs` (daily 02:30 UTC)
//! - `delete_email_notification_logs` (daily 02:45 UTC)
//! - `delete_page_versions` (daily 03:00 UTC)
//! - `delete_issue_description_versions` (daily 03:15 UTC)
//! - `delete_webhook_logs` (daily 03:30 UTC)
//! - `delete_old_s3_link` (daily 01:30 and 03:45 UTC)
//!
//! Note on telemetry `push_instance_metrics`: deliberately not ported (no phoning home in Saturn).

use std::time::Duration;
use chrono::{DateTime, NaiveTime, Timelike, Utc};
use uuid::Uuid;

use crate::common::AppState;

pub const JOB_NAMES: &[&str] = &[
    "archive_and_close_old_issues",
    "unsnooze_expired_intake_issues",
    "hard_delete",
    "delete_unuploaded_file_asset",
    "delete_api_logs",
    "delete_email_notification_logs",
    "delete_page_versions",
    "delete_issue_description_versions",
    "delete_webhook_logs",
    "delete_old_s3_link",
];

/// Computes the duration until the next 5-minute boundary (:00, :05, :10, ..., :55) in UTC.
pub fn duration_until_next_5min(now: DateTime<Utc>) -> Duration {
    let second = now.second();
    let minute = now.minute();
    let next_min = ((minute / 5) + 1) * 5;
    let delta_secs = if next_min == 60 {
        ((60 - minute) * 60) - second
    } else {
        ((next_min - minute) * 60) - second
    };
    // If delta_secs is 0 (exactly on the boundary with nanoseconds), bump to next interval
    let delta_secs = if delta_secs == 0 { 300 } else { delta_secs };
    Duration::from_secs(delta_secs as u64)
}

/// Computes duration until the next occurrence of (hour, minute) UTC.
pub fn duration_until_next_daily(now: DateTime<Utc>, target_hour: u32, target_minute: u32) -> Duration {
    let today = now.date_naive();
    let target_time = NaiveTime::from_hms_opt(target_hour, target_minute, 0).expect("valid hms");
    let target_dt = today.and_time(target_time).and_utc();

    let next_run = if target_dt > now {
        target_dt
    } else {
        (today + chrono::Duration::days(1)).and_time(target_time).and_utc()
    };

    let diff = (next_run - now).num_seconds().max(1);
    Duration::from_secs(diff as u64)
}

/// Computes sleep duration until the next run of a named scheduled task.
pub fn duration_until_next_run(job_name: &str, now: DateTime<Utc>) -> Option<Duration> {
    match job_name {
        "unsnooze_expired_intake_issues" => Some(duration_until_next_5min(now)),
        "hard_delete" => Some(duration_until_next_daily(now, 0, 0)),
        "archive_and_close_old_issues" => Some(duration_until_next_daily(now, 1, 0)),
        "delete_old_s3_link" => {
            // Plane runs at 01:30 and 03:45 UTC. Next one wins.
            let d1 = duration_until_next_daily(now, 1, 30);
            let d2 = duration_until_next_daily(now, 3, 45);
            Some(d1.min(d2))
        }
        "delete_unuploaded_file_asset" => Some(duration_until_next_daily(now, 2, 0)),
        "delete_api_logs" => Some(duration_until_next_daily(now, 2, 30)),
        "delete_email_notification_logs" => Some(duration_until_next_daily(now, 2, 45)),
        "delete_page_versions" => Some(duration_until_next_daily(now, 3, 0)),
        "delete_issue_description_versions" => Some(duration_until_next_daily(now, 3, 15)),
        "delete_webhook_logs" => Some(duration_until_next_daily(now, 3, 30)),
        _ => None,
    }
}

/// Deterministic 64-bit lock key for pg_try_advisory_lock per job name.
fn advisory_lock_key(job_name: &str) -> i64 {
    // 64-bit FNV-1a hash
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in job_name.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash as i64
}

/// Executes a single job under a transaction holding an advisory lock.
pub async fn run_job_locked(st: &AppState, job_name: &str) -> anyhow::Result<bool> {
    let lock_key = advisory_lock_key(job_name);
    let mut tx = st.db.begin().await?;

    let locked: bool = sqlx::query_scalar("SELECT pg_try_advisory_xact_lock($1)")
        .bind(lock_key)
        .fetch_one(&mut *tx)
        .await?;

    if !locked {
        tracing::info!(job = job_name, "scheduled job skipped; another instance holds the lock");
        return Ok(false);
    }

    match job_name {
        "archive_and_close_old_issues" => run_archive_and_close(&mut tx).await?,
        "unsnooze_expired_intake_issues" => run_unsnooze_intake(&mut tx).await?,
        "hard_delete" => run_hard_delete(&mut tx).await?,
        "delete_unuploaded_file_asset" => run_delete_unuploaded_assets(st, &mut tx).await?,
        "delete_api_logs" => run_delete_api_logs(&mut tx).await?,
        "delete_email_notification_logs" => run_delete_email_logs(&mut tx).await?,
        "delete_page_versions" => run_delete_page_versions(&mut tx).await?,
        "delete_issue_description_versions" => run_delete_issue_description_versions(&mut tx).await?,
        "delete_webhook_logs" => run_delete_webhook_logs(&mut tx).await?,
        "delete_old_s3_link" => run_delete_old_s3_link(st, &mut tx).await?,
        unknown => anyhow::bail!("unknown job: {unknown}"),
    }

    tx.commit().await?;
    Ok(true)
}

/// Helper for issue activity rows in jobs.rs (avoids cross-worker collisions with src/activity.rs).
async fn insert_issue_activity(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    issue_id: Uuid,
    project_id: Uuid,
    workspace_id: Uuid,
    actor_id: Option<Uuid>,
    field: Option<&str>,
    old_value: Option<&str>,
    new_value: Option<&str>,
    comment: &str,
    new_identifier: Option<Uuid>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO issue_activities (
            issue_id, project_id, workspace_id, actor_id, created_by_id, updated_by_id,
            verb, field, old_value, new_value, comment, attachments, new_identifier, epoch
         ) VALUES ($1, $2, $3, $4, $4, $4, 'updated', $5, $6, $7, $8, ARRAY[]::varchar[], $9, extract(epoch from now()))"
    )
    .bind(issue_id)
    .bind(project_id)
    .bind(workspace_id)
    .bind(actor_id)
    .bind(field)
    .bind(old_value)
    .bind(new_value)
    .bind(comment)
    .bind(new_identifier)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// 1. archive_and_close_old_issues
/// Projects with archive_in > 0: archive completed/cancelled issues older than archive_in * 30 days.
/// Projects with close_in > 0: close backlog/unstarted/started issues older than close_in * 30 days to default_state (or cancelled state).
async fn run_archive_and_close(tx: &mut sqlx::Transaction<'_, sqlx::Postgres>) -> anyhow::Result<()> {
    // --- 1a. Archive ---
    #[derive(sqlx::FromRow)]
    struct ArchiveProject {
        id: Uuid,
        workspace_id: Uuid,
        archive_in: i32,
        created_by_id: Option<Uuid>,
    }

    let archive_projects: Vec<ArchiveProject> = sqlx::query_as(
        "SELECT id, workspace_id, archive_in, created_by_id FROM projects WHERE archive_in > 0 AND deleted_at IS NULL"
    )
    .fetch_all(&mut **tx)
    .await?;

    let today = Utc::now().date_naive();

    for p in archive_projects {
        let days = (p.archive_in as i64) * 30;
        let cutoff = Utc::now() - chrono::Duration::days(days);

        // Find candidate issues matching Plane's filters
        let issue_ids: Vec<Uuid> = sqlx::query_scalar(
            "SELECT i.id FROM issues i
             JOIN states s ON s.id = i.state_id AND s.deleted_at IS NULL
             WHERE i.project_id = $1
               AND i.workspace_id = $2
               AND i.archived_at IS NULL
               AND i.deleted_at IS NULL
               AND i.updated_at <= $3
               AND s.group IN ('completed', 'cancelled')
               AND (
                   NOT EXISTS (SELECT 1 FROM cycle_issues ci WHERE ci.issue_id = i.id AND ci.deleted_at IS NULL)
                   OR EXISTS (
                       SELECT 1 FROM cycle_issues ci
                       JOIN cycles c ON c.id = ci.cycle_id AND c.deleted_at IS NULL
                       WHERE ci.issue_id = i.id AND ci.deleted_at IS NULL AND c.end_date < now()
                   )
               )
               AND (
                   NOT EXISTS (SELECT 1 FROM module_issues mi WHERE mi.issue_id = i.id AND mi.deleted_at IS NULL)
                   OR EXISTS (
                       SELECT 1 FROM module_issues mi
                       JOIN modules m ON m.id = mi.module_id AND m.deleted_at IS NULL
                       WHERE mi.issue_id = i.id AND mi.deleted_at IS NULL AND m.target_date < now()
                   )
               )
               AND (
                   NOT EXISTS (SELECT 1 FROM intake_issues ii WHERE ii.issue_id = i.id AND ii.deleted_at IS NULL)
                   OR EXISTS (
                       SELECT 1 FROM intake_issues ii
                       WHERE ii.issue_id = i.id AND ii.deleted_at IS NULL AND ii.status IN (-1, 1, 2)
                   )
               )
             FOR UPDATE OF i"
        )
        .bind(p.id)
        .bind(p.workspace_id)
        .bind(cutoff)
        .fetch_all(&mut **tx)
        .await?;

        if !issue_ids.is_empty() {
            sqlx::query(
                "UPDATE issues SET archived_at = $1, updated_at = now() WHERE id = ANY($2)"
            )
            .bind(today)
            .bind(&issue_ids)
            .execute(&mut **tx)
            .await?;

            for issue_id in issue_ids {
                insert_issue_activity(
                    tx,
                    issue_id,
                    p.id,
                    p.workspace_id,
                    p.created_by_id,
                    Some("archived_at"),
                    None,
                    Some("archive"),
                    "Plane has archived the issue",
                    None,
                )
                .await?;
            }
        }
    }

    // --- 1b. Close ---
    #[derive(sqlx::FromRow)]
    struct CloseProject {
        id: Uuid,
        workspace_id: Uuid,
        close_in: i32,
        default_state_id: Option<Uuid>,
        created_by_id: Option<Uuid>,
    }

    let close_projects: Vec<CloseProject> = sqlx::query_as(
        "SELECT id, workspace_id, close_in, default_state_id, created_by_id FROM projects WHERE close_in > 0 AND deleted_at IS NULL"
    )
    .fetch_all(&mut **tx)
    .await?;

    for p in close_projects {
        let days = (p.close_in as i64) * 30;
        let cutoff = Utc::now() - chrono::Duration::days(days);

        // Resolve target state (default_state or first cancelled state in project)
        let target_state: Option<(Uuid, String)> = if let Some(ds_id) = p.default_state_id {
            sqlx::query_as("SELECT id, name FROM states WHERE id = $1 AND project_id = $2 AND deleted_at IS NULL")
                .bind(ds_id)
                .bind(p.id)
                .fetch_optional(&mut **tx)
                .await?
        } else {
            sqlx::query_as("SELECT id, name FROM states WHERE project_id = $1 AND \"group\" = 'cancelled' AND deleted_at IS NULL ORDER BY created_at LIMIT 1")
                .bind(p.id)
                .fetch_optional(&mut **tx)
                .await?
        };

        let (target_state_id, target_state_name) = match target_state {
            Some(s) => s,
            None => continue,
        };

        let issue_ids: Vec<Uuid> = sqlx::query_scalar(
            "SELECT i.id FROM issues i
             JOIN states s ON s.id = i.state_id AND s.deleted_at IS NULL
             WHERE i.project_id = $1
               AND i.workspace_id = $2
               AND i.archived_at IS NULL
               AND i.deleted_at IS NULL
               AND i.updated_at <= $3
               AND s.group IN ('backlog', 'unstarted', 'started')
               AND (
                   NOT EXISTS (SELECT 1 FROM cycle_issues ci WHERE ci.issue_id = i.id AND ci.deleted_at IS NULL)
                   OR EXISTS (
                       SELECT 1 FROM cycle_issues ci
                       JOIN cycles c ON c.id = ci.cycle_id AND c.deleted_at IS NULL
                       WHERE ci.issue_id = i.id AND ci.deleted_at IS NULL AND c.end_date < now()
                   )
               )
               AND (
                   NOT EXISTS (SELECT 1 FROM module_issues mi WHERE mi.issue_id = i.id AND mi.deleted_at IS NULL)
                   OR EXISTS (
                       SELECT 1 FROM module_issues mi
                       JOIN modules m ON m.id = mi.module_id AND m.deleted_at IS NULL
                       WHERE mi.issue_id = i.id AND mi.deleted_at IS NULL AND m.target_date < now()
                   )
               )
               AND (
                   NOT EXISTS (SELECT 1 FROM intake_issues ii WHERE ii.issue_id = i.id AND ii.deleted_at IS NULL)
                   OR EXISTS (
                       SELECT 1 FROM intake_issues ii
                       WHERE ii.issue_id = i.id AND ii.deleted_at IS NULL AND ii.status IN (-1, 1, 2)
                   )
               )
             FOR UPDATE OF i"
        )
        .bind(p.id)
        .bind(p.workspace_id)
        .bind(cutoff)
        .fetch_all(&mut **tx)
        .await?;

        if !issue_ids.is_empty() {
            sqlx::query(
                "UPDATE issues SET state_id = $1, updated_at = now() WHERE id = ANY($2)"
            )
            .bind(target_state_id)
            .bind(&issue_ids)
            .execute(&mut **tx)
            .await?;

            for issue_id in issue_ids {
                insert_issue_activity(
                    tx,
                    issue_id,
                    p.id,
                    p.workspace_id,
                    p.created_by_id,
                    Some("state"),
                    None,
                    Some(&target_state_name),
                    "Plane updated the state to ",
                    Some(target_state_id),
                )
                .await?;
            }
        }
    }

    Ok(())
}

/// 2. unsnooze_expired_intake_issues
/// Set intake_issues.status = -2 (PENDING), snoozed_till = NULL where status = 0 (SNOOZED) and snoozed_till <= now.
async fn run_unsnooze_intake(tx: &mut sqlx::Transaction<'_, sqlx::Postgres>) -> anyhow::Result<()> {
    let count = sqlx::query(
        "UPDATE intake_issues
         SET status = -2, snoozed_till = NULL, updated_at = now()
         WHERE status = 0 AND snoozed_till <= now() AND deleted_at IS NULL"
    )
    .execute(&mut **tx)
    .await?
    .rows_affected();

    if count > 0 {
        tracing::info!(count, "unsnoozed intake issues returned to pending");
    }
    Ok(())
}

/// 3. hard_delete
/// Deletes rows whose deleted_at < now() - HARD_DELETE_AFTER_DAYS.
/// Models covered in Plane:
/// workspaces, projects, cycles, modules, issues, pages, issue_views, labels, states,
/// issue_activities, issue_comments, issue_links, issue_reactions, user_favorites,
/// module_issues, cycle_issues, estimates, estimate_points.
/// Plane default: 60 days (read HARD_DELETE_AFTER_DAYS env).
async fn run_hard_delete(tx: &mut sqlx::Transaction<'_, sqlx::Postgres>) -> anyhow::Result<()> {
    let days: i64 = std::env::var("HARD_DELETE_AFTER_DAYS")
        .ok()
        .and_then(|v| v.parse().ok())
        .filter(|&d| d >= 0)
        .unwrap_or(60);

    let cutoff = Utc::now() - chrono::Duration::days(days);

    // Dependent child tables deleted before parent tables to respect foreign keys where applicable
    const TABLES: &[&str] = &[
        "user_favorites",
        "issue_reactions",
        "comment_reactions",
        "issue_links",
        "issue_comments",
        "issue_activities",
        "issue_assignees",
        "issue_labels",
        "issue_subscribers",
        "module_issues",
        "cycle_issues",
        "intake_issues",
        "draft_issues",
        "issues",
        "issue_views",
        "labels",
        "states",
        "estimate_points",
        "estimates",
        "modules",
        "cycles",
        "pages",
        "project_members",
        "projects",
        "workspace_members",
        "workspaces",
    ];

    for &tbl in TABLES {
        let q = format!("DELETE FROM {tbl} WHERE deleted_at IS NOT NULL AND deleted_at < $1");
        let n = sqlx::query(&q).bind(cutoff).execute(&mut **tx).await?.rows_affected();
        if n > 0 {
            tracing::info!(table = tbl, count = n, "hard-deleted expired rows");
        }
    }

    Ok(())
}

/// 4. delete_unuploaded_file_asset
/// Deletes file_assets with is_uploaded = false and created_at < now() - UNUPLOADED_ASSET_DELETE_DAYS (Plane default: 7).
/// Also cleans up the disk file under STORAGE_DIR if present, safely preventing traversal.
async fn run_delete_unuploaded_assets(st: &AppState, tx: &mut sqlx::Transaction<'_, sqlx::Postgres>) -> anyhow::Result<()> {
    let days: i64 = std::env::var("UNUPLOADED_ASSET_DELETE_DAYS")
        .ok()
        .and_then(|v| v.parse().ok())
        .filter(|&d| d >= 0)
        .unwrap_or(7);

    let cutoff = Utc::now() - chrono::Duration::days(days);

    let assets: Vec<(Uuid, String)> = sqlx::query_as(
        "SELECT id, asset FROM file_assets
         WHERE is_uploaded = false AND created_at < $1"
    )
    .bind(cutoff)
    .fetch_all(&mut **tx)
    .await?;

    if !assets.is_empty() {
        let ids: Vec<Uuid> = assets.iter().map(|(id, _)| *id).collect();
        sqlx::query("DELETE FROM file_assets WHERE id = ANY($1)")
            .bind(&ids)
            .execute(&mut **tx)
            .await?;

        for (_, key) in assets {
            remove_stored_file(st, &key).await;
        }
    }

    Ok(())
}

/// Safely removes a file from STORAGE_DIR if it exists, without traversing out of storage_dir.
async fn remove_stored_file(st: &AppState, key: &str) {
    if key.contains("..") || key.starts_with('/') || key.contains('\\') {
        return;
    }
    let path = st.cfg.storage_dir.join(key);
    if let (Ok(real), Ok(root)) = (path.canonicalize(), st.cfg.storage_dir.canonicalize()) {
        if real.starts_with(&root) {
            let _ = tokio::fs::remove_file(&real).await;
        }
    }
}

/// 5a. delete_api_logs
/// Retention: API_ACTIVITY_LOG_RETENTION_DAYS (Plane default: 14).
async fn run_delete_api_logs(tx: &mut sqlx::Transaction<'_, sqlx::Postgres>) -> anyhow::Result<()> {
    let days: i64 = std::env::var("API_ACTIVITY_LOG_RETENTION_DAYS")
        .ok()
        .and_then(|v| v.parse().ok())
        .filter(|&d| d >= 0)
        .unwrap_or(14);
    let cutoff = Utc::now() - chrono::Duration::days(days);
    sqlx::query("DELETE FROM api_activity_logs WHERE created_at <= $1")
        .bind(cutoff)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

/// 5b. delete_email_notification_logs
/// Retention: EMAIL_LOG_RETENTION_DAYS (Plane default: 7).
async fn run_delete_email_logs(tx: &mut sqlx::Transaction<'_, sqlx::Postgres>) -> anyhow::Result<()> {
    let days: i64 = std::env::var("EMAIL_LOG_RETENTION_DAYS")
        .ok()
        .and_then(|v| v.parse().ok())
        .filter(|&d| d >= 0)
        .unwrap_or(7);
    let cutoff = Utc::now() - chrono::Duration::days(days);
    sqlx::query("DELETE FROM email_notification_logs WHERE sent_at <= $1")
        .bind(cutoff)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

/// 5c. delete_page_versions
/// Keeps latest 20 versions per page.
async fn run_delete_page_versions(tx: &mut sqlx::Transaction<'_, sqlx::Postgres>) -> anyhow::Result<()> {
    sqlx::query(
        "DELETE FROM page_versions
         WHERE id IN (
             SELECT id FROM (
                 SELECT id, ROW_NUMBER() OVER (PARTITION BY page_id ORDER BY created_at DESC) as rn
                 FROM page_versions
             ) sub WHERE sub.rn > 20
         )"
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// 5d. delete_issue_description_versions
/// Keeps latest 20 versions per issue.
async fn run_delete_issue_description_versions(tx: &mut sqlx::Transaction<'_, sqlx::Postgres>) -> anyhow::Result<()> {
    sqlx::query(
        "DELETE FROM issue_description_versions
         WHERE id IN (
             SELECT id FROM (
                 SELECT id, ROW_NUMBER() OVER (PARTITION BY issue_id ORDER BY created_at DESC) as rn
                 FROM issue_description_versions
             ) sub WHERE sub.rn > 20
         )"
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// 5e. delete_webhook_logs
/// Retention: WEBHOOK_LOG_RETENTION_DAYS (Plane default: 14).
async fn run_delete_webhook_logs(tx: &mut sqlx::Transaction<'_, sqlx::Postgres>) -> anyhow::Result<()> {
    let days: i64 = std::env::var("WEBHOOK_LOG_RETENTION_DAYS")
        .ok()
        .and_then(|v| v.parse().ok())
        .filter(|&d| d >= 0)
        .unwrap_or(14);
    let cutoff = Utc::now() - chrono::Duration::days(days);
    sqlx::query("DELETE FROM webhook_logs WHERE created_at <= $1")
        .bind(cutoff)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

/// 6. delete_old_s3_link
/// Plane's exporter_expired_task expires exporter files older than 8 days: deletes object and sets url=NULL.
/// Saturn stores exports under STORAGE_DIR/exports/<token>/<filename> with key = "exports/<token>/<filename>".
/// We delete the on-disk file and clear url (url=NULL).
async fn run_delete_old_s3_link(st: &AppState, tx: &mut sqlx::Transaction<'_, sqlx::Postgres>) -> anyhow::Result<()> {
    let cutoff = Utc::now() - chrono::Duration::days(8);

    let rows: Vec<(Uuid, Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT id, key, token FROM exporters
         WHERE url IS NOT NULL AND created_at <= $1"
    )
    .bind(cutoff)
    .fetch_all(&mut **tx)
    .await?;

    if !rows.is_empty() {
        let ids: Vec<Uuid> = rows.iter().map(|(id, _, _)| *id).collect();
        sqlx::query("UPDATE exporters SET url = NULL, updated_at = now() WHERE id = ANY($1)")
            .bind(&ids)
            .execute(&mut **tx)
            .await?;

        for (_, key, token) in rows {
            if let Some(k) = key {
                remove_stored_file(st, &k).await;
            }
            if let Some(tok) = token {
                // Also clean up directory if empty
                if !tok.contains("..") && !tok.starts_with('/') && !tok.contains('\\') {
                    let dir = st.cfg.storage_dir.join("exports").join(&tok);
                    let _ = tokio::fs::remove_dir_all(&dir).await;
                }
            }
        }
    }

    Ok(())
}

/// Entry point to spawn background tokio tasks for each scheduled job.
/// SATURN_JOBS=0 disables scheduled jobs.
pub fn spawn(st: AppState) {
    if std::env::var("SATURN_JOBS").unwrap_or_default() == "0" {
        tracing::info!("SATURN_JOBS=0; scheduled background jobs disabled");
        return;
    }

    for &job_name in JOB_NAMES {
        let st_clone = st.clone();
        tokio::spawn(async move {
            loop {
                let now = Utc::now();
                let sleep_dur = match duration_until_next_run(job_name, now) {
                    Some(d) => d,
                    None => break,
                };
                tokio::time::sleep(sleep_dur).await;

                if let Err(e) = run_job_locked(&st_clone, job_name).await {
                    tracing::error!(job = job_name, error = %e, "scheduled job failed");
                }
            }
        });
    }
}

/// CLI runner for `saturn jobs run <name>`.
pub async fn cli_run(st: &AppState, args: &[String]) -> anyhow::Result<()> {
    let job_name = args.get(0).map(String::as_str).unwrap_or("");
    if job_name.is_empty() {
        eprintln!("Usage: saturn jobs run <job_name>");
        eprintln!("Available jobs:");
        for name in JOB_NAMES {
            eprintln!("  {name}");
        }
        std::process::exit(1);
    }

    if !JOB_NAMES.contains(&job_name) {
        eprintln!("Unknown job: {job_name}");
        eprintln!("Available jobs:");
        for name in JOB_NAMES {
            eprintln!("  {name}");
        }
        std::process::exit(1);
    }

    let ran = run_job_locked(st, job_name).await?;
    if ran {
        println!("job {job_name} completed successfully");
    } else {
        println!("job {job_name} skipped (lock held)");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn test_duration_until_next_5min() {
        // At 12:00:00 -> exactly 300s to 12:05:00
        let t1 = Utc.with_ymd_and_hms(2026, 11, 13, 12, 0, 0).unwrap();
        assert_eq!(duration_until_next_5min(t1), Duration::from_secs(300));

        // At 12:02:30 -> 150s to 12:05:00
        let t2 = Utc.with_ymd_and_hms(2026, 11, 13, 12, 2, 30).unwrap();
        assert_eq!(duration_until_next_5min(t2), Duration::from_secs(150));

        // At 12:59:50 -> 10s to 13:00:00
        let t3 = Utc.with_ymd_and_hms(2026, 11, 13, 12, 59, 50).unwrap();
        assert_eq!(duration_until_next_5min(t3), Duration::from_secs(10));
    }

    #[test]
    fn test_duration_until_next_daily() {
        // Daily at 01:00 UTC
        // When now is 00:30 UTC -> 30 mins (1800s)
        let t1 = Utc.with_ymd_and_hms(2026, 11, 13, 0, 30, 0).unwrap();
        assert_eq!(duration_until_next_daily(t1, 1, 0), Duration::from_secs(1800));

        // When now is 01:00:00 UTC -> next day at 01:00 (86400s)
        let t2 = Utc.with_ymd_and_hms(2026, 11, 13, 1, 0, 0).unwrap();
        assert_eq!(duration_until_next_daily(t2, 1, 0), Duration::from_secs(86400));

        // When now is 02:00:00 UTC -> next day at 01:00 (23 * 3600 = 82800s)
        let t3 = Utc.with_ymd_and_hms(2026, 11, 13, 2, 0, 0).unwrap();
        assert_eq!(duration_until_next_daily(t3, 1, 0), Duration::from_secs(82800));
    }

    #[test]
    fn test_advisory_lock_deterministic() {
        let k1 = advisory_lock_key("hard_delete");
        let k2 = advisory_lock_key("hard_delete");
        let k3 = advisory_lock_key("archive_and_close_old_issues");
        assert_eq!(k1, k2);
        assert_ne!(k1, k3);
    }
}
