# Public API v1 archive round-trip and work-item attachments (src/api_v1.rs, src/api_v1_items.rs).
# Reuses call_v1/field/V1_TOKEN from 97-api-v1.sh; sorts after 97/98/99.
V1_PROJ="/api/v1/workspaces/$SLUG/projects/$PROJECT"
call_v1 POST "$V1_PROJ/cycles/" '{"name":"smoke past cycle","start_date":"2020-01-01","end_date":"2020-01-10"}'
V1_CYCLE=$(field '.id')
call_v1 POST "$V1_PROJ/cycles/$V1_CYCLE/archive/" '{}'
call_v1 GET "$V1_PROJ/archived-cycles/"
jq -e --arg id "$V1_CYCLE" '.results | any(.id == $id)' <<<"$body" >/dev/null || { echo "✖ archived cycle missing" >&2; exit 1; }
call_v1 DELETE "$V1_PROJ/archived-cycles/$V1_CYCLE/unarchive/"
call_v1 POST "$V1_PROJ/cycles/" '{"name":"smoke future cycle","start_date":"2099-01-01","end_date":"2099-01-10"}'
FUTURE_CYCLE=$(field '.id')
c=$(curl -s -o /dev/null -w '%{http_code}' -H "X-Api-Key: $V1_TOKEN" -X POST "$BASE$V1_PROJ/cycles/$FUTURE_CYCLE/archive/")
[[ "$c" == 400 ]] || { echo "✖ archive future cycle -> $c" >&2; exit 1; }

call_v1 POST "$V1_PROJ/modules/" '{"name":"smoke done module","status":"completed"}'
V1_MODULE=$(field '.id')
call_v1 POST "$V1_PROJ/modules/$V1_MODULE/archive/" '{}'
call_v1 DELETE "$V1_PROJ/archived-modules/$V1_MODULE/unarchive/"

V1_ITEM_PATH="$V1_PROJ/work-items"
call_v1 POST "$V1_ITEM_PATH/" '{"name":"smoke attachment host"}'
ATT_ITEM=$(field '.id')
call_v1 POST "$V1_ITEM_PATH/$ATT_ITEM/attachments/" '{"name":"spec.pdf","type":"application/pdf","size":"1024","external_id":"smoke-ext-1","external_source":"smoke"}'
ATT_ASSET=$(field '.asset_id')
jq -e '.upload_data.url and .attachment.id and .asset_url' <<<"$body" >/dev/null || { echo "✖ attachment presign shape" >&2; exit 1; }
c=$(curl -s -o /dev/null -w '%{http_code}' -H "X-Api-Key: $V1_TOKEN" "$BASE$V1_ITEM_PATH/$ATT_ITEM/attachments/$ATT_ASSET/")
[[ "$c" == 400 ]] || { echo "✖ unuploaded attachment GET -> $c" >&2; exit 1; }
c=$(curl -s -o /dev/null -w '%{http_code}' -H "X-Api-Key: $V1_TOKEN" -H 'content-type: application/json' -d '{"name":"spec.pdf","type":"application/pdf","size":10,"external_id":"smoke-ext-1","external_source":"smoke"}' "$BASE$V1_ITEM_PATH/$ATT_ITEM/attachments/")
[[ "$c" == 409 ]] || { echo "✖ duplicate external_id -> $c" >&2; exit 1; }
c=$(curl -s -o /dev/null -w '%{http_code}' -H "X-Api-Key: $V1_TOKEN" -H 'content-type: application/json' -d '{"name":"x.exe","type":"application/x-msdownload","size":10}' "$BASE$V1_ITEM_PATH/$ATT_ITEM/attachments/")
[[ "$c" == 400 ]] || { echo "✖ bad mime -> $c" >&2; exit 1; }
call_v1 PATCH "$V1_ITEM_PATH/$ATT_ITEM/attachments/$ATT_ASSET/" '{}'
call_v1 GET "$V1_ITEM_PATH/$ATT_ITEM/attachments/$ATT_ASSET/"
jq -e --arg id "$ATT_ASSET" '.id == $id and .attributes.name == "spec.pdf"' <<<"$body" >/dev/null || { echo "✖ attachment GET shape" >&2; exit 1; }
call_v1 GET "$V1_ITEM_PATH/$ATT_ITEM/attachments/"
jq -e --arg id "$ATT_ASSET" 'any(.[]; .id == $id)' <<<"$body" >/dev/null || { echo "✖ uploaded attachment missing from list" >&2; exit 1; }
call_v1 POST "/api/v1/workspaces/$SLUG/projects/$PROJECT/issues/$ATT_ITEM/issue-attachments/" '{"name":"alias.txt","type":"text/plain","size":3}'
ALIAS_ASSET=$(field '.asset_id')
call_v1 PATCH "/api/v1/workspaces/$SLUG/projects/$PROJECT/issues/$ATT_ITEM/issue-attachments/$ALIAS_ASSET/" '{}'
call_v1 DELETE "/api/v1/workspaces/$SLUG/projects/$PROJECT/issues/$ATT_ITEM/issue-attachments/$ALIAS_ASSET/"
c=$(curl -s -o /dev/null -w '%{http_code}' -H "X-Api-Key: $V1_TOKEN" "$BASE/api/v1/workspaces/$SLUG/projects/$PROJECT/modules/00000000-0000-0000-0000-000000000000/module-issues/$ATT_ITEM/")
[[ "$c" == 405 ]] || { echo "✖ module-issues GET should stay 405 (Plane), got $c" >&2; exit 1; }
