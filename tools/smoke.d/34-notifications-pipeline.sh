# Notifications reach assigned and mentioned project members, never their actor.
NT_DB=${DATABASE_URL:-postgres://localhost/saturn}
NT_USER=$(psql "$NT_DB" -X -q -v ON_ERROR_STOP=1 -At -v email="$EMAIL" <<< "INSERT INTO users (email,username,password,display_name) SELECT gen_random_uuid()::text || '@smoke.invalid',gen_random_uuid()::text,password,'Notification member' FROM users WHERE email=:'email' RETURNING id")
NT_EMAIL=$(psql "$NT_DB" -X -q -v ON_ERROR_STOP=1 -At -v id="$NT_USER" <<< "SELECT email FROM users WHERE id=:'id'::uuid")
psql "$NT_DB" -X -q -v ON_ERROR_STOP=1 -v id="$NT_USER" -v slug="$SLUG" <<< "INSERT INTO workspace_members (workspace_id,member_id,role) SELECT id,:'id'::uuid,15 FROM workspaces WHERE slug=:'slug'"
call POST "/api/workspaces/$SLUG/projects/$PROJECT/members/" '{"members":[{"member_id":"'"$NT_USER"'","role":15}]}'
call POST "/api/workspaces/$SLUG/projects/$PROJECT/issues/" '{"name":"Notification smoke","assignee_ids":["'"$NT_USER"'"]}'
NT_ISSUE=$(field '.id')
call PATCH "/api/workspaces/$SLUG/projects/$PROJECT/issues/$NT_ISSUE/" '{"priority":"high"}'
call POST "/api/workspaces/$SLUG/projects/$PROJECT/issues/$NT_ISSUE/comments/" '{"comment_html":"<mention-component entity_identifier=\"'"$NT_USER"'\" entity_name=\"user_mention\"></mention-component>"}'
NT_JAR=$(mktemp)
nt_redirect=$(curl -s -o /dev/null -w '%{redirect_url}' -c "$NT_JAR" -d "email=$NT_EMAIL&password=$PASSWORD" "$BASE/auth/sign-in/")
[[ "$nt_redirect" != *error_code* ]]
for _ in {1..50}; do
  NT_COUNT=$(psql "$NT_DB" -X -q -At -v issue="$NT_ISSUE" -v user="$NT_USER" <<< "SELECT count(*) FROM notifications WHERE entity_identifier=:'issue'::uuid AND receiver_id=:'user'::uuid")
  [[ "$NT_COUNT" -ge 2 ]] && break
  sleep 0.1
done
nt_body=$(curl -fsS -b "$NT_JAR" "$BASE/api/workspaces/$SLUG/users/notifications/?type=assigned")
jq -e --arg id "$NT_ISSUE" 'any(.results[]; .entity_identifier == $id)' <<<"$nt_body" >/dev/null
nt_body=$(curl -fsS -b "$NT_JAR" "$BASE/api/workspaces/$SLUG/users/notifications/?mentioned=true")
jq -e --arg id "$NT_ISSUE" 'any(.results[]; .entity_identifier == $id and (.sender | contains("mentioned")))' <<<"$nt_body" >/dev/null
[[ $(psql "$NT_DB" -X -q -At -v issue="$NT_ISSUE" -v user="$NT_USER" <<< "SELECT count(*) FROM email_notification_logs WHERE entity_identifier=:'issue'::uuid AND receiver_id=:'user'::uuid") -ge 1 ]]
[[ $(psql "$NT_DB" -X -q -At -v issue="$NT_ISSUE" <<< "SELECT count(*) FROM notifications WHERE entity_identifier=:'issue'::uuid AND receiver_id=triggered_by_id") == 0 ]]
# Rules verified against a live Plane worker: issue creation itself does not notify, comment mentions
# name the actor, mention emails use field "mention", in-app data carries no activity_time.
[[ $(psql "$NT_DB" -X -q -At -v issue="$NT_ISSUE" <<< "SELECT count(*) FROM notifications WHERE entity_identifier=:'issue'::uuid AND data->'issue_activity'->>'verb'='created' AND coalesce(data->'issue_activity'->>'field','')=''") == 0 ]] || { echo "✖ issue creation must not notify" >&2; exit 1; }
[[ $(psql "$NT_DB" -X -q -At -v issue="$NT_ISSUE" <<< "SELECT count(*) FROM notifications WHERE entity_identifier=:'issue'::uuid AND sender LIKE '%mentioned' AND message::text LIKE '%has mentioned you in a comment in issue%'") -ge 1 ]] || { echo "✖ comment mention message" >&2; exit 1; }
[[ $(psql "$NT_DB" -X -q -At -v issue="$NT_ISSUE" <<< "SELECT count(*) FROM email_notification_logs WHERE entity_identifier=:'issue'::uuid AND data->'issue_activity'->>'field'='mention'") -ge 1 ]] || { echo "✖ mention email field" >&2; exit 1; }
[[ $(psql "$NT_DB" -X -q -At -v issue="$NT_ISSUE" <<< "SELECT count(*) FROM notifications WHERE entity_identifier=:'issue'::uuid AND data->'issue_activity' ? 'activity_time'") == 0 ]] || { echo "✖ in-app data has activity_time" >&2; exit 1; }
echo "✔ notifications follow Plane rules (no create notice, mention message/email shape)"
rm -f "$NT_JAR"
