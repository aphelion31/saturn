# Workspaces (src/workspaces.rs)
call GET /api/users/me/workspaces/
call GET "/api/workspace-slug-check/?slug=smoke-$RANDOM"
call GET "/api/workspaces/$SLUG/"
call PATCH "/api/workspaces/$SLUG/" '{"name":"Acme"}'
call GET "/api/workspaces/$SLUG/members/"
call GET "/api/workspaces/$SLUG/workspace-members/me/"
call GET "/api/workspaces/$SLUG/project-members/"
call GET "/api/users/me/workspaces/$SLUG/project-roles/"
call GET "/api/workspaces/$SLUG/user-properties/"
call PATCH "/api/workspaces/$SLUG/user-properties/" '{"navigation_project_limit":10}'
call GET /api/users/last-visited-workspace/
call GET "/api/workspaces/$SLUG/invitations/"
call POST "/api/workspaces/$SLUG/invitations/" '{"emails":[{"email":"smoke-'$RANDOM'@example.com","role":5}]}'
call GET "/api/workspaces/$SLUG/user-favorites/"
call POST "/api/workspaces/$SLUG/user-favorites/" '{"entity_type":"folder","name":"Smoke","is_folder":true}'
UID_SELF=$(psql "${DATABASE_URL:-postgres://localhost/saturn}" -Atc "select u.id from users u where u.email='$EMAIL'" 2>/dev/null || true)
[[ -n "$UID_SELF" ]] && call GET "/api/workspaces/$SLUG/user-stats/$UID_SELF/" && call GET "/api/workspaces/$SLUG/user-profile/$UID_SELF/" && call GET "/api/workspaces/$SLUG/user-activity/$UID_SELF/"
