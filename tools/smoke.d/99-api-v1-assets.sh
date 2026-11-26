# Public API v1 assets and schema (src/api_v1_assets.rs). Reuses call_v1/V1_TOKEN from 97-api-v1.sh.
call_v1 POST /api/v1/assets/user-assets/ '{"name":"avatar.png","type":"image/png","size":4,"entity_type":"USER_AVATAR"}'
V1_ASSET=$(field '.asset_id')
V1_UPLOAD=$(field '.upload_data.url')
V1_UPLOAD_TOKEN=$(field '.upload_data.fields.token')
code=$(curl -s -o /dev/null -w '%{http_code}' -F "token=$V1_UPLOAD_TOKEN" -F 'file=@-;filename=avatar.png;type=image/png' "$BASE$V1_UPLOAD" <<<"test")
[[ "$code" == 204 ]] || { echo "✖ asset upload -> $code" >&2; exit 1; }
call_v1 PATCH "/api/v1/assets/user-assets/$V1_ASSET/" '{}'
call_v1 DELETE "/api/v1/assets/user-assets/$V1_ASSET/"
call_v1 POST /api/v1/assets/user-assets/server/ '{"name":"cover.png","type":"image/png","size":4,"entity_type":"USER_COVER"}'
V1_ASSET=$(field '.asset_id')
call_v1 PATCH "/api/v1/assets/user-assets/$V1_ASSET/server/" '{}'
call_v1 DELETE "/api/v1/assets/user-assets/$V1_ASSET/server/"
call_v1 POST "/api/v1/workspaces/$SLUG/assets/" '{"name":"doc.txt","type":"text/plain","size":4,"project_id":"'"$PROJECT"'"}'
V1_ASSET=$(field '.asset_id')
call_v1 PATCH "/api/v1/workspaces/$SLUG/assets/$V1_ASSET/" '{"is_uploaded":true}'
call_v1 GET "/api/v1/workspaces/$SLUG/assets/$V1_ASSET/"
[[ $(field '.asset_name') == doc.txt ]] || exit 1
code=$(curl -s -o /dev/null -w '%{http_code}' "$BASE/api/v1/schema/")
[[ "$code" == 404 || "$code" == 200 ]] || { echo "✖ schema -> $code" >&2; exit 1; }
