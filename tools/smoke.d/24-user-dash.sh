# User dashboard (src/user_dash.rs)
call GET "/api/users/me/activities/"
call GET "/api/users/me/workspaces/$SLUG/activity-graph/"
call GET "/api/users/me/workspaces/$SLUG/issues-completed-graph/?month=1"
call GET "/api/users/me/workspaces/$SLUG/dashboard/"
[[ "$(field '.results | type')" == "array" ]]
ME_ID=$(psql "${DATABASE_URL:-postgres://localhost/saturn}" -Atc "select id from users where email='$EMAIL'")
call GET "/api/workspaces/$SLUG/user-issues/$ME_ID/"
[[ "$(field '.results | type')" == "object" ]] || [[ "$(field '.results | type')" == "array" ]]
