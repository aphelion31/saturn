# Modules: create, update, issue membership, links, favorites, user properties, archive, soft delete.
call POST "/api/workspaces/$SLUG/projects/$PROJECT/modules/" '{"name":"Smoke module","status":"planned"}'
MOD=$(field '.id')
call POST "/api/workspaces/$SLUG/projects/$PROJECT/issues/" '{"name":"Module count one"}'
MOD_ISSUE1=$(field '.id')
call POST "/api/workspaces/$SLUG/projects/$PROJECT/issues/" '{"name":"Module count two"}'
MOD_ISSUE2=$(field '.id')
call POST "/api/workspaces/$SLUG/projects/$PROJECT/modules/$MOD/issues/" '{"issues":["'$MOD_ISSUE1'","'$MOD_ISSUE2'"]}'
call GET "/api/workspaces/$SLUG/projects/$PROJECT/modules/$MOD/"
[[ "$(field '.total_issues')" == 2 ]]
call DELETE "/api/workspaces/$SLUG/projects/$PROJECT/issues/$MOD_ISSUE1/"
call DELETE "/api/workspaces/$SLUG/projects/$PROJECT/issues/$MOD_ISSUE2/"
call GET "/api/workspaces/$SLUG/projects/$PROJECT/modules/"
call GET "/api/workspaces/$SLUG/projects/$PROJECT/modules/$MOD/"
call PATCH "/api/workspaces/$SLUG/projects/$PROJECT/modules/$MOD/" '{"name":"Smoke module 2"}'
call POST "/api/workspaces/$SLUG/projects/$PROJECT/user-favorite-modules/" "{\"module\":\"$MOD\"}"
call GET "/api/workspaces/$SLUG/projects/$PROJECT/user-favorite-modules/"
call DELETE "/api/workspaces/$SLUG/projects/$PROJECT/user-favorite-modules/$MOD/"
call POST "/api/workspaces/$SLUG/projects/$PROJECT/modules/$MOD/module-links/" '{"url":"example.com/doc","title":"doc"}'
call GET "/api/workspaces/$SLUG/projects/$PROJECT/modules/$MOD/module-links/"
call GET "/api/workspaces/$SLUG/projects/$PROJECT/modules/$MOD/user-properties/"
call PATCH "/api/workspaces/$SLUG/projects/$PROJECT/modules/$MOD/user-properties/" '{"display_properties":{"assignee":false}}'
call GET "/api/workspaces/$SLUG/projects/$PROJECT/modules/$MOD/issues/"
call DELETE "/api/workspaces/$SLUG/projects/$PROJECT/modules/$MOD/"
call GET "/api/workspaces/$SLUG/projects/$PROJECT/archived-modules/"
