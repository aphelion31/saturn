# Workspace settings: webhooks, API tokens, workspace themes (src/workspaces.rs, src/users.rs)
call POST "/api/workspaces/$SLUG/webhooks/" '{"url":"https://example.com/smoke-'$RANDOM'","issue":true}'
WH=$(field "['id']")
call GET "/api/workspaces/$SLUG/webhooks/"
call GET "/api/workspaces/$SLUG/webhooks/$WH/"
call PATCH "/api/workspaces/$SLUG/webhooks/$WH/" '{"cycle":true}'
call POST "/api/workspaces/$SLUG/webhooks/$WH/regenerate/"
call GET "/api/workspaces/$SLUG/webhook-logs/$WH/"
call DELETE "/api/workspaces/$SLUG/webhooks/$WH/"
call POST /api/users/api-tokens/ '{"label":"smoke"}'
TOKEN_PK=$(field "['id']")
call GET /api/users/api-tokens/
call GET "/api/users/api-tokens/$TOKEN_PK/"
call DELETE "/api/users/api-tokens/$TOKEN_PK/"
call POST "/api/workspaces/$SLUG/workspace-themes/" '{"name":"Smoke '$RANDOM'","colors":{}}'
TH=$(field "['id']")
call GET "/api/workspaces/$SLUG/workspace-themes/"
call PATCH "/api/workspaces/$SLUG/workspace-themes/$TH/" '{"name":"Smoke renamed"}'
call DELETE "/api/workspaces/$SLUG/workspace-themes/$TH/"
