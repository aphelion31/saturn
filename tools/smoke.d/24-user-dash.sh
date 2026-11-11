# User dashboard (src/user_dash.rs)
call GET "/api/users/me/activities/"
jq -e '.results | type == "array"' <<<"$body" >/dev/null || { echo "✖ activities.results is not an array" >&2; exit 1; }
call GET "/api/users/me/workspaces/$SLUG/activity-graph/"
call GET "/api/users/me/workspaces/$SLUG/issues-completed-graph/?month=1"
call GET "/api/users/me/workspaces/$SLUG/dashboard/"
jq -e 'has("assigned_issues_count") and (.state_distribution | type == "array")' <<<"$body" >/dev/null || { echo "✖ dashboard shape" >&2; exit 1; }
ME_ID=$(psql "${DATABASE_URL:-postgres://localhost/saturn}" -Atc "select id from users where email='$EMAIL'")
call GET "/api/workspaces/$SLUG/user-issues/$ME_ID/"
jq -e '.results | type == "object" or type == "array"' <<<"$body" >/dev/null || { echo "✖ user-issues shape" >&2; exit 1; }
SMOKE_PROJECT=$(psql "${DATABASE_URL:-postgres://localhost/saturn}" -Atc "select p.id from projects p join workspaces w on w.id=p.workspace_id where w.slug='$SLUG' and p.deleted_at is null order by p.created_at limit 1")
call GET "/api/workspaces/$SLUG/entity-search/?project_id=$SMOKE_PROJECT&query_type=user_mention,issue,project,cycle,module,page&query=a"
