# Workspaces (src/workspaces.rs)
call GET /api/users/me/workspaces/
call GET "/api/workspace-slug-check/?slug=smoke-$RANDOM"
call GET "/api/workspaces/$SLUG/"
call PATCH "/api/workspaces/$SLUG/" '{"name":"Acme"}'
