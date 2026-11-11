# Public API v1 routes (src/api_v1.rs)
call POST /api/users/api-tokens/ '{"label":"smoke-v1"}'
V1_TOKEN=$(field '.token')

call_v1() {
  local out code
  out=$(curl -s -X "$1" -H "X-Api-Key: $V1_TOKEN" -H 'content-type: application/json' ${3:+-d "$3"} -w '\n%{http_code}' "$BASE$2")
  code=${out##*$'\n'}
  body=${out%$'\n'*}
  if [[ "$code" != 2* ]]; then echo "✖ $1 $2 -> $code $body" >&2; exit 1; fi
  echo "✔ v1 $1 $2 -> $code"
}

# 1. Cycles
call_v1 GET "/api/v1/workspaces/$SLUG/projects/$PROJECT/cycles/?cycle_view=all"
call_v1 GET "/api/v1/workspaces/$SLUG/projects/$PROJECT/cycles/?cycle_view=current"
call_v1 GET "/api/v1/workspaces/$SLUG/projects/$PROJECT/cycles-lite/"
call_v1 POST "/api/v1/workspaces/$SLUG/projects/$PROJECT/cycles/" '{"name":"Smoke V1 Cycle '$RANDOM'"}'
SMOKE_V1_CYCLE=$(field '.id')
call_v1 GET "/api/v1/workspaces/$SLUG/projects/$PROJECT/cycles/$SMOKE_V1_CYCLE/"
call_v1 PATCH "/api/v1/workspaces/$SLUG/projects/$PROJECT/cycles/$SMOKE_V1_CYCLE/" '{"description":"Cycle desc"}'
call POST "/api/workspaces/$SLUG/projects/$PROJECT/issues/" '{"name":"V1 Cycle test issue"}'
V1_C_ISSUE=$(field '.id')
call_v1 POST "/api/v1/workspaces/$SLUG/projects/$PROJECT/cycles/$SMOKE_V1_CYCLE/cycle-issues/" '{"issues":["'"$V1_C_ISSUE"'"]}'
call_v1 GET "/api/v1/workspaces/$SLUG/projects/$PROJECT/cycles/$SMOKE_V1_CYCLE/cycle-issues/"
call_v1 GET "/api/v1/workspaces/$SLUG/projects/$PROJECT/cycles/$SMOKE_V1_CYCLE/cycle-issues/$V1_C_ISSUE/"
call_v1 DELETE "/api/v1/workspaces/$SLUG/projects/$PROJECT/cycles/$SMOKE_V1_CYCLE/cycle-issues/$V1_C_ISSUE/"
call_v1 DELETE "/api/v1/workspaces/$SLUG/projects/$PROJECT/cycles/$SMOKE_V1_CYCLE/"

# 2. Modules
call_v1 GET "/api/v1/workspaces/$SLUG/projects/$PROJECT/modules/"
call_v1 GET "/api/v1/workspaces/$SLUG/projects/$PROJECT/modules-lite/"
call_v1 POST "/api/v1/workspaces/$SLUG/projects/$PROJECT/modules/" '{"name":"Smoke V1 Module '$RANDOM'"}'
SMOKE_V1_MOD=$(field '.id')
call_v1 GET "/api/v1/workspaces/$SLUG/projects/$PROJECT/modules/$SMOKE_V1_MOD/"
call_v1 PATCH "/api/v1/workspaces/$SLUG/projects/$PROJECT/modules/$SMOKE_V1_MOD/" '{"description":"Mod desc","status":"completed"}'
call POST "/api/workspaces/$SLUG/projects/$PROJECT/issues/" '{"name":"V1 Module test issue"}'
V1_M_ISSUE=$(field '.id')
call_v1 POST "/api/v1/workspaces/$SLUG/projects/$PROJECT/modules/$SMOKE_V1_MOD/module-issues/" '{"issues":["'"$V1_M_ISSUE"'"]}'
call_v1 GET "/api/v1/workspaces/$SLUG/projects/$PROJECT/modules/$SMOKE_V1_MOD/module-issues/"
call_v1 DELETE "/api/v1/workspaces/$SLUG/projects/$PROJECT/modules/$SMOKE_V1_MOD/module-issues/$V1_M_ISSUE/"
call_v1 POST "/api/v1/workspaces/$SLUG/projects/$PROJECT/modules/$SMOKE_V1_MOD/archive/"
call_v1 GET "/api/v1/workspaces/$SLUG/projects/$PROJECT/archived-modules/"
call_v1 DELETE "/api/v1/workspaces/$SLUG/projects/$PROJECT/archived-modules/$SMOKE_V1_MOD/unarchive/"
call_v1 DELETE "/api/v1/workspaces/$SLUG/projects/$PROJECT/modules/$SMOKE_V1_MOD/"

# 3. Members & Lite & Summary
call_v1 GET "/api/v1/workspaces/$SLUG/members-lite/"
call_v1 GET "/api/v1/workspaces/$SLUG/projects-lite/"
call_v1 GET "/api/v1/workspaces/$SLUG/projects/$PROJECT/summary/"
call_v1 GET "/api/v1/workspaces/$SLUG/projects/$PROJECT/summary/?fields=issues,cycles"
call_v1 GET "/api/v1/workspaces/$SLUG/projects/$PROJECT/project-members/"
call_v1 GET "/api/v1/workspaces/$SLUG/projects/$PROJECT/project-members-lite/"

# 4. Intake issues & Estimates
call_v1 GET "/api/v1/workspaces/$SLUG/projects/$PROJECT/intake-issues/"
call_v1 POST "/api/v1/workspaces/$SLUG/projects/$PROJECT/intake-issues/" '{"issue":{"name":"Smoke V1 Intake '$RANDOM'"}}'
V1_II=$(field '.issue')
call_v1 GET "/api/v1/workspaces/$SLUG/projects/$PROJECT/intake-issues/$V1_II/"
call_v1 PATCH "/api/v1/workspaces/$SLUG/projects/$PROJECT/intake-issues/$V1_II/" '{"status":1}'
call_v1 DELETE "/api/v1/workspaces/$SLUG/projects/$PROJECT/intake-issues/$V1_II/"

call_v1 GET "/api/v1/workspaces/$SLUG/projects/$PROJECT/estimates/"
V1_EST=$(field '.id')
call_v1 GET "/api/v1/workspaces/$SLUG/projects/$PROJECT/estimates/$V1_EST/estimate-points/"

# 5. Workspace detail & project members alias (Plane urls/member.py)
call_v1 GET "/api/v1/workspaces/$SLUG/"
call_v1 GET "/api/v1/workspaces/$SLUG/projects/$PROJECT/members/"
MEMBER_PK=$(field '.[0].id')
call_v1 GET "/api/v1/workspaces/$SLUG/projects/$PROJECT/members/$MEMBER_PK/"
