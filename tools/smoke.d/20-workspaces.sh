# Workspaces (src/workspaces.rs)
call GET /api/users/me/workspaces/
call GET "/api/workspace-slug-check/?slug=smoke-$RANDOM"
call GET "/api/workspaces/$SLUG/"
call PATCH "/api/workspaces/$SLUG/" '{"name":"Acme"}'
call PATCH "/api/workspaces/$SLUG/" '{"timezone":"Asia/Hong_Kong"}'
[[ $(field "['timezone']") == Asia/Hong_Kong ]]
call GET "/api/workspaces/$SLUG/"
[[ $(field "['timezone']") == Asia/Hong_Kong ]]
code=$(curl -s -o /dev/null -w '%{http_code}' -b "$jar" -X PATCH -H 'content-type: application/json' -d '{"timezone":"not/a-zone"}' "$BASE/api/workspaces/$SLUG/")
[[ "$code" == 400 ]]
call GET "/api/workspaces/$SLUG/members/"
call GET "/api/workspaces/$SLUG/workspace-members/me/"
call GET "/api/workspaces/$SLUG/project-members/"
call GET "/api/users/me/workspaces/$SLUG/project-roles/"
call GET "/api/workspaces/$SLUG/user-properties/"
call PATCH "/api/workspaces/$SLUG/user-properties/" '{"navigation_project_limit":10}'
call GET /api/users/last-visited-workspace/
call GET "/api/workspaces/$SLUG/invitations/"
call POST "/api/workspaces/$SLUG/invitations/" '{"emails":[{"email":"smoke-'$RANDOM'@example.com","role":5}]}'
# Plane #9660: another account cannot accept the invitation.
INVITE=$(psql "${DATABASE_URL:-postgres://localhost/saturn}" -Atc "select i.id || ':' || i.token from workspace_member_invites i join workspaces w on w.id=i.workspace_id where w.slug='$SLUG' and i.deleted_at is null order by i.created_at desc limit 1")
code=$(curl -s -o /dev/null -w '%{http_code}' -b "$jar" -H 'content-type: application/json' -d "{\"token\":\"${INVITE#*:}\",\"accepted\":true}" "$BASE/api/workspaces/$SLUG/invitations/${INVITE%%:*}/join/")
[[ "$code" == 403 ]] || { echo "wrong-email invite returned $code" >&2; exit 1; }
call GET "/api/workspaces/$SLUG/user-favorites/"
call POST "/api/workspaces/$SLUG/user-favorites/" '{"entity_type":"folder","name":"Smoke","is_folder":true}'
UID_SELF=$(psql "${DATABASE_URL:-postgres://localhost/saturn}" -Atc "select u.id from users u where u.email='$EMAIL'" 2>/dev/null || true)
[[ -n "$UID_SELF" ]] && call GET "/api/workspaces/$SLUG/user-stats/$UID_SELF/" && call GET "/api/workspaces/$SLUG/user-profile/$UID_SELF/" && call GET "/api/workspaces/$SLUG/user-activity/$UID_SELF/"
