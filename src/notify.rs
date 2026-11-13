use crate::common::AppState;
use serde_json::Value;
use sqlx::PgConnection;
use uuid::Uuid;

pub type ActivitySnapshot = Value;

pub async fn snapshots(
    tx: &mut PgConnection,
    issue: Uuid,
    project: Uuid,
    workspace: Uuid,
) -> Result<Vec<Value>, sqlx::Error> {
    sqlx::query_scalar("SELECT to_jsonb(a) FROM issue_activities a WHERE issue_id=$1 AND project_id=$2 AND workspace_id=$3 AND a.deleted_at IS NULL AND a.xmin = pg_current_xact_id()::text::xid ORDER BY created_at,id")
        .bind(issue).bind(project).bind(workspace).fetch_all(tx).await
}

fn mentions(html: &str) -> Vec<Uuid> {
    // Ammonia normalizes attributes to quoted values before activities are recorded.
    html.split("<mention-component")
        .skip(1)
        .filter_map(|s| {
            let tag = s.split('>').next()?;
            let attr = |key: &str| {
                tag.split_whitespace().find_map(|part| {
                    part.strip_prefix(key)?
                        .strip_prefix('=')
                        .map(|v| v.trim_matches(['\"', '\'']))
                })
            };
            if attr("entity_name") != Some("user_mention") {
                return None;
            }
            Uuid::parse_str(attr("entity_identifier")?).ok()
        })
        .collect()
}

pub fn after_activity(
    st: &AppState,
    issue: Uuid,
    project: Uuid,
    workspace: Uuid,
    actor: Uuid,
    activities: Vec<ActivitySnapshot>,
) {
    let st = st.clone();
    tokio::spawn(async move {
        if let Err(e) = dispatch(&st, issue, project, workspace, actor, activities).await {
            tracing::error!(error=%e, "notification dispatch failed");
        }
    });
}

async fn dispatch(
    st: &AppState,
    issue: Uuid,
    project: Uuid,
    workspace: Uuid,
    actor: Uuid,
    activities: Vec<Value>,
) -> Result<(), sqlx::Error> {
    if activities.is_empty() {
        return Ok(());
    }
    let mut tx = st.db.begin().await?;
    let subscribe = activities
        .iter()
        .any(|a| a["field"] == "comment" && a["verb"] == "created" || a["field"].is_null() && a["verb"] == "created");
    if subscribe {
        sqlx::query("INSERT INTO issue_subscribers (issue_id,project_id,workspace_id,subscriber_id) SELECT $1,$2,$3,$4 WHERE NOT EXISTS (SELECT 1 FROM issue_subscribers WHERE issue_id=$1 AND project_id=$2 AND workspace_id=$3 AND subscriber_id=$4 AND deleted_at IS NULL) ON CONFLICT DO NOTHING")
            .bind(issue).bind(project).bind(workspace).bind(actor).execute(&mut *tx).await?;
    }
    for a in activities {
        let field = a["field"].as_str().unwrap_or("");
        let new_mentions = if field == "comment" || field == "description" {
            mentions(a["new_value"].as_str().unwrap_or(""))
        } else {
            vec![]
        };
        let old_mentions = mentions(a["old_value"].as_str().unwrap_or(""));
        let new_mentions: Vec<Uuid> = new_mentions
            .into_iter()
            .filter(|id| !old_mentions.contains(id) && *id != actor)
            .collect();
        sqlx::query("INSERT INTO issue_subscribers (issue_id,project_id,workspace_id,subscriber_id) SELECT $1,$2,$3,pm.member_id FROM project_members pm WHERE pm.project_id=$2 AND pm.workspace_id=$3 AND pm.is_active AND pm.deleted_at IS NULL AND pm.member_id=ANY($4) AND NOT EXISTS (SELECT 1 FROM issue_subscribers s WHERE s.issue_id=$1 AND s.project_id=$2 AND s.workspace_id=$3 AND s.subscriber_id=pm.member_id AND s.deleted_at IS NULL) ON CONFLICT DO NOTHING")
            .bind(issue).bind(project).bind(workspace).bind(&new_mentions).execute(&mut *tx).await?;
        let rows: Vec<(Uuid,String,Value,bool)> = sqlx::query_as("SELECT pm.member_id, CASE WHEN pm.member_id=ANY($5) THEN 'mentioned' WHEN i.created_by_id=pm.member_id THEN 'created' WHEN ia.assignee_id IS NOT NULL THEN 'assigned' ELSE 'subscribed' END, jsonb_build_object('issue',jsonb_build_object('id',i.id,'name',i.name,'identifier',p.identifier,'sequence_id',i.sequence_id,'state_name',s.name,'state_group',s.\"group\",'project_id',p.id,'workspace_slug',w.slug),'issue_activity',jsonb_build_object('id',$6::jsonb->>'id','verb',$6::jsonb->>'verb','field',$6::jsonb->>'field','actor',$6::jsonb->>'actor_id','new_value',$6::jsonb->>'new_value','old_value',$6::jsonb->>'old_value','old_identifier',$6::jsonb->>'old_identifier','new_identifier',$6::jsonb->>'new_identifier','issue_comment',COALESCE(c.comment_stripped,''),'activity_time',$6::jsonb->>'created_at')), CASE WHEN pm.member_id=ANY($5) THEN COALESCE(pref.mention,true) WHEN $7='state' THEN COALESCE(pref.state_change,true) OR (s.\"group\"='completed' AND COALESCE(pref.issue_completed,true)) OR COALESCE(pref.property_change,true) WHEN $7='comment' THEN COALESCE(pref.comment,true) OR COALESCE(pref.property_change,true) ELSE COALESCE(pref.property_change,true) END FROM issues i JOIN projects p ON p.id=i.project_id AND p.workspace_id=i.workspace_id JOIN workspaces w ON w.id=i.workspace_id LEFT JOIN states s ON s.id=i.state_id AND s.project_id=i.project_id AND s.workspace_id=i.workspace_id LEFT JOIN issue_comments c ON c.id=($6::jsonb->>'issue_comment_id')::uuid AND c.issue_id=i.id AND c.project_id=i.project_id AND c.workspace_id=i.workspace_id AND c.deleted_at IS NULL JOIN project_members pm ON pm.project_id=i.project_id AND pm.workspace_id=i.workspace_id AND pm.is_active AND pm.deleted_at IS NULL LEFT JOIN issue_assignees ia ON ia.issue_id=i.id AND ia.project_id=i.project_id AND ia.workspace_id=i.workspace_id AND ia.assignee_id=pm.member_id AND ia.deleted_at IS NULL LEFT JOIN user_notification_preferences pref ON pref.user_id=pm.member_id AND pref.deleted_at IS NULL AND pref.project_id IS NULL AND pref.workspace_id IS NULL WHERE i.id=$1 AND i.project_id=$2 AND i.workspace_id=$3 AND i.deleted_at IS NULL AND pm.member_id<>$4 AND (pm.member_id=ANY($5) OR ($7<>'description' AND (ia.assignee_id IS NOT NULL OR EXISTS (SELECT 1 FROM issue_subscribers sub WHERE sub.issue_id=i.id AND sub.project_id=i.project_id AND sub.workspace_id=i.workspace_id AND sub.subscriber_id=pm.member_id AND sub.deleted_at IS NULL))))")
            .bind(issue).bind(project).bind(workspace).bind(actor).bind(&new_mentions).bind(&a).bind(field).fetch_all(&mut *tx).await?;
        for (receiver, kind, data, email) in rows {
            let message = if kind == "mentioned" {
                Some(format!(
                    "You have been mentioned in the issue {}",
                    data["issue"]["name"].as_str().unwrap_or("")
                ))
            } else {
                None
            };
            sqlx::query("INSERT INTO notifications (workspace_id,project_id,receiver_id,triggered_by_id,entity_identifier,entity_name,sender,title,message,data) VALUES ($1,$2,$3,$4,$5,'issue',$6,$7,$8,$9)")
                .bind(workspace).bind(project).bind(receiver).bind(actor).bind(issue).bind(format!("in_app:issue_activities:{kind}")).bind(if kind == "mentioned" { "" } else { a["comment"].as_str().unwrap_or("") }).bind(message.map(Value::String)).bind(&data).execute(&mut *tx).await?;
            if email {
                sqlx::query("INSERT INTO email_notification_logs (receiver_id,triggered_by_id,entity_identifier,entity_name,entity,data) VALUES ($1,$2,$3,'issue','',$4)").bind(receiver).bind(actor).bind(issue).bind(&data).execute(&mut *tx).await?;
            }
        }
    }
    tx.commit().await?;
    Ok(())
}

pub fn start(st: &AppState) {
    if std::env::var("SATURN_JOBS").ok().as_deref() == Some("0") {
        return;
    }
    let st = st.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(300));
        interval.tick().await;
        loop {
            interval.tick().await;
            if let Err(e) = stack_email_notification(&st).await {
                tracing::error!(error=%e, "notification digest failed");
            }
        }
    });
}

pub async fn stack_email_notification(st: &AppState) -> Result<(), sqlx::Error> {
    let mut tx = st.db.begin().await?;
    let locked: bool = sqlx::query_scalar("SELECT pg_try_advisory_xact_lock($1)")
        .bind(0x5341544e_i64)
        .fetch_one(&mut *tx)
        .await?;
    if !locked {
        return Ok(());
    }
    let groups: Vec<(Uuid, Uuid, String, Vec<Uuid>, Vec<Value>)> = sqlx::query_as("SELECT l.receiver_id,l.entity_identifier,u.email,array_agg(l.id ORDER BY l.created_at),array_agg(l.data ORDER BY l.created_at) FROM email_notification_logs l JOIN users u ON u.id=l.receiver_id WHERE l.processed_at IS NULL AND l.deleted_at IS NULL AND l.entity_name='issue' AND l.entity_identifier IS NOT NULL GROUP BY l.receiver_id,l.entity_identifier,u.email").fetch_all(&mut *tx).await?;
    let smtp = crate::outbound::smtp(st).await;
    for (_, _, email, ids, data) in groups {
        if let Some(smtp) = &smtp {
            let issue = data
                .first()
                .and_then(|v| v["issue"]["name"].as_str())
                .unwrap_or("Issue");
            let text = format!(
                "Updates to {issue}\n{}",
                data.iter()
                    .map(|v| format!(
                        "{}: {}",
                        v["issue_activity"]["field"].as_str().unwrap_or(""),
                        v["issue_activity"]["new_value"].as_str().unwrap_or("")
                    ))
                    .collect::<Vec<_>>()
                    .join("\n")
            );
            if let Err(e) = crate::outbound::send_mail(
                smtp,
                &email,
                &format!("Updates to {issue}"),
                &ammonia::clean(&text),
                &text,
            )
            .await
            {
                tracing::warn!(error=%e, "notification digest send failed");
                continue;
            }
            sqlx::query("UPDATE email_notification_logs SET processed_at=now(),sent_at=now(),updated_at=now() WHERE id=ANY($1) AND processed_at IS NULL").bind(&ids).execute(&mut *tx).await?;
        } else {
            tracing::info!("SMTP not configured, discarding notification digest");
            sqlx::query("UPDATE email_notification_logs SET processed_at=now(),updated_at=now() WHERE id=ANY($1) AND processed_at IS NULL").bind(&ids).execute(&mut *tx).await?;
        }
    }
    tx.commit().await
}

#[cfg(test)]
mod tests {
    #[test]
    fn parse_mentions() {
        let id = "00000000-0000-0000-0000-000000000001";
        assert_eq!(
            super::mentions(&format!(
                "<mention-component entity_identifier=\"{id}\" entity_name=\"user_mention\"></mention-component>"
            ))
            .len(),
            1
        );
        assert!(super::mentions("<mention-component entity_name=\"other\">").is_empty());
    }
}
