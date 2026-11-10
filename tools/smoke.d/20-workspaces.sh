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
