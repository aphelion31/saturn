# File uploads on local disk (src/assets.rs): presign, multipart upload, mark uploaded, download.
call POST "/api/assets/v2/workspaces/$SLUG/projects/$PROJECT/" '{"name":"smoke.txt","type":"text/plain","size":12,"entity_type":"ISSUE_DESCRIPTION"}'
ASSET=$(field '.asset_id')
TOKEN=$(field '.upload_data.fields.token')
code=$(curl -s -o /dev/null -w '%{http_code}' -F "token=$TOKEN" -F "file=@-;filename=smoke.txt" "$BASE/api/assets/v2/upload/$ASSET/" <<< "hello saturn")
[[ "$code" == 204 ]] || { echo "✖ upload -> $code" >&2; exit 1; }
echo "✔ POST /api/assets/v2/upload/{id}/ -> $code"
code=$(curl -s -o /dev/null -w '%{http_code}' -F "token=bad" -F "file=@-;filename=x.txt" "$BASE/api/assets/v2/upload/$ASSET/" <<< "x")
[[ "$code" == 400 || "$code" == 403 ]] || { echo "✖ bad token upload -> $code" >&2; exit 1; }
echo "✔ bad token rejected -> $code"
call PATCH "/api/assets/v2/workspaces/$SLUG/projects/$PROJECT/$ASSET/" '{}'
got=$(curl -s -b "$jar" "$BASE/api/assets/v2/workspaces/$SLUG/projects/$PROJECT/$ASSET/")
[[ "$got" == "hello saturn" ]] || { echo "✖ download returned: $got" >&2; exit 1; }
echo "✔ download round trip"
call DELETE "/api/assets/v2/workspaces/$SLUG/projects/$PROJECT/$ASSET/"
