# Workspace seed, recent visits, project member email, v1 API activity log (src/workspace_seed.rs,
# src/recent_visits.rs, src/projects.rs add_members, src/api_log.rs). Uses its own API token so it does not
# depend on 97-api-v1.sh having run first.
DB_CONN="${DATABASE_URL:-postgres://localhost/saturn}"
q() { psql "$DB_CONN" -Atq -c "$1" | head -n 1; }

# --- workspace seed: create via API, poll up to ~10s for seeded project + issues ---
if [ "${SATURN_WORKSPACE_SEED:-1}" != "0" ]; then
  SEED_SLUG="seed-$RANDOM$RANDOM"
  call POST /api/workspaces/ "{\"name\":\"Seed $SEED_SLUG\",\"slug\":\"$SEED_SLUG\",\"organization_size\":\"2-10\"}"
  SEED_WS=$(field '.id')
  ISSUES=0
  for _ in $(seq 1 20); do
    ISSUES=$(q "SELECT count(*) FROM issues WHERE workspace_id='$SEED_WS' AND deleted_at IS NULL")
    [ "${ISSUES:-0}" -gt 0 ] && break
    sleep 0.5
  done
  if [ "${ISSUES:-0}" -lt 1 ]; then echo "✖ workspace seed produced no issues within 10s" >&2; exit 1; fi
  PROJECTS=$(q "SELECT count(*) FROM projects WHERE workspace_id='$SEED_WS' AND deleted_at IS NULL")
  BOTS=$(q "SELECT count(*) FROM users u JOIN workspace_members m ON m.member_id=u.id WHERE m.workspace_id='$SEED_WS' AND u.is_bot AND u.bot_type='WORKSPACE_SEED'")
  if [ "${PROJECTS:-0}" -lt 1 ] || [ "${BOTS:-0}" -lt 1 ]; then echo "✖ seed missing projects=$PROJECTS bots=$BOTS" >&2; exit 1; fi
  echo "✔ workspace seed: projects=$PROJECTS issues=$ISSUES bot=$BOTS"
else
  echo "· SATURN_WORKSPACE_SEED=0, skipping seed assertions"
fi

# --- recent visits: GET issue detail upserts user_recent_visits ---
ISSUE=$(q "SELECT id FROM issues WHERE project_id='$PROJECT' AND deleted_at IS NULL ORDER BY created_at LIMIT 1")
USER_ID=$(q "SELECT id FROM users WHERE email='$EMAIL'")
WS_ID=$(q "SELECT id FROM workspaces WHERE slug='$SLUG'")
call GET "/api/workspaces/$SLUG/projects/$PROJECT/issues/$ISSUE/"
RV=0
for _ in $(seq 1 20); do
  RV=$(q "SELECT count(*) FROM user_recent_visits WHERE user_id='$USER_ID' AND workspace_id='$WS_ID' AND entity_name='issue' AND entity_identifier='$ISSUE'")
  [ "${RV:-0}" -ge 1 ] && break
  sleep 0.25
done
if [ "${RV:-0}" -lt 1 ]; then echo "✖ no user_recent_visits row after GET issue detail" >&2; exit 1; fi
echo "✔ recent visit recorded for issue"

# --- v1 API activity log: X-Api-Key call writes a row; raw key never stored ---
call POST /api/users/api-tokens/ '{"label":"smoke-seed-recents"}'
LOG_TOKEN=$(field '.token')
C=$(curl -s -o /dev/null -w '%{http_code}' -H "X-Api-Key: $LOG_TOKEN" "$BASE/api/v1/users/me/")
[[ "$C" == 2* ]] || { echo "✖ v1 users/me -> $C" >&2; exit 1; }
AFTER=0
for _ in $(seq 1 20); do
  AFTER=$(q "SELECT count(*) FROM api_activity_logs WHERE path='/api/v1/users/me/' AND response_code=200")
  [ "${AFTER:-0}" -ge 1 ] && break
  sleep 0.25
done
if [ "${AFTER:-0}" -lt 1 ]; then echo "✖ no api_activity_logs row for v1 call" >&2; exit 1; fi
LEAK=$(q "SELECT count(*) FROM api_activity_logs WHERE headers LIKE '%$LOG_TOKEN%' OR token_identifier='$LOG_TOKEN'")
if [ "${LEAK:-0}" -ne 0 ]; then echo "✖ raw API key stored in api_activity_logs" >&2; exit 1; fi
echo "✔ api_activity_logs row recorded, key redacted"
