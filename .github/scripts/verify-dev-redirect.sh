#!/usr/bin/env bash
set -euo pipefail

expected_status=${1:-307}
case "$expected_status" in
    307|308) ;;
    *) echo 'Expected HTTP 307 or 308.' >&2; exit 1 ;;
esac

test_dir=$(mktemp -d)
table=shortlinks-dev
dev_url=https://dev-sc.jpro.dev
id=$(openssl rand -hex 8)
owner="verify-$id@example.test"
target='https://example.test/destination?keep=1&utm_source=stored#details'
slugs=()
owner_value=$(jq -nc --arg owner "$owner" '{":owner":{"S":$owner}}')

cleanup() {
    result=$?
    for slug in "${slugs[@]}"; do
        key=$(jq -nc --arg slug "$slug" '{slug:{S:$slug}}')
        aws dynamodb delete-item --table-name "$table" --key "$key" \
            --condition-expression 'created_by = :owner' \
            --expression-attribute-values "$owner_value" --no-cli-pager > /dev/null || result=1
    done
    rm -rf "$test_dir"
    exit "$result"
}
trap cleanup EXIT

put_fixture() {
    local slug=$1 delay=$2 item
    item=$(jq -nc --arg slug "$slug" --arg owner "$owner" --arg target "$target" \
        --arg created "$(date +%s)" --arg delay "$delay" '
        {slug:{S:$slug}, created_by:{S:$owner}, original_url:{S:$target},
         created_at:{N:$created}, is_active:{BOOL:true}, redirect_delay:{N:$delay}}')
    aws dynamodb put-item --table-name "$table" --item "$item" \
        --condition-expression 'attribute_not_exists(slug)' --no-cli-pager
    slugs+=("$slug")
}

put_fixture "verify-$id" 0
put_fixture "verify-$id-delay" 10

query='utm_source=verification&utm_medium=email&discard=1'
expected='https://example.test/destination?keep=1&utm_source=verification&utm_medium=email#details'
status=$(curl -sS --max-time 15 -D "$test_dir/headers" -o /dev/null -w '%{http_code}' \
    "$dev_url/verify-$id?$query")
[[ "$status" == "$expected_status" ]] || {
    echo "Ordinary redirect returned HTTP $status instead of $expected_status." >&2
    exit 1
}
tr -d '\r' < "$test_dir/headers" > "$test_dir/clean-headers"
grep -Fqix "location: $expected" "$test_dir/clean-headers"
if [[ "$expected_status" = 307 ]]; then
    grep -Fqix 'cache-control: no-store' "$test_dir/clean-headers"
fi

curl -fsS --max-time 15 "$dev_url/verify-$id-delay?$query" > "$test_dir/countdown"
encoded=$(printf '%s' "$expected" | sed 's/\&/\&amp;/g')
grep -Fq "href=\"$encoded\"" "$test_dir/countdown"
grep -Fq "const targetUrl = \"$expected\";" "$test_dir/countdown"
echo "Verified HTTP $expected_status redirect and both countdown destinations in dev."
