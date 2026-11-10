# Issue views and workspace issues (src/views.rs)
call POST "/api/workspaces/$SLUG/projects/$PROJECT/views/" '{"name":"Smoke view '$RANDOM'","filters":{"priority":["high"]}}'
VIEW=$(field "['id']")
call GET "/api/workspaces/$SLUG/projects/$PROJECT/views/"
call GET "/api/workspaces/$SLUG/projects/$PROJECT/views/$VIEW/"
call PATCH "/api/workspaces/$SLUG/projects/$PROJECT/views/$VIEW/" '{"name":"Renamed view","display_filters":{"group_by":"state"}}'
call DELETE "/api/workspaces/$SLUG/projects/$PROJECT/views/$VIEW/"
call GET "/api/workspaces/$SLUG/issues/?per_page=5"
