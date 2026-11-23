#!/usr/bin/env bash
set -euo pipefail

stage=$1
case "$stage" in
    dev|prod) ;;
    *) echo "Invalid deploy environment." >&2; exit 1 ;;
esac
cd "$2"

execution_role=()
if [[ -n ${SAM_EXECUTION_ROLE_ARN:-} ]]; then
    execution_role=(--role-arn "$SAM_EXECUTION_ROLE_ARN")
fi

sam deploy \
    --stack-name "url-shortener-$stage" \
    --template-file template.yaml \
    --resolve-s3 \
    --s3-prefix "url-shortener-$stage" \
    --capabilities CAPABILITY_IAM \
    --no-confirm-changeset \
    --no-fail-on-empty-changeset \
    "${execution_role[@]}" \
    --parameter-overrides \
        "StageName=$stage" \
        "AllowedDomain=$ALLOWED_DOMAIN" \
        "GoogleOAuthClientId=$GOOGLE_OAUTH_CLIENT_ID" \
        "ShortlinkDomain=$SHORTLINK_DOMAIN" \
        "CorsAllowOrigin=$CORS_ALLOW_ORIGIN" \
        "CustomDomainName=$CUSTOM_DOMAIN_NAME" \
        "CustomDomainCertificateArn=$CUSTOM_DOMAIN_CERT_ARN" \
        "AdminEmails=$ADMIN_EMAILS"
