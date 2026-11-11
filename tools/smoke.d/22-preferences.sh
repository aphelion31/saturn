# Sidebar and home preferences (src/workspaces.rs)
# Plane #9283: self-hosted asset URLs are valid profile covers.
cover='/api/assets/v2/static/00000000-0000-0000-0000-000000000001/'
call PATCH /api/users/me/ "{\"cover_image\":\"$cover\"}"
[[ $(field '.cover_image') == "$cover" ]]
call GET /api/users/me/
[[ $(field '.cover_image') == "$cover" ]]
call PATCH /api/users/me/ '{"user_timezone":"America/New_York"}'
[[ $(field '.user_timezone') == America/New_York ]]
call GET /api/users/me/profile/
[[ $(field '.user_timezone') == America/New_York ]]
call PATCH /api/users/me/profile/ '{"user_timezone":"Asia/Tokyo"}'
[[ $(field '.user_timezone') == Asia/Tokyo ]]
call GET /api/users/me/
[[ $(field '.user_timezone') == Asia/Tokyo ]]
call GET "/api/workspaces/$SLUG/user-profile/$UID_SELF/"
[[ $(field '.user_data.user_timezone') == Asia/Tokyo ]]
code=$(curl -s -o /dev/null -w '%{http_code}' -b "$jar" -X PATCH -H 'content-type: application/json' -d '{"user_timezone":"not/a-zone"}' "$BASE/api/users/me/")
[[ "$code" == 400 ]]
call GET "/api/workspaces/$SLUG/sidebar-preferences/"
call PATCH "/api/workspaces/$SLUG/sidebar-preferences/" '[{"key":"views","is_pinned":true}]'
call PATCH "/api/workspaces/$SLUG/sidebar-preferences/analytics/" '{"sort_order":1}'
call GET "/api/workspaces/$SLUG/home-preferences/"
call PATCH "/api/workspaces/$SLUG/home-preferences/recents/" '{"is_enabled":false}'
