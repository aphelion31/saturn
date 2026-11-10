# Project pages (src/pages.rs)
call GET "/api/workspaces/$SLUG/projects/$PROJECT/pages/"
call GET "/api/workspaces/$SLUG/projects/$PROJECT/pages-summary/"
call POST "/api/workspaces/$SLUG/projects/$PROJECT/pages/" '{"name":"Smoke page"}'
PAGE=$(field "['id']")
call GET "/api/workspaces/$SLUG/projects/$PROJECT/pages/$PAGE/"
call PATCH "/api/workspaces/$SLUG/projects/$PROJECT/pages/$PAGE/" '{"name":"Updated smoke page"}'
call GET "/api/workspaces/$SLUG/projects/$PROJECT/pages/$PAGE/description/"
call POST "/api/workspaces/$SLUG/projects/$PROJECT/pages/$PAGE/archive/"
call DELETE "/api/workspaces/$SLUG/projects/$PROJECT/pages/$PAGE/"
