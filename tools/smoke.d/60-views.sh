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
# Intake (src/views.rs, Plane's intake endpoints)
call GET "/api/workspaces/$SLUG/projects/$PROJECT/intakes/"
INTAKE=$(field "['id']")
call GET "/api/workspaces/$SLUG/projects/$PROJECT/intakes/$INTAKE/"
call POST "/api/workspaces/$SLUG/projects/$PROJECT/intake-issues/" '{"issue":{"name":"Smoke intake '$RANDOM'","priority":"high","description_html":"<p>Hi <script>x</script></p>"}}'
II=$(field "['issue']['id']")
call GET "/api/workspaces/$SLUG/projects/$PROJECT/intake-issues/?per_page=5"
call GET "/api/workspaces/$SLUG/projects/$PROJECT/intake-issues/$II/"
call PATCH "/api/workspaces/$SLUG/projects/$PROJECT/intake-issues/$II/" '{"status":1,"issue":{"name":"Accepted"}}'
call DELETE "/api/workspaces/$SLUG/projects/$PROJECT/intake-issues/$II/"
