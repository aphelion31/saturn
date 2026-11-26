# Intake container create (src/views.rs intake_create): Plane IntakeViewSet.perform_create.
call POST "/api/workspaces/$SLUG/projects/$PROJECT/intakes/" '{"name":"Smoke intake '$RANDOM'"}'
[[ "$(field '.is_default')" == false ]] || { echo "✖ new intake should not be default" >&2; exit 1; }
SMOKE_INTAKE=$(field '.id')
call DELETE "/api/workspaces/$SLUG/projects/$PROJECT/intakes/$SMOKE_INTAKE/"
