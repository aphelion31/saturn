# User dashboard (src/user_dash.rs)
call GET "/api/users/me/activities/"
call GET "/api/users/me/workspaces/$SLUG/activity-graph/"
call GET "/api/users/me/workspaces/$SLUG/issues-completed-graph/?month=1"
call GET "/api/users/me/workspaces/$SLUG/dashboard/"
[[ "$(field '.results | type')" == "array" ]]
call GET "/api/workspaces/$SLUG/user-issues//"
