# User dashboard (src/user_dash.rs)
call GET "/api/users/me/activities/"
call GET "/api/users/me/workspaces/$SLUG/activity-graph/"
call GET "/api/users/me/workspaces/$SLUG/issues-completed-graph/?month=1"
call GET "/api/users/me/workspaces/$SLUG/dashboard/"
[[ "$(field '.results | type')" == "array" ]]
ME_ID=$(psql "${DATABASE_URL:-postgres://localhost/saturn}" -Atc "select id from users where email='$EMAIL'")
call GET "/api/workspaces/$SLUG/user-issues/$ME_ID/"
[[ "$(field '.results | type')" == "object" ]] || [[ "$(field '.results | type')" == "array" ]]
SMOKE_PROJECT=$(psql "${DATABASE_URL:-postgres://localhost/saturn}" -Atc "select p.id from projects p join workspaces w on w.id=p.workspace_id where w.slug='$SLUG' and p.deleted_at is null order by p.created_at limit 1")
call GET "/api/workspaces/$SLUG/entity-search/?project_id=$SMOKE_PROJECT&query_type=user_mention,issue,project,cycle,module,page&query=a"
