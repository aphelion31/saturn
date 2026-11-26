# God-mode instance admin (src/instance.rs). The seeded user is the instance admin.
curl -s -o /dev/null -b "$jar" -c "$jar" -d "email=$EMAIL&password=$PASSWORD" "$BASE/api/instances/admins/sign-in/"
call GET /api/instances/admins/session/
call GET /api/instances/configurations/
call PATCH /api/instances/configurations/ '{"ENABLE_SIGNUP":"1"}'
call GET /api/instances/admins/
call GET /api/instances/workspaces/
call GET "/api/instances/workspace-slug-check/?slug=smoke-$RANDOM"
