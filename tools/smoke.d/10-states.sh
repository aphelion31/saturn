# Project states (src/states.rs)
call GET "/api/workspaces/$SLUG/projects/$PROJECT/states/"
call POST "/api/workspaces/$SLUG/projects/$PROJECT/states/" '{"name":"Smoke '$RANDOM'","color":"#123456","group":"started"}'
STATE=$(field '.id')
call PATCH "/api/workspaces/$SLUG/projects/$PROJECT/states/$STATE/" '{"color":"#654321"}'
call DELETE "/api/workspaces/$SLUG/projects/$PROJECT/states/$STATE/"
call GET "/api/workspaces/$SLUG/projects/$PROJECT/states/"
[[ "$body" != *"$STATE"* ]]
