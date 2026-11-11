# Modules: create, update, issue membership, links, favorites, user properties, archive, soft delete.
call POST "/api/workspaces/$SLUG/projects/$PROJECT/modules/" '{"name":"Smoke module","status":"planned"}'
MOD=$(field '.id')
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
