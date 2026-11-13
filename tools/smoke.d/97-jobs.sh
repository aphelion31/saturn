# Scheduled jobs smoke test (src/jobs.rs)
# Tests manual CLI execution: saturn jobs run <name>
SATURN_BIN="${SATURN_BIN:-./target/debug/saturn}"
DB_CONN="${DATABASE_URL:-postgres://localhost/saturn}"

query() {
  psql "$DB_CONN" -Atq -c "$1" | head -n 1
}

WS_ID=$(query "SELECT id FROM workspaces WHERE slug='$SLUG'")
USER_ID=$(query "SELECT id FROM users WHERE email='$EMAIL'")

echo "== [jobs] test CLI run without arguments exits with code 1"
if "$SATURN_BIN" jobs run >/dev/null 2>&1; then
  echo "✖ expected 'saturn jobs run' without args to fail" >&2
  exit 1
fi
echo "✔ saturn jobs run usage check passed"

# -----------------------------------------------------------------------------
# 1. unsnooze_expired_intake_issues
# -----------------------------------------------------------------------------
echo "== [jobs] fixture: expired snoozed intake issue"
INTAKE_ID=$(query "SELECT id FROM intakes WHERE workspace_id='$WS_ID' AND project_id='$PROJECT' LIMIT 1")
if [ -z "$INTAKE_ID" ]; then
  INTAKE_ID=$(query "INSERT INTO intakes (workspace_id, project_id, name, description, is_default, view_props, created_by_id) VALUES ('$WS_ID', '$PROJECT', 'Smoke Intake', '', true, '{}'::jsonb, '$USER_ID') RETURNING id")
fi

SNOOZED_ISSUE_ID=$(query "INSERT INTO issues (project_id, workspace_id, name, created_by_id, updated_by_id, state_id) VALUES ('$PROJECT', '$WS_ID', 'Snoozed Issue', '$USER_ID', '$USER_ID', (SELECT id FROM states WHERE project_id='$PROJECT' LIMIT 1)) RETURNING id")
INTAKE_ISSUE_ID=$(query "INSERT INTO intake_issues (workspace_id, project_id, intake_id, issue_id, status, snoozed_till, created_by_id, extra) VALUES ('$WS_ID', '$PROJECT', '$INTAKE_ID', '$SNOOZED_ISSUE_ID', 0, now() - interval '1 hour', '$USER_ID', '{}'::jsonb) RETURNING id")

"$SATURN_BIN" jobs run unsnooze_expired_intake_issues

STATUS=$(query "SELECT status FROM intake_issues WHERE id='$INTAKE_ISSUE_ID'")
SNOOZED_TILL=$(query "SELECT snoozed_till FROM intake_issues WHERE id='$INTAKE_ISSUE_ID'")
if [ "$STATUS" != "-2" ] || [ -n "$SNOOZED_TILL" ]; then
  echo "✖ intake issue was not unsnoozed: status=$STATUS, snoozed_till=$SNOOZED_TILL" >&2
  exit 1
fi
echo "✔ unsnooze_expired_intake_issues succeeded (returned to pending)"

# -----------------------------------------------------------------------------
# 2. archive_and_close_old_issues
# -----------------------------------------------------------------------------
echo "== [jobs] fixture: old completed issue in project with archive_in=1"
query "UPDATE projects SET archive_in=1, close_in=1 WHERE id='$PROJECT'" >/dev/null

COMPLETED_STATE_ID=$(query "SELECT id FROM states WHERE project_id='$PROJECT' AND \"group\"='completed' LIMIT 1")
OLD_ISSUE_ID=$(query "INSERT INTO issues (project_id, workspace_id, name, created_by_id, updated_by_id, state_id, updated_at) VALUES ('$PROJECT', '$WS_ID', 'Old Completed Issue', '$USER_ID', '$USER_ID', '$COMPLETED_STATE_ID', now() - interval '60 days') RETURNING id")

"$SATURN_BIN" jobs run archive_and_close_old_issues

ARCHIVED_AT=$(query "SELECT archived_at FROM issues WHERE id='$OLD_ISSUE_ID'")
if [ -z "$ARCHIVED_AT" ]; then
  echo "✖ old issue was not archived" >&2
  exit 1
fi
ACTIVITY_CNT=$(query "SELECT count(*) FROM issue_activities WHERE issue_id='$OLD_ISSUE_ID' AND field='archived_at' AND new_value='archive'")
if [ "$ACTIVITY_CNT" -lt 1 ]; then
  echo "✖ expected issue activity for archive not found" >&2
  exit 1
fi
echo "✔ archive_and_close_old_issues archived old completed issue and logged activity"

# -----------------------------------------------------------------------------
# 3. hard_delete
# -----------------------------------------------------------------------------
echo "== [jobs] fixture: old soft-deleted issue with comment + old soft-deleted project"
DEL_ISSUE_ID=$(query "INSERT INTO issues (project_id, workspace_id, name, created_by_id, updated_by_id, state_id, deleted_at) VALUES ('$PROJECT', '$WS_ID', 'Old Deleted Issue', '$USER_ID', '$USER_ID', '$COMPLETED_STATE_ID', now() - interval '90 days') RETURNING id")
DEL_COMMENT_ID=$(query "INSERT INTO issue_comments (issue_id, project_id, workspace_id, actor_id, comment_json, comment_html, created_by_id, updated_by_id) VALUES ('$DEL_ISSUE_ID', '$PROJECT', '$WS_ID', '$USER_ID', '{}'::jsonb, '<p>bye</p>', '$USER_ID', '$USER_ID') RETURNING id")

call POST "/api/workspaces/$SLUG/projects/" '{"name":"HardDeleteProj","identifier":"HDP"}'
PROJECT2=$(field '.id')
call POST "/api/workspaces/$SLUG/projects/$PROJECT2/issue-labels/" '{"name":"HDLabel","color":"#ff0000"}'
LBL2=$(field '.id')
call POST "/api/workspaces/$SLUG/projects/$PROJECT2/cycles/" '{"name":"HDCycle"}'
CYC2=$(field '.id')
call POST "/api/workspaces/$SLUG/projects/$PROJECT2/issues/" "{\"name\":\"HDIssue\",\"label_ids\":[\"$LBL2\"]}"
ISS2=$(field '.id')
call POST "/api/workspaces/$SLUG/projects/$PROJECT2/cycles/$CYC2/cycle-issues/" "{\"issues\":[\"$ISS2\"]}"

query "UPDATE projects SET deleted_at = now() - interval '90 days' WHERE id='$PROJECT2'" >/dev/null

"$SATURN_BIN" jobs run hard_delete

for tbl_check in \
  "SELECT count(*) FROM projects WHERE id='$PROJECT2'" \
  "SELECT count(*) FROM issues WHERE project_id='$PROJECT2'" \
  "SELECT count(*) FROM states WHERE project_id='$PROJECT2'" \
  "SELECT count(*) FROM project_identifiers WHERE project_id='$PROJECT2'" \
  "SELECT count(*) FROM project_members WHERE project_id='$PROJECT2'" \
  "SELECT count(*) FROM labels WHERE project_id='$PROJECT2'" \
  "SELECT count(*) FROM cycles WHERE project_id='$PROJECT2'" \
  "SELECT count(*) FROM issues WHERE id='$DEL_ISSUE_ID'" \
  "SELECT count(*) FROM issue_comments WHERE id='$DEL_COMMENT_ID'"
do
  CNT=$(query "$tbl_check")
  if [ "$CNT" != "0" ]; then
    echo "✖ hard_delete left rows for: $tbl_check (count=$CNT)" >&2
    exit 1
  fi
done

SEEDED_CNT=$(query "SELECT count(*) FROM projects WHERE id='$PROJECT'")
if [ "$SEEDED_CNT" != "1" ]; then
  echo "✖ seeded first project was affected: count=$SEEDED_CNT" >&2
  exit 1
fi
echo "✔ hard_delete cascaded project, issues, states, identifiers, members, and issue comments"

# -----------------------------------------------------------------------------
# 4. delete_unuploaded_file_asset
# -----------------------------------------------------------------------------
echo "== [jobs] fixture: old unuploaded file asset"
UNUPLOADED_ASSET_ID=$(query "INSERT INTO file_assets (attributes, asset, is_deleted, is_archived, is_uploaded, size, workspace_id, project_id, created_at, updated_at) VALUES ('{}'::jsonb, 'dummy/asset.txt', false, false, false, 0, '$WS_ID', '$PROJECT', now() - interval '10 days', now() - interval '10 days') RETURNING id")

"$SATURN_BIN" jobs run delete_unuploaded_file_asset

EXISTS_ASSET=$(query "SELECT count(*) FROM file_assets WHERE id='$UNUPLOADED_ASSET_ID'")
if [ "$EXISTS_ASSET" != "0" ]; then
  echo "✖ unuploaded asset was not deleted" >&2
  exit 1
fi
echo "✔ delete_unuploaded_file_asset deleted unuploaded file asset"

# -----------------------------------------------------------------------------
# 5. cleanup_task logs and versions
# -----------------------------------------------------------------------------
echo "== [jobs] cleanup logs and versions"
query "INSERT INTO api_activity_logs (token_identifier, path, method, response_code, created_at, updated_at) VALUES ('tok', '/test', 'GET', 200, now() - interval '30 days', now() - interval '30 days')" >/dev/null
"$SATURN_BIN" jobs run delete_api_logs
OLD_API_LOGS=$(query "SELECT count(*) FROM api_activity_logs WHERE created_at < now() - interval '20 days'")
if [ "$OLD_API_LOGS" != "0" ]; then
  echo "✖ old api logs not deleted" >&2
  exit 1
fi

WH_ID=$(query "SELECT id FROM webhooks WHERE workspace_id='$WS_ID' LIMIT 1")
if [ -z "$WH_ID" ]; then
  WH_ID=$(query "INSERT INTO webhooks (workspace_id, url) VALUES ('$WS_ID', 'http://example.com/wh') RETURNING id")
fi
query "INSERT INTO webhook_logs (webhook, workspace_id, retry_count, created_at, updated_at) VALUES ('$WH_ID', '$WS_ID', 0, now() - interval '30 days', now() - interval '30 days')" >/dev/null
"$SATURN_BIN" jobs run delete_webhook_logs
OLD_WH_LOGS=$(query "SELECT count(*) FROM webhook_logs WHERE created_at < now() - interval '20 days'")
if [ "$OLD_WH_LOGS" != "0" ]; then
  echo "✖ old webhook logs not deleted" >&2
  exit 1
fi

query "INSERT INTO email_notification_logs (entity_name, entity, receiver_id, triggered_by_id, sent_at, created_at, updated_at) VALUES ('Issue', 'issue', '$USER_ID', '$USER_ID', now() - interval '20 days', now() - interval '20 days', now() - interval '20 days')" >/dev/null
"$SATURN_BIN" jobs run delete_email_notification_logs
OLD_EMAIL_LOGS=$(query "SELECT count(*) FROM email_notification_logs WHERE sent_at < now() - interval '15 days'")
if [ "$OLD_EMAIL_LOGS" != "0" ]; then
  echo "✖ old email logs not deleted" >&2
  exit 1
fi

# page versions cleanup (>20 versions)
PAGE_ID=$(query "SELECT p.id FROM pages p JOIN project_pages pp ON pp.page_id=p.id WHERE p.workspace_id='$WS_ID' AND pp.project_id='$PROJECT' LIMIT 1")
if [ -z "$PAGE_ID" ]; then
  PAGE_ID=$(query "INSERT INTO pages (name, description_json, description_html, owned_by_id, workspace_id, color, access, is_locked, is_global, view_props, logo_props) VALUES ('Smoke Page', '{}'::jsonb, '<p></p>', '$USER_ID', '$WS_ID', '', 0, false, false, '{}'::jsonb, '{}'::jsonb) RETURNING id")
  query "INSERT INTO project_pages (workspace_id, project_id, page_id, created_by_id, updated_by_id) VALUES ('$WS_ID', '$PROJECT', '$PAGE_ID', '$USER_ID', '$USER_ID')" >/dev/null
fi
for i in $(seq 1 25); do
  query "INSERT INTO page_versions (page_id, workspace_id, owned_by_id, description_json, description_html, sub_pages_data, last_saved_at, created_at, updated_at) VALUES ('$PAGE_ID', '$WS_ID', '$USER_ID', '{}'::jsonb, '<p>$i</p>', '{}'::jsonb, now() - interval '$i hours', now() - interval '$i hours', now() - interval '$i hours')" >/dev/null
done
"$SATURN_BIN" jobs run delete_page_versions
PAGE_VERS_CNT=$(query "SELECT count(*) FROM page_versions WHERE page_id='$PAGE_ID'")
if [ "$PAGE_VERS_CNT" -gt 20 ]; then
  echo "✖ expected page versions to be <= 20, got $PAGE_VERS_CNT" >&2
  exit 1
fi

# issue description versions cleanup (>20 versions)
for i in $(seq 1 25); do
  query "INSERT INTO issue_description_versions (issue_id, project_id, workspace_id, owned_by_id, description_json, description_html, last_saved_at, created_at, updated_at) VALUES ('$OLD_ISSUE_ID', '$PROJECT', '$WS_ID', '$USER_ID', '{}'::jsonb, '<p>$i</p>', now() - interval '$i hours', now() - interval '$i hours', now() - interval '$i hours')" >/dev/null
done
"$SATURN_BIN" jobs run delete_issue_description_versions
ISSUE_DESC_VERS_CNT=$(query "SELECT count(*) FROM issue_description_versions WHERE issue_id='$OLD_ISSUE_ID'")
if [ "$ISSUE_DESC_VERS_CNT" -gt 20 ]; then
  echo "✖ expected issue description versions to be <= 20, got $ISSUE_DESC_VERS_CNT" >&2
  exit 1
fi

echo "✔ cleanup logs and versions jobs succeeded"

# -----------------------------------------------------------------------------
# 6. delete_old_s3_link (exports)
# -----------------------------------------------------------------------------
echo "== [jobs] fixture: old expired exporter"
EXPORTER_ID=$(query "INSERT INTO exporters (workspace_id, project, provider, status, url, token, key, initiated_by_id, type, created_at, updated_at) VALUES ('$WS_ID', ARRAY['$PROJECT']::uuid[], 'csv', 'completed', 'http://example.com/download', 'tok123', 'exports/tok123/export.csv', '$USER_ID', 'issue_exports', now() - interval '10 days', now() - interval '10 days') RETURNING id")

"$SATURN_BIN" jobs run delete_old_s3_link

EXPORTER_URL=$(query "SELECT url FROM exporters WHERE id='$EXPORTER_ID'")
if [ -n "$EXPORTER_URL" ]; then
  echo "✖ exporter url was not cleared: $EXPORTER_URL" >&2
  exit 1
fi
echo "✔ delete_old_s3_link cleared expired exporter url"
