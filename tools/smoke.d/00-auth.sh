# Authentication smoke assertions (src/auth.rs)

# Plane #9672: sign-in with no User-Agent header must succeed, not 500
code=$(curl -s -o /dev/null -w '%{http_code}' -H 'User-Agent:' -d "email=$EMAIL&password=$PASSWORD" "$BASE/auth/sign-in/")
[[ "$code" == 302 ]] || { echo "✖ sign-in with empty User-Agent -> $code (expected 302)" >&2; exit 1; }
echo "✔ sign-in with empty User-Agent -> $code"
