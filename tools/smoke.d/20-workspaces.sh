# Workspaces (src/workspaces.rs)
# Plane #10001: drafts are per author; an unknown or foreign draft id is not found (never 403/500).
code=$(curl -s -o /dev/null -w '%{http_code}' -b "$jar" "$BASE/api/workspaces/$SLUG/draft-issues/00000000-0000-0000-0000-000000000001/")
[[ "$code" == 404 ]] || { echo "✖ unknown draft -> $code, expected 404" >&2; exit 1; }
call GET /api/users/me/workspaces/
call GET "/api/workspace-slug-check/?slug=smoke-$RANDOM"
call GET "/api/workspaces/$SLUG/"
call PATCH "/api/workspaces/$SLUG/" '{"name":"Acme"}'
call PATCH "/api/workspaces/$SLUG/" '{"timezone":"Asia/Hong_Kong"}'
[[ $(field '.timezone') == Asia/Hong_Kong ]]
call GET "/api/workspaces/$SLUG/"
[[ $(field '.timezone') == Asia/Hong_Kong ]]
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
# Plane #9662: removing a member revokes an unconsumed invitation.
test_email="removed-$RANDOM-$RANDOM@example.com"
test_user=$(echo "INSERT INTO users (email, username, password, display_name, token, is_active, last_login_medium, token_updated_at) SELECT :'email', gen_random_uuid()::text, password, 'Removed', gen_random_uuid()::text, true, 'email', now() FROM users WHERE email = :'admin' RETURNING id" | psql "${DATABASE_URL:-postgres://localhost/saturn}" -At -v ON_ERROR_STOP=1 -v email="$test_email" -v admin="$EMAIL" | head -1)
member=$(echo "INSERT INTO workspace_members (id, workspace_id, member_id, role, is_active, view_props, default_props, issue_props, explored_features, getting_started_checklist, tips) SELECT gen_random_uuid(), id, :'uid'::uuid, 15, true, '{}', '{}', '{}', '{}', '{}', '{}' FROM workspaces WHERE slug = :'slug' RETURNING id" | psql "${DATABASE_URL:-postgres://localhost/saturn}" -At -v ON_ERROR_STOP=1 -v uid="$test_user" -v slug="$SLUG" | head -1)
echo "INSERT INTO workspace_member_invites (id,email,token,role,accepted,workspace_id) SELECT gen_random_uuid(), :'email', 'stale-token', 15, false, id FROM workspaces WHERE slug = :'slug'" | psql "${DATABASE_URL:-postgres://localhost/saturn}" -q -v ON_ERROR_STOP=1 -v email="$test_email" -v slug="$SLUG"
stale=$(echo "SELECT id FROM workspace_member_invites WHERE email = :'email' AND deleted_at IS NULL" | psql "${DATABASE_URL:-postgres://localhost/saturn}" -At -v email="$test_email")
call DELETE "/api/workspaces/$SLUG/members/$member/"
testjar=$(mktemp)
curl -s -o /dev/null -c "$testjar" -d "email=$test_email&password=$PASSWORD" "$BASE/auth/sign-in/"
code=$(curl -s -o /dev/null -w '%{http_code}' -b "$testjar" -H 'content-type: application/json' -d '{"token":"stale-token","accepted":true}' "$BASE/api/workspaces/$SLUG/invitations/$stale/join/")
[[ "$code" != 2* ]]
curl -s -o /dev/null -b "$testjar" -H 'content-type: application/json' -d "{\"invitations\":[\"$stale\"]}" "$BASE/api/users/me/workspaces/invitations/"
[[ $(echo "SELECT is_active FROM workspace_members m JOIN workspaces w ON w.id=m.workspace_id WHERE m.member_id=:'uid'::uuid AND w.slug=:'slug'" | psql "${DATABASE_URL:-postgres://localhost/saturn}" -At -v uid="$test_user" -v slug="$SLUG") == f ]]
rm -f "$testjar"
call GET "/api/workspaces/$SLUG/user-favorites/"
call POST "/api/workspaces/$SLUG/user-favorites/" '{"entity_type":"folder","name":"Smoke","is_folder":true}'
UID_SELF=$(psql "${DATABASE_URL:-postgres://localhost/saturn}" -Atc "select u.id from users u where u.email='$EMAIL'" 2>/dev/null || true)
[[ -n "$UID_SELF" ]] && call GET "/api/workspaces/$SLUG/user-stats/$UID_SELF/" && call GET "/api/workspaces/$SLUG/user-profile/$UID_SELF/" && call GET "/api/workspaces/$SLUG/user-activity/$UID_SELF/"
if [[ -n "$UID_SELF" ]]; then
  # The profile overview page maps over these arrays; a missing key crashes it.
  call GET "/api/workspaces/$SLUG/user-stats/$UID_SELF/"
  jq -e '(.state_distribution|type=="array") and (.priority_distribution|type=="array") and (.present_cycles|type=="array") and (.upcoming_cycles|type=="array")' <<<"$body" >/dev/null \
    || { echo "✖ user-stats is missing distribution or cycle arrays" >&2; exit 1; }
fi
