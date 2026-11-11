# Method parity: own tokens, identifier guard, board lifecycle and public invite lookup.
call POST /api/users/api-tokens/ '{"label":"method-smoke"}'
METHOD_TOKEN=$(field '.id')
call PATCH "/api/users/api-tokens/$METHOD_TOKEN/" '{"description":"updated"}'
[[ "$(field '.description')" == updated ]]
call DELETE "/api/users/api-tokens/$METHOD_TOKEN/"
call DELETE "/api/workspaces/$SLUG/project-identifiers/" '{"name":"UNUSED-SMOKE-IDENTIFIER"}'
call POST "/api/workspaces/$SLUG/projects/$PROJECT/project-deploy-boards/" '{}'
METHOD_BOARD=$(field '.id')
call GET "/api/workspaces/$SLUG/projects/$PROJECT/project-deploy-boards/$METHOD_BOARD/"
call DELETE "/api/workspaces/$SLUG/projects/$PROJECT/project-deploy-boards/$METHOD_BOARD/"
