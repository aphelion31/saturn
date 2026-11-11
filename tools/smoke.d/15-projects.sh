# Project list and details
call GET "/api/workspaces/$SLUG/projects/"
call GET "/api/workspaces/$SLUG/projects/details/"
call GET "/api/workspaces/$SLUG/project-identifiers/?name=GS"
call GET "/api/workspaces/$SLUG/projects/$PROJECT/"
call GET "/api/workspaces/$SLUG/projects/$PROJECT/user-properties/"
call PATCH "/api/workspaces/$SLUG/projects/$PROJECT/user-properties/" '{"filters":{"priority":["high"]}}'
[[ $(field '.filters.priority[0]') == high ]]
call GET "/api/workspaces/$SLUG/projects/$PROJECT/estimates/"
call GET "/api/workspaces/$SLUG/estimates/"
call POST "/api/workspaces/$SLUG/projects/$PROJECT/estimates/" '{"estimate":{"name":"Smoke","type":"categories"},"estimate_points":[{"key":1,"value":"1"}]}'
ESTIMATE=$(field '.id')
POINT=$(field '.points[0].id')
call PATCH "/api/workspaces/$SLUG/projects/$PROJECT/estimates/$ESTIMATE/" '{"estimate_points":[{"id":"'"$POINT"'","value":"2"}]}'
call POST "/api/workspaces/$SLUG/projects/$PROJECT/estimates/$ESTIMATE/estimate-points/" '{"key":2,"value":"3"}'
POINT2=$(field '.id')
call PATCH "/api/workspaces/$SLUG/projects/$PROJECT/estimates/$ESTIMATE/estimate-points/$POINT2/" '{"value":"5"}'
call DELETE "/api/workspaces/$SLUG/projects/$PROJECT/estimates/$ESTIMATE/estimate-points/$POINT2/" '{}'
call DELETE "/api/workspaces/$SLUG/projects/$PROJECT/estimates/$ESTIMATE/"
call GET "/api/workspaces/$SLUG/projects/$PROJECT/issue-labels/"
call POST "/api/workspaces/$SLUG/projects/$PROJECT/issue-labels/" '{"name":"Smoke '$RANDOM'","color":"#123456"}'
LABEL=$(field '.id')
call GET "/api/workspaces/$SLUG/projects/$PROJECT/issue-labels/$LABEL/"
call PATCH "/api/workspaces/$SLUG/projects/$PROJECT/issue-labels/$LABEL/" '{"color":"#654321"}'
call DELETE "/api/workspaces/$SLUG/projects/$PROJECT/issue-labels/$LABEL/"
call GET "/api/workspaces/$SLUG/projects/$PROJECT/members/"
call GET "/api/workspaces/$SLUG/projects/$PROJECT/project-members/me/"
call GET "/api/workspaces/$SLUG/user-favorite-projects/"
call POST "/api/workspaces/$SLUG/user-favorite-projects/" '{"project":"'"$PROJECT"'"}'
call DELETE "/api/workspaces/$SLUG/user-favorite-projects/$PROJECT/"
call POST "/api/workspaces/$SLUG/projects/" '{"name":"Smoke project '$RANDOM'","identifier":"S'$RANDOM'"}'
SMOKE_PROJECT=$(field '.id')
# Use a unique disposable workspace member with the seed user's password hash.
SMOKE_USER=$(psql "${DATABASE_URL:-postgres://localhost/saturn}" -X -q -v ON_ERROR_STOP=1 -At -v email="$EMAIL" <<< "INSERT INTO users (email, username, password, display_name) SELECT gen_random_uuid()::text || '@smoke.invalid', gen_random_uuid()::text, password, 'Smoke member' FROM users WHERE email=:'email' RETURNING id")
[[ -n "$SMOKE_USER" ]]
SMOKE_EMAIL=$(psql "${DATABASE_URL:-postgres://localhost/saturn}" -X -v ON_ERROR_STOP=1 -At -v id="$SMOKE_USER" <<< "SELECT email FROM users WHERE id=:'id'::uuid")
psql "${DATABASE_URL:-postgres://localhost/saturn}" -X -v ON_ERROR_STOP=1 -q -v id="$SMOKE_USER" -v slug="$SLUG" <<< "INSERT INTO workspace_members (workspace_id, member_id, role) SELECT id, :'id'::uuid, 15 FROM workspaces WHERE slug=:'slug'"
MEMBER_JAR=$(mktemp)
member_redirect=$(curl -s -o /dev/null -w '%{redirect_url}' -c "$MEMBER_JAR" -d "email=$SMOKE_EMAIL&password=$PASSWORD" "$BASE/auth/sign-in/")
[[ "$member_redirect" != *error_code* ]]
call POST "/api/workspaces/$SLUG/projects/$SMOKE_PROJECT/members/" '{"members":[{"member_id":"'"$SMOKE_USER"'","role":15}]}'
MEMBERSHIP=$(field "[.[] | select(.member.id==\"$SMOKE_USER\")][0].id")
call PATCH "/api/workspaces/$SLUG/projects/$SMOKE_PROJECT/members/$MEMBERSHIP/" '{"role":5}'
call PATCH "/api/workspaces/$SLUG/projects/$SMOKE_PROJECT/members/$MEMBERSHIP/" '{"role":15}'
code=$(curl -s -o /dev/null -w '%{http_code}' -b "$MEMBER_JAR" -X PATCH -H 'content-type: application/json' -d '{"role":20}' "$BASE/api/workspaces/$SLUG/projects/$SMOKE_PROJECT/members/$MEMBERSHIP/")
[[ "$code" == 403 ]] || { echo "✖ member promotion -> $code, expected 403" >&2; exit 1; }
echo "✔ member promotion rejected -> 403"
INVITE=$(psql "${DATABASE_URL:-postgres://localhost/saturn}" -X -q -v ON_ERROR_STOP=1 -At -v project="$SMOKE_PROJECT" -v email="$SMOKE_EMAIL" <<< "INSERT INTO project_member_invites (workspace_id,project_id,email,token,role) SELECT workspace_id,id,:'email','smoke-valid-token',15 FROM projects WHERE id=:'project'::uuid RETURNING id")
code=$(curl -s -o /dev/null -w '%{http_code}' -b "$MEMBER_JAR" -X POST -H 'content-type: application/json' -d '{"token":"wrong-token","accepted":true}' "$BASE/api/workspaces/$SLUG/projects/$SMOKE_PROJECT/join/$INVITE/")
[[ "$code" == 403 ]] || { echo "✖ invalid invite token -> $code, expected 403" >&2; exit 1; }
echo "✔ invalid invite token rejected -> 403"
code=$(curl -s -o /dev/null -w '%{http_code}' -b "$MEMBER_JAR" -X POST "$BASE/api/workspaces/$SLUG/projects/$SMOKE_PROJECT/members/leave/")
[[ "$code" == 204 ]] || { echo "✖ member leave -> $code, expected 204" >&2; exit 1; }
echo "✔ member leave -> 204"
call POST "/api/workspaces/$SLUG/projects/$SMOKE_PROJECT/members/" '{"members":[{"member_id":"'"$SMOKE_USER"'","role":15}]}'
call DELETE "/api/workspaces/$SLUG/projects/$SMOKE_PROJECT/members/$MEMBERSHIP/"
psql "${DATABASE_URL:-postgres://localhost/saturn}" -X -q -v ON_ERROR_STOP=1 -v project="$SMOKE_PROJECT" <<< "UPDATE projects SET network=2 WHERE id=:'project'::uuid"
code=$(curl -s -o /dev/null -w '%{http_code}' -b "$MEMBER_JAR" -X POST -H 'content-type: application/json' -d '{"project_ids":["'"$SMOKE_PROJECT"'"]}' "$BASE/api/users/me/workspaces/$SLUG/projects/invitations/")
[[ "$code" == 403 ]] || { echo "✖ removed member rejoin -> $code" >&2; exit 1; }
rm -f "$MEMBER_JAR"
call GET "/api/workspaces/$SLUG/projects/$SMOKE_PROJECT/project-deploy-boards/"
call POST "/api/workspaces/$SLUG/projects/$SMOKE_PROJECT/project-deploy-boards/" '{"is_comments_enabled":true}'
BOARD=$(field '.id')
call PATCH "/api/workspaces/$SLUG/projects/$SMOKE_PROJECT/project-deploy-boards/$BOARD/" '{"is_votes_enabled":true}'
call PATCH "/api/workspaces/$SLUG/projects/$SMOKE_PROJECT/" '{"description":"Smoke test"}'
call POST "/api/workspaces/$SLUG/projects/$SMOKE_PROJECT/archive/"
call DELETE "/api/workspaces/$SLUG/projects/$SMOKE_PROJECT/archive/"
call DELETE "/api/workspaces/$SLUG/projects/$SMOKE_PROJECT/"
