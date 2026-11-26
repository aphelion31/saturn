# Analytics (src/analytics.rs)
for relation in cycle module; do
  case "$relation" in
    cycle) table=cycles ;;
    module) table=modules ;;
  esac
  foreign_id=$(psql "${DATABASE_URL:-postgres://localhost/saturn}" -X -q -v ON_ERROR_STOP=1 -At -v slug="$SLUG" <<< "SELECT t.id FROM $table t JOIN workspaces w ON w.id=t.workspace_id WHERE w.slug <> :'slug' AND t.deleted_at IS NULL LIMIT 1")
  if [[ -n "$foreign_id" ]]; then
    call GET "/api/workspaces/$SLUG/projects/$PROJECT/advance-analytics-charts/?type=work-items&${relation}_id=$foreign_id"
    jq -e '.data == []' <<<"$body" >/dev/null
  fi
done
A="/api/workspaces/$SLUG/advance-analytics"
call GET "$A/?tab=overview"
jq -e 'has("total_users") and has("total_work_items")' <<<"$body" >/dev/null
call GET "$A/?tab=work-items"
call GET "$A-stats/?type=work-items"
call GET "$A-charts/?type=projects"
call GET "$A-charts/?type=work-items"
call GET "$A-charts/?type=custom-work-items&x_axis=PRIORITY"
call GET "/api/workspaces/$SLUG/projects/$PROJECT/advance-analytics/"
jq -e 'has("total_work_items")' <<<"$body" >/dev/null
call GET "/api/workspaces/$SLUG/projects/$PROJECT/advance-analytics-stats/?type=work-items"
call GET "/api/workspaces/$SLUG/projects/$PROJECT/advance-analytics-charts/?type=work-items"
call GET "/api/workspaces/$SLUG/project-stats/"
call GET "/api/workspaces/$SLUG/analytics/?x_axis=priority&y_axis=issue_count"
call GET "/api/workspaces/$SLUG/default-analytics/"
call POST "/api/workspaces/$SLUG/analytic-view/" '{"name":"Smoke '$RANDOM'","description":"","query_dict":{"x_axis":"priority","y_axis":"issue_count"}}'
AV=$(field '.id')
call GET "/api/workspaces/$SLUG/analytic-view/"
call GET "/api/workspaces/$SLUG/analytic-view/$AV/"
call PATCH "/api/workspaces/$SLUG/analytic-view/$AV/" '{"name":"Smoke renamed"}'
call GET "/api/workspaces/$SLUG/saved-analytic-view/$AV/?segment=state_id"
call DELETE "/api/workspaces/$SLUG/analytic-view/$AV/"
