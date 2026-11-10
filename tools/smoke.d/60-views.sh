# Issue views, workspace views, favorites, workspace issues (src/views.rs)
call POST "/api/workspaces/$SLUG/projects/$PROJECT/views/" '{"name":"Smoke view '$RANDOM'","filters":{"priority":["high"]}}'
VIEW=$(field "['id']")
call GET "/api/workspaces/$SLUG/projects/$PROJECT/views/"
call GET "/api/workspaces/$SLUG/projects/$PROJECT/views/$VIEW/"
call PATCH "/api/workspaces/$SLUG/projects/$PROJECT/views/$VIEW/" '{"name":"Renamed view","display_filters":{"group_by":"state"}}'
call POST "/api/workspaces/$SLUG/projects/$PROJECT/user-favorite-views/" '{"view":"'$VIEW'"}'
call GET "/api/workspaces/$SLUG/projects/$PROJECT/user-favorite-views/"
call DELETE "/api/workspaces/$SLUG/projects/$PROJECT/user-favorite-views/$VIEW/"
call DELETE "/api/workspaces/$SLUG/projects/$PROJECT/views/$VIEW/"
call POST "/api/workspaces/$SLUG/views/" '{"name":"Smoke ws view '$RANDOM'","filters":{}}'
WSVIEW=$(field "['id']")
call GET "/api/workspaces/$SLUG/views/"
call GET "/api/workspaces/$SLUG/views/$WSVIEW/"
call PATCH "/api/workspaces/$SLUG/views/$WSVIEW/" '{"name":"Renamed ws view"}'
call DELETE "/api/workspaces/$SLUG/views/$WSVIEW/"
call GET "/api/workspaces/$SLUG/issues/?per_page=5"
