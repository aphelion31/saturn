# Public API v1 gaps: workspace invitations and stickies (src/api_v1_extra.rs).
# Reuses call_v1/field/V1_TOKEN from 97-api-v1.sh.
V1_WS="/api/v1/workspaces/$SLUG"

# 1. Workspace invitations (admin only)
GAP_EMAIL="gap$RANDOM$RANDOM@example.com"
call_v1 POST "$V1_WS/invitations/" '{"email":"'"$GAP_EMAIL"'","role":5}'
GAP_INVITE=$(field '.id')
jq -e '.email and .role == 5 and .accepted == false' <<<"$body" >/dev/null || { echo "✖ invite create shape" >&2; exit 1; }
c=$(curl -s -o /dev/null -w '%{http_code}' -H "X-Api-Key: $V1_TOKEN" -H 'content-type: application/json' -d '{"email":"'"$GAP_EMAIL"'"}' "$BASE$V1_WS/invitations/")
[[ "$c" == 400 ]] || { echo "✖ duplicate invite -> $c" >&2; exit 1; }
call_v1 GET "$V1_WS/invitations/"
jq -e --arg id "$GAP_INVITE" 'any(.[]; .id == $id)' <<<"$body" >/dev/null || { echo "✖ invite missing from list" >&2; exit 1; }
call_v1 GET "$V1_WS/invitations/$GAP_INVITE/"
call_v1 PATCH "$V1_WS/invitations/$GAP_INVITE/" '{"role":15}'
jq -e '.role == 15' <<<"$body" >/dev/null || { echo "✖ invite role patch" >&2; exit 1; }
c=$(curl -s -o /dev/null -w '%{http_code}' -H "X-Api-Key: $V1_TOKEN" -H 'content-type: application/json' -X PATCH -d '{"email":"other@example.com"}' "$BASE$V1_WS/invitations/$GAP_INVITE/")
[[ "$c" == 400 ]] || { echo "✖ invite email patch -> $c" >&2; exit 1; }
c=$(curl -s -o /dev/null -w '%{http_code}' -H "X-Api-Key: $V1_TOKEN" -H 'content-type: application/json' -X PUT -d '{"role":15}' "$BASE$V1_WS/invitations/$GAP_INVITE/")
[[ "$c" == 405 ]] || { echo "✖ invite PUT -> $c (Plane answers 405)" >&2; exit 1; }
call_v1 DELETE "$V1_WS/invitations/$GAP_INVITE/"
c=$(curl -s -o /dev/null -w '%{http_code}' -H "X-Api-Key: $V1_TOKEN" "$BASE$V1_WS/invitations/$GAP_INVITE/")
[[ "$c" == 404 ]] || { echo "✖ deleted invite GET -> $c" >&2; exit 1; }

# 2. Stickies (owner-scoped)
call_v1 POST "$V1_WS/stickies/" '{"name":"gap sticky","description_html":"<p>gap <script>x</script>note</p>","color":"#fff"}'
GAP_STICKY=$(field '.id')
jq -e '.description_stripped | contains("gap") and (contains("script") | not)' <<<"$body" >/dev/null || { echo "✖ sticky sanitize/strip" >&2; exit 1; }
call_v1 GET "$V1_WS/stickies/$GAP_STICKY/"
call_v1 PATCH "$V1_WS/stickies/$GAP_STICKY/" '{"name":"gap sticky renamed","description_html":"<p>changed</p>"}'
jq -e '.name == "gap sticky renamed" and .description_stripped == "changed"' <<<"$body" >/dev/null || { echo "✖ sticky patch" >&2; exit 1; }
call_v1 GET "$V1_WS/stickies/?query=changed"
jq -e --arg id "$GAP_STICKY" '.results | any(.id == $id)' <<<"$body" >/dev/null || { echo "✖ sticky search" >&2; exit 1; }
call_v1 DELETE "$V1_WS/stickies/$GAP_STICKY/"
c=$(curl -s -o /dev/null -w '%{http_code}' -H "X-Api-Key: $V1_TOKEN" "$BASE$V1_WS/stickies/$GAP_STICKY/")
[[ "$c" == 404 ]] || { echo "✖ deleted sticky GET -> $c" >&2; exit 1; }
