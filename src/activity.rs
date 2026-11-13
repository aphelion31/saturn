use sqlx::{Executor, Postgres};
use uuid::Uuid;

/// A Plane issue activity. Optional values are left null unless the event supplies them.
pub struct ActivityEvent<'a> {
    pub issue_id: Uuid,
    pub project_id: Uuid,
    pub workspace_id: Uuid,
    pub actor_id: Uuid,
    pub verb: &'a str,
    pub field: Option<&'a str>,
    pub old_value: Option<&'a str>,
    pub new_value: Option<&'a str>,
    pub comment: &'a str,
    pub old_identifier: Option<Uuid>,
    pub new_identifier: Option<Uuid>,
    pub issue_comment_id: Option<Uuid>,
    pub epoch: Option<f64>,
}

impl<'a> ActivityEvent<'a> {
    pub fn new(
        issue_id: Uuid,
        project_id: Uuid,
        workspace_id: Uuid,
        actor_id: Uuid,
        verb: &'a str,
        comment: &'a str,
    ) -> Self {
        Self {
            issue_id,
            project_id,
            workspace_id,
            actor_id,
            verb,
            field: None,
            old_value: None,
            new_value: None,
            comment,
            old_identifier: None,
            new_identifier: None,
            issue_comment_id: None,
            epoch: None,
        }
    }
}

/// Use the mutation's transaction when one exists, so its activity commits atomically.
pub async fn record<'e, E>(executor: E, event: ActivityEvent<'_>) -> Result<(), sqlx::Error>
where
    E: Executor<'e, Database = Postgres>,
{
    sqlx::query(
        "INSERT INTO issue_activities \
         (issue_id, project_id, workspace_id, actor_id, verb, field, old_value, new_value, \
          comment, old_identifier, new_identifier, issue_comment_id, attachments, epoch) \
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,ARRAY[]::varchar[], \
                 COALESCE($13, floor(extract(epoch from now()))::double precision))",
    )
    .bind(event.issue_id)
    .bind(event.project_id)
    .bind(event.workspace_id)
    .bind(event.actor_id)
    .bind(event.verb)
    .bind(event.field)
    .bind(event.old_value)
    .bind(event.new_value)
    .bind(event.comment)
    .bind(event.old_identifier)
    .bind(event.new_identifier)
    .bind(event.issue_comment_id)
    .bind(event.epoch)
    .execute(executor)
    .await?;
    Ok(())
}

pub async fn grouping(
    conn: &mut sqlx::PgConnection,
    issue: Uuid,
    project: Uuid,
    workspace: Uuid,
    actor: Uuid,
    kind: &str,
    id: Uuid,
    name: &str,
    added: bool,
) -> Result<(), sqlx::Error> {
    let comment = grouping_comment(kind, name, added);
    let mut event = ActivityEvent::new(
        issue,
        project,
        workspace,
        actor,
        if added { "created" } else { "deleted" },
        &comment,
    );
    event.field = Some(kind);
    event.old_value = Some(if added { "" } else { name });
    event.new_value = Some(if added { name } else { "" });
    if added {
        event.new_identifier = Some(id);
    } else {
        event.old_identifier = Some(id);
    }
    record(conn, event).await
}

fn grouping_comment(kind: &str, name: &str, added: bool) -> String {
    if added {
        format!("added {} {name}", if kind == "cycles" { "cycle" } else { "module" })
    } else {
        format!("removed this issue from {name}")
    }
}

#[cfg(test)]
mod tests {
    use super::grouping_comment;

    #[test]
    fn plane_grouping_comments() {
        assert_eq!(grouping_comment("cycles", "Sprint", true), "added cycle Sprint");
        assert_eq!(grouping_comment("modules", "API", true), "added module API");
        assert_eq!(
            grouping_comment("cycles", "Sprint", false),
            "removed this issue from Sprint"
        );
    }
}
