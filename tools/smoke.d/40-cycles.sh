# Cycles (src/cycles.rs)
C="/api/workspaces/$SLUG/projects/$PROJECT/cycles"
call POST "$C/" '{"name":"Completed cycle","start_date":"2000-01-01","end_date":"2000-01-10"}'
OLD_CYCLE=$(field "['id']")
call PATCH "$C/$OLD_CYCLE/" '{"sort_order":42,"name":"Smuggled"}'
call GET "$C/$OLD_CYCLE/"
[[ "$(field "['name']")" == 'Completed cycle' ]]
call DELETE "$C/$OLD_CYCLE/"
call POST "$C/" '{"name":"Smoke cycle '$RANDOM'","start_date":"2030-01-01","end_date":"2030-01-10"}'
CYCLE=$(field '.id')
call POST "/api/workspaces/$SLUG/projects/$PROJECT/issues/" '{"name":"Cycle issue"}'
CYCLE_ISSUE=$(field '.id')
call POST "$C/$CYCLE/cycle-issues/" '{"issues":["'$CYCLE_ISSUE'"]}'
call POST "$C/$CYCLE/cycle-issues/" '{"issues":["'$CYCLE_ISSUE'"]}'
call DELETE "/api/workspaces/$SLUG/projects/$PROJECT/issues/$CYCLE_ISSUE/"
call GET "$C/"
call GET "$C/$CYCLE/"
call PATCH "$C/$CYCLE/" '{"name":"Smoke renamed"}'
call POST "$C/date-check/" '{"start_date":"2040-01-01","end_date":"2040-01-05"}'
call GET "$C/$CYCLE/progress/"
call GET "$C/$CYCLE/user-properties/"
call PATCH "$C/$CYCLE/user-properties/" '{"display_properties":{"state":false}}'
call GET "$C/$CYCLE/analytics/"
call GET "/api/workspaces/$SLUG/projects/$PROJECT/issues/?updated_at__exact=1900-01-01"
[[ "$(field "['total_count']")" == 0 ]]
call GET "/api/workspaces/$SLUG/projects/$PROJECT/issues/?per_page=0"
call GET "/api/workspaces/$SLUG/projects/$PROJECT/issues/?per_page=-1"
call GET "/api/workspaces/$SLUG/projects/$PROJECT/search-issues/?search=2147483648"
call GET "/api/workspaces/$SLUG/projects/$PROJECT/issues/?start_date=1900-01-01%3Bbefore"
[[ "$(field "['total_count']")" == 0 ]]
call POST "/api/workspaces/$SLUG/projects/$PROJECT/user-favorite-cycles/" '{"cycle":"'$CYCLE'"}'
call GET "/api/workspaces/$SLUG/projects/$PROJECT/user-favorite-cycles/"
call DELETE "/api/workspaces/$SLUG/projects/$PROJECT/user-favorite-cycles/$CYCLE/"
call DELETE "$C/$CYCLE/"
