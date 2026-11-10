# Project list and details
call GET "/api/workspaces/$SLUG/projects/"
call GET "/api/workspaces/$SLUG/projects/details/"
call GET "/api/workspaces/$SLUG/project-identifiers/?name=GS"
call GET "/api/workspaces/$SLUG/projects/$PROJECT/"
call GET "/api/workspaces/$SLUG/projects/$PROJECT/members/"
call GET "/api/workspaces/$SLUG/projects/$PROJECT/project-members/me/"
call GET "/api/workspaces/$SLUG/user-favorite-projects/"
call POST "/api/workspaces/$SLUG/user-favorite-projects/" '{"project":"'"$PROJECT"'"}'
call DELETE "/api/workspaces/$SLUG/user-favorite-projects/$PROJECT/"
call POST "/api/workspaces/$SLUG/projects/" '{"name":"Smoke project '$RANDOM'","identifier":"S'$RANDOM'"}'
SMOKE_PROJECT=$(field "['id']")
call PATCH "/api/workspaces/$SLUG/projects/$SMOKE_PROJECT/" '{"description":"Smoke test"}'
call POST "/api/workspaces/$SLUG/projects/$SMOKE_PROJECT/archive/"
call DELETE "/api/workspaces/$SLUG/projects/$SMOKE_PROJECT/archive/"
call DELETE "/api/workspaces/$SLUG/projects/$SMOKE_PROJECT/"
