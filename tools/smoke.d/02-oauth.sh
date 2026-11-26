# --- oauth ---
for provider in google github gitlab gitea; do
  for prefix in /auth /auth/spaces; do
    response=$(curl -s -o /dev/null -w '%{http_code} %{redirect_url}' "$BASE$prefix/$provider/callback/?state=invalid&code=invalid")
    [[ "$response" == 302*error_code=* ]] || { echo "✖ OAuth state $prefix/$provider: $response" >&2; exit 1; }
    echo "✔ OAuth state $prefix/$provider -> 302"
  done
done
