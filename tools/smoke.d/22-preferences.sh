# Sidebar and home preferences (src/workspaces.rs)
# Plane #9283: self-hosted asset URLs are valid profile covers.
cover='/api/assets/v2/static/00000000-0000-0000-0000-000000000001/'
call PATCH /api/users/me/ "{\"cover_image\":\"$cover\"}"
[[ $(field '.cover_image') == "$cover" ]]
call GET /api/users/me/
[[ $(field '.cover_image') == "$cover" ]]
# cover_image is a URL field: script schemes are rejected (stored-XSS sink), null clears it.
for bad in 'javascript:alert(1)' 'data:text/html,x' 'ftp://example.com/x.png' 'not a url'; do
  code=$(curl -s -o /dev/null -w '%{http_code}' -b "$jar" -X PATCH -H 'content-type: application/json' -d "{\"cover_image\":\"$bad\"}" "$BASE/api/users/me/")
  [[ "$code" == 400 ]] || { echo "✖ cover_image '$bad' -> $code, expected 400" >&2; exit 1; }
done
echo "✔ cover_image rejects non-http schemes"
call GET /api/users/me/
[[ $(field '.cover_image') == "$cover" ]]
call PATCH /api/users/me/ '{"cover_image":"https://images.unsplash.com/photo-1.jpg"}'
[[ $(field '.cover_image') == 'https://images.unsplash.com/photo-1.jpg' ]]
call PATCH /api/users/me/ '{"cover_image":null}'
[[ $(field '.cover_image') == null ]]
call PATCH /api/users/me/ '{"first_name":"Admin"}'
[[ $(field '.cover_image') == null ]]
call PATCH /api/users/me/ '{"user_timezone":"America/New_York"}'
[[ $(field '.user_timezone') == America/New_York ]]
call GET /api/users/me/profile/
# Plane's ProfileSerializer has no user_timezone; the timezone lives on the user.
[[ $(field '.user_timezone') == null ]]
call GET /api/users/me/
[[ $(field '.user_timezone') == America/New_York ]]
call PATCH /api/users/me/profile/ '{"user_timezone":"Asia/Tokyo"}'
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
