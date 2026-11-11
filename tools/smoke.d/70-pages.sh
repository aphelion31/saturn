# Project pages (src/pages.rs)
call GET "/api/workspaces/$SLUG/projects/$PROJECT/pages/"
call GET "/api/workspaces/$SLUG/projects/$PROJECT/pages-summary/"
call POST "/api/workspaces/$SLUG/projects/$PROJECT/pages/" '{"name":"Smoke page"}'
PAGE=$(field '.id')
call GET "/api/workspaces/$SLUG/projects/$PROJECT/pages/$PAGE/"
call PATCH "/api/workspaces/$SLUG/projects/$PROJECT/pages/$PAGE/" '{"name":"Updated smoke page"}'
call POST "/api/workspaces/$SLUG/projects/$PROJECT/favorite-pages/$PAGE/"
call GET "/api/workspaces/$SLUG/projects/$PROJECT/pages/$PAGE/"
[[ "$(field '.is_favorite')" == true ]]
call DELETE "/api/workspaces/$SLUG/projects/$PROJECT/favorite-pages/$PAGE/"
call GET "/api/workspaces/$SLUG/projects/$PROJECT/pages/$PAGE/versions/"
call POST "/api/workspaces/$SLUG/projects/$PROJECT/pages/$PAGE/duplicate/"
COPY=$(field '.id')
call POST "/api/workspaces/$SLUG/projects/$PROJECT/pages/$COPY/archive/"
call DELETE "/api/workspaces/$SLUG/projects/$PROJECT/pages/$COPY/"
call PATCH "/api/workspaces/$SLUG/projects/$PROJECT/pages/$PAGE/description/" '{"description_binary":"AQIDBA==","description_html":"<p>safe</p><script>alert(1)</script>"}'
call GET "/api/workspaces/$SLUG/projects/$PROJECT/pages/$PAGE/versions/"
VERSION=$(field "[0]['id']")
call GET "/api/workspaces/$SLUG/projects/$PROJECT/pages/$PAGE/versions/$VERSION/"
[[ "$(field "['description_html']")" == '<p>safe</p>' ]]
call GET "/api/workspaces/$SLUG/projects/$PROJECT/pages/$PAGE/"
[[ "$(field '.description_html')" == '<p>safe</p>' ]]
call GET "/api/workspaces/$SLUG/projects/$PROJECT/pages/$PAGE/description/"
call POST "/api/workspaces/$SLUG/projects/$PROJECT/pages/$PAGE/archive/"
call DELETE "/api/workspaces/$SLUG/projects/$PROJECT/pages/$PAGE/"
