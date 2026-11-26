# Extra views and project endpoints (src/views.rs, src/projects.rs)

# 1. Global / workspace views via /workspace-views/
call POST "/api/workspaces/$SLUG/workspace-views/" '{"name":"Extra ws view '$RANDOM'","filters":{"priority":["high"]}}'
EXTRA_WSVIEW=$(field '.id')
call GET "/api/workspaces/$SLUG/workspace-views/"
call GET "/api/workspaces/$SLUG/workspace-views/$EXTRA_WSVIEW/"
call PATCH "/api/workspaces/$SLUG/workspace-views/$EXTRA_WSVIEW/" '{"name":"Renamed extra ws view"}'
call GET "/api/workspaces/$SLUG/issues/?view_id=$EXTRA_WSVIEW&per_page=5"
call POST "/api/workspaces/$SLUG/workspace-views/" '{"view_props":{"collapsed":true}}'
call DELETE "/api/workspaces/$SLUG/workspace-views/$EXTRA_WSVIEW/"

# 2. Project user views / project-views
call GET "/api/workspaces/$SLUG/projects/$PROJECT/project-views/"
call POST "/api/workspaces/$SLUG/projects/$PROJECT/project-views/" '{"view_props":{"sort":"asc"}}'

# 3. Project invitations CRUD
call POST "/api/workspaces/$SLUG/projects/$PROJECT/invitations/" '{"emails":[{"email":"smoke-extra-'$RANDOM'@example.com","role":5}]}'
call GET "/api/workspaces/$SLUG/projects/$PROJECT/invitations/"
EXTRA_INVITE=$(field '.[0].id')
call GET "/api/workspaces/$SLUG/projects/$PROJECT/invitations/$EXTRA_INVITE/"
call DELETE "/api/workspaces/$SLUG/projects/$PROJECT/invitations/$EXTRA_INVITE/"

# 4. Project estimates
call GET "/api/workspaces/$SLUG/projects/$PROJECT/project-estimates/"
call POST "/api/workspaces/$SLUG/projects/$PROJECT/estimates/" '{"estimate":{"name":"SmokeEst '$RANDOM'","type":"categories"},"estimate_points":[{"key":1,"value":"1"}]}'
EXTRA_EST_ID=$(field '.id')
call PATCH "/api/workspaces/$SLUG/projects/$PROJECT/" '{"estimate":"'$EXTRA_EST_ID'"}'
call GET "/api/workspaces/$SLUG/projects/$PROJECT/project-estimates/"

# 5. Project member preferences
call GET /api/users/me/
ME_ID=$(field '.id')
call GET "/api/workspaces/$SLUG/projects/$PROJECT/preferences/member/$ME_ID/"
call PATCH "/api/workspaces/$SLUG/projects/$PROJECT/preferences/member/$ME_ID/" '{"pinned":["smoke"]}'

# 6. Bulk create labels & workspace labels
call POST "/api/workspaces/$SLUG/projects/$PROJECT/bulk-create-labels/" '{"label_data":[{"name":"BulkLabel-'$RANDOM'","description":"desc"}]}'
call GET "/api/workspaces/$SLUG/labels/"
