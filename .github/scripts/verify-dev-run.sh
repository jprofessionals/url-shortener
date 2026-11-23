#!/usr/bin/env bash
set -euo pipefail

workflow=$1
run_id=${2:-}
repo=${GITHUB_REPOSITORY:?}
runs="repos/$repo/actions/runs"

if [[ -z $run_id ]]; then
    run_id=$(gh api "repos/$repo/actions/workflows/$workflow/runs?branch=main&status=success&per_page=1" \
        --jq '.workflow_runs[0].id // empty')
fi
if [[ ! $run_id =~ ^[0-9]+$ ]]; then
    echo 'Expected the numeric ID of a successful dev deployment run.' >&2
    exit 1
fi

run=$(gh api "$runs/$run_id")
sha=$(jq -er --arg repo "$repo" --arg path ".github/workflows/$workflow" '
    select(.path == $path and .repository.full_name == $repo
        and .head_repository.full_name == $repo and .head_branch == "main"
        and (.event == "push" or .event == "workflow_dispatch")
        and .conclusion == "success")
    | .head_sha | select(test("^[0-9a-f]{40}$"))
' <<< "$run") || {
    echo "Run $run_id is not a successful $workflow run from main." >&2
    exit 1
}

gh api "$runs/$run_id/jobs?filter=latest&per_page=100" |
    jq -e 'any(.jobs[]; .name == "deploy-dev" and .conclusion == "success")' > /dev/null || {
    echo "Run $run_id did not deploy to dev successfully." >&2
    exit 1
}

printf 'run_id=%s\nsha=%s\n' "$run_id" "$sha" >> "${GITHUB_OUTPUT:?}"
printf 'Verified dev deployment %s for commit %s\n' "$run_id" "$sha"
