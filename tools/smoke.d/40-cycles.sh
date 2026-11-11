# Cycles (src/cycles.rs)
C="/api/workspaces/$SLUG/projects/$PROJECT/cycles"
call POST "$C/" '{"name":"Smoke cycle '$RANDOM'","start_date":"2030-01-01","end_date":"2030-01-10"}'
CYCLE=$(field '.id')
call GET "$C/"
call GET "$C/$CYCLE/"
call PATCH "$C/$CYCLE/" '{"name":"Smoke renamed"}'
call POST "$C/date-check/" '{"start_date":"2040-01-01","end_date":"2040-01-05"}'
call GET "$C/$CYCLE/progress/"
call GET "$C/$CYCLE/user-properties/"
call PATCH "$C/$CYCLE/user-properties/" '{"display_properties":{"state":false}}'
call GET "$C/$CYCLE/analytics/"
call POST "/api/workspaces/$SLUG/projects/$PROJECT/user-favorite-cycles/" '{"cycle":"'$CYCLE'"}'
call GET "/api/workspaces/$SLUG/projects/$PROJECT/user-favorite-cycles/"
call DELETE "/api/workspaces/$SLUG/projects/$PROJECT/user-favorite-cycles/$CYCLE/"
call DELETE "$C/$CYCLE/"
