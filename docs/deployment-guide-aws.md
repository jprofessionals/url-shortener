# AWS Deployment Guide

This guide walks through deploying the URL shortener to AWS using SAM (Serverless Application Model).

## Architecture Overview

```
┌─────────────────────────────────────────────────────────────────┐
│                        API Gateway (HTTP API)                    │
│                                                                  │
│  GET /{slug}          GET /api/links         POST /api/links    │
│       │                     │                      │             │
└───────┼─────────────────────┼──────────────────────┼─────────────┘
        │                     │                      │
        ▼                     └──────────┬───────────┘
┌───────────────┐                        ▼
│lambda-redirect│              ┌──────────────────┐
│   (public)    │              │  lambda-admin    │
└───────┬───────┘              │ (Google auth)    │
        │                      └────────┬─────────┘
        │                               │
        ▼                               ▼
┌───────────────────────────────────────────────────┐
│                    DynamoDB                        │
│  ┌─────────────────┐    ┌─────────────────┐       │
│  │ shortlinks-{env}│    │ counters-{env}  │       │
│  └─────────────────┘    └─────────────────┘       │
└───────────────────────────────────────────────────┘
```

## Environments

| Environment | Short URL (API) | Admin UI | Region |
|-------------|-----------------|----------|--------|
| dev | `dev-sc.jpro.dev` | `dev-admin-sc.jpro.dev` | eu-north-1 |
| prod | `sc.jpro.dev` | `admin-sc.jpro.dev` | eu-north-1 |

## Prerequisites

### 1. AWS Account & CLI

```bash
# Install AWS CLI (if not using Nix flake)
# macOS: brew install awscli
# Linux: https://docs.aws.amazon.com/cli/latest/userguide/getting-started-install.html

# Configure credentials
aws configure
# Enter: Access Key ID, Secret Access Key, Region (e.g., eu-north-1), Output format (json)

# Verify
aws sts get-caller-identity
```

### 2. AWS SAM CLI

```bash
# If using Nix flake (recommended):
nix develop
# SAM CLI is automatically installed in .venv/

# Or install manually:
uv pip install aws-sam-cli

# Verify
sam --version
```

### 3. Rust Build Target

```bash
# Add the Linux target for cross-compilation
rustup target add x86_64-unknown-linux-gnu

# On macOS, you'll need a linker. Install via:
# brew install filosottile/musl-cross/musl-cross
# Or use cargo-zigbuild / cargo-lambda
```

### 4. Google OAuth Client ID

1. Go to [Google Cloud Console](https://console.cloud.google.com/)
2. Create a new project or select existing
3. Navigate to **APIs & Services → Credentials**
4. Click **Create Credentials → OAuth client ID**
5. Select **Web application**
6. Add authorized JavaScript origins:
   - `http://localhost:8000` (for local dev)
   - `https://admin.yourdomain.com` (for production)
7. Copy the **Client ID** (looks like: `123456789012-abc...xyz.apps.googleusercontent.com`)

## Deployment Steps

### Step 1: Build Lambda Artifacts

```bash
# From project root
make build-lambdas

# This creates:
# - infra/sam/artifacts/lambda-redirect/bootstrap
# - infra/sam/artifacts/lambda-admin/bootstrap
```

Verify the artifacts exist:
```bash
ls -la infra/sam/artifacts/*/bootstrap
```

### Step 2: Validate SAM Template

```bash
sam validate -t infra/sam/template.yaml
```

### Step 3: Deploy with SAM

For first-time deployment, use `--guided` to set parameters:

```bash
cd infra/sam

sam deploy --guided \
  --stack-name url-shortener-dev \
  --capabilities CAPABILITY_IAM
```

SAM will prompt for:

| Parameter | Description | Example |
|-----------|-------------|---------|
| `StageName` | Environment name | `dev`, `staging`, `prod` |
| `AllowedDomain` | Google Workspace domain | `yourcompany.com` |
| `GoogleOAuthClientId` | OAuth client ID | `123...apps.googleusercontent.com` |
| `ShortlinkDomain` | Base URL for short links | `https://go.yourcompany.com` |
| `CorsAllowOrigin` | Admin UI origin | `https://admin.yourcompany.com` |

**Example session (dev environment):**
```
Setting default arguments for 'sam deploy'
=========================================
Stack Name [url-shortener-dev]:
AWS Region [eu-north-1]:
Parameter StageName [dev]:
Parameter AllowedDomain: jpro.no
Parameter GoogleOAuthClientId: 123456789-abc.apps.googleusercontent.com
Parameter ShortlinkDomain: https://dev-sc.jpro.dev
Parameter CorsAllowOrigin: http://localhost:8000
Confirm changes before deploy [Y/n]: Y
Allow SAM CLI IAM role creation [Y/n]: Y
Save arguments to configuration file [Y/n]: Y
```

### Step 4: Note the Outputs

After deployment, SAM shows outputs:

```
CloudFormation outputs from deployed stack
------------------------------------------
Key                 ApiEndpoint
Value               https://abc123xyz.execute-api.eu-north-1.amazonaws.com/dev/

Key                 ShortlinksTableOut
Value               shortlinks-dev

Key                 CountersTableOut
Value               counters-dev
```

**Save the `ApiEndpoint`** — this is your API base URL.

### Step 5: Test the Deployment

```bash
# Set your API endpoint
API_URL="https://abc123xyz.execute-api.eu-north-1.amazonaws.com/dev"

# Test redirect (should return 404 since no links exist)
curl -i "$API_URL/test123"

# Test admin API (requires Google auth token)
# For now, this will return 401 Unauthorized
curl -i "$API_URL/api/links"
```

## Configuration File (samconfig.toml)

After `--guided` deployment, SAM creates `samconfig.toml`:

```toml
version = 0.1

[default.deploy.parameters]
stack_name = "url-shortener-dev"
resolve_s3 = true
s3_prefix = "url-shortener-dev"
region = "eu-north-1"
capabilities = "CAPABILITY_IAM"
parameter_overrides = "StageName=\"dev\" AllowedDomain=\"yourcompany.com\" GoogleOAuthClientId=\"123...\" ShortlinkDomain=\"https://go.yourcompany.com\" CorsAllowOrigin=\"https://admin.yourcompany.com\""
```

For subsequent deployments, just run:
```bash
sam deploy
```

## Multiple Environments

The `samconfig.toml` supports multiple environments. Copy the example and fill in your Google OAuth Client ID:

```bash
cp infra/sam/samconfig.toml.example infra/sam/samconfig.toml
# Edit samconfig.toml - replace REPLACE_WITH_YOUR_CLIENT_ID
```

### Deploy Dev (default)
```bash
cd infra/sam
sam deploy
```

### Deploy Prod
```bash
cd infra/sam
sam deploy --config-env prod
```

### What Each Environment Creates

| Resource         | Dev                              | Prod                               |
|------------------|----------------------------------|------------------------------------|
| Stack name       | `url-shortener-dev`              | `url-shortener-prod`               |
| DynamoDB tables  | `shortlinks-dev`, `counters-dev` | `shortlinks-prod`, `counters-prod` |
| Short URL (API)  | `dev-sc.jpro.dev`                | `sc.jpro.dev`                      |
| Admin UI (CORS)  | `dev-admin-sc.jpro.dev`          | `admin-sc.jpro.dev`                |
| Lambda functions | `url-shortener-*-dev`            | `url-shortener-*-prod`             |

## Custom Domain Setup (Cloudflare DNS)

This guide assumes DNS is managed in Cloudflare. Adjust for other providers as needed.

### Secrets and Non-Secrets

| Item                                                   | Secret?                         | Where to Store                                              |
|--------------------------------------------------------|---------------------------------|-------------------------------------------------------------|
| ACM Certificate ARN                                    | No                              | Can commit to docs/notes                                    |
| API Gateway API ID                                     | No                              | Visible in AWS console, SAM outputs                         |
| API Gateway domain name (e.g., `d-xxx.execute-api...`) | No                              | Public endpoint                                             |
| AWS Account ID                                         | No (but avoid sharing publicly) | Keep in private notes                                       |
| Google OAuth Client ID                                 | No                              | Already in `config.js` and `samconfig.toml.example`         |
| Google OAuth Client Secret                             | **YES**                         | Never commit; not needed for this app (frontend-only OAuth) |
| AWS Access Keys                                        | **YES**                         | Never commit; use `aws configure` or env vars               |

### Step 1: Request Wildcard ACM Certificate

Use a **wildcard certificate** to cover all subdomains with a single cert:

```bash
aws acm request-certificate \
  --domain-name "*.jpro.dev" \
  --validation-method DNS \
  --region eu-north-1
```

This single certificate covers all subdomains:
- `sc.jpro.dev` (prod API)
- `dev-sc.jpro.dev` (dev API)
- `admin-sc.jpro.dev` (prod admin UI)
- `dev-admin-sc.jpro.dev` (dev admin UI)
- Any future subdomains

Save the `CertificateArn` from the output:

| Domain        | ARN                                                                                    |
|---------------|----------------------------------------------------------------------------------------|
| `*.jpro.dev`  | `arn:aws:acm:eu-north-1:607433350488:certificate/2fefeaed-0d05-4ced-a182-c3099ffc1414` |

### Step 2: Get DNS Validation Record

```bash
aws acm describe-certificate \
  --certificate-arn "arn:aws:acm:eu-north-1:607433350488:certificate/2fefeaed-0d05-4ced-a182-c3099ffc1414" \
  --region eu-north-1 \
  --query 'Certificate.DomainValidationOptions[0].ResourceRecord'
```

Output:
```json
{
   "Name": "_eb93e53e7a0468ded54530135a302757.jpro.dev.",
   "Type": "CNAME",
   "Value": "_a525a08ab7ba22b66fdf20d2f879458d.jkddzztszm.acm-validations.aws."
}
```

### Step 3: Add Validation Record in Cloudflare

1. Go to Cloudflare Dashboard → DNS → Records
2. Click "Add record"
3. Set:
   - **Type**: CNAME
   - **Name**: The `Name` value without trailing dot and without `.jpro.dev`
     - e.g., if Name is `_a1b2c3d4e5.jpro.dev.`, enter just `_a1b2c3d4e5`
   - **Target**: The `Value` from above (e.g., `_x9y8z7.acm-validations.aws.`)
   - **Proxy status**: **DNS only** (grey cloud) — important!
4. Save

### Step 4: Wait for Certificate Validation

Check status (wait 2-5 minutes):
```bash
aws acm describe-certificate \
    --certificate-arn "arn:aws:acm:eu-north-1:607433350488:certificate/2fefeaed-0d05-4ced-a182-c3099ffc1414" \
    --region eu-north-1 \
    --query 'Certificate.Status'
```

Should return `"ISSUED"` when ready.

### Step 5: Deploy with Custom Domain (Automated)

If you have `CustomDomainName` and `CustomDomainCertificateArn` set in `samconfig.toml`, SAM automatically creates the API Gateway custom domain and API mapping during deploy:

```bash
cd infra/sam
sam deploy                    # Dev
sam deploy --config-env prod  # Prod
```

After deploy, note the `CustomDomainTarget` from the outputs:

| Domain            | CustomDomainTarget (from SAM output)                 |
|-------------------|------------------------------------------------------|
| `dev-sc.jpro.dev` | `d-nvmqz6pv2m.execute-api.eu-north-1.amazonaws.com`  |
| `sc.jpro.dev`     | `d-tfvnfjog12.execute-api.eu-north-1.amazonaws.com`  |

### Step 6: Add DNS Records in Cloudflare

Add CNAME records for each domain:

| Type  | Name     | Target (CustomDomainTarget from SAM output)         | Proxy Status |
|-------|----------|-----------------------------------------------------|--------------|
| CNAME | `dev-sc` | `d-nvmqz6pv2m.execute-api.eu-north-1.amazonaws.com` | DNS only     |
| CNAME | `sc`     | `d-tfvnfjog12.execute-api.eu-north-1.amazonaws.com` | DNS only     |

**Important:** Use "DNS only" (grey cloud) for API Gateway domains.

### Step 7: Test

```bash
# Dev - should return 404 (no links exist)
curl -i https://dev-sc.jpro.dev/test123

# Dev - should return 401 (no auth)
curl -i https://dev-sc.jpro.dev/api/links

# Prod (after deploying prod stack)
curl -i https://sc.jpro.dev/test123
```

## Admin Frontend Deployment (Cloudflare Pages)

The admin frontend (`admin-frontend/`) can be deployed to Cloudflare Pages:

### Step 1: Update config.js for Production

Create environment-specific configs or update before deploy:
```javascript
window.APP_CONFIG = {
  API_BASE: "https://dev-sc.jpro.dev",  // or https://sc.jpro.dev for prod
  GOOGLE_CLIENT_ID: "333449424444-bb173lfcpqurosj5o2b39lmkpovnceqi.apps.googleusercontent.com",
  AUTH_DISABLED: false
};
```

### Step 2: Deploy to Cloudflare Pages

**Option A: Cloudflare Dashboard**
1. Go to Cloudflare Dashboard → Workers & Pages → Create
2. Select "Pages" → "Upload assets"
3. Upload the contents of `admin-frontend/`
4. Set custom domain: `dev-admin-sc.jpro.dev`

**Option B: Wrangler CLI**
```bash
cd admin-frontend
npx wrangler pages deploy . --project-name=url-shortener-admin-dev
# Then add custom domain in dashboard
```

### Step 3: Configure Custom Domain

In Cloudflare Pages project settings:
1. Go to Custom domains
2. Add `dev-admin-sc.jpro.dev`
3. Cloudflare automatically configures DNS if on same account

## Monitoring & Logs

### View Lambda Logs

```bash
# Redirect function logs
sam logs -n url-shortener-redirect-dev --tail

# Admin function logs
sam logs -n url-shortener-admin-dev --tail
```

### CloudWatch Insights Query

```sql
fields @timestamp, @message
| filter @message like /error/
| sort @timestamp desc
| limit 100
```

## Cleanup

To delete the stack (preserves DynamoDB tables due to DeletionPolicy: Retain):

```bash
sam delete --stack-name url-shortener-dev
```

To fully delete including tables:
```bash
# First, update template to remove DeletionPolicy: Retain, then:
sam delete --stack-name url-shortener-dev

# Or manually:
aws dynamodb delete-table --table-name shortlinks-dev
aws dynamodb delete-table --table-name counters-dev
```

## Troubleshooting

### Build Fails on macOS

If cross-compilation fails, use `cargo-zigbuild`:
```bash
cargo install cargo-zigbuild
cargo zigbuild --release --target x86_64-unknown-linux-gnu -p lambda-redirect -p lambda-admin
```

### Lambda Timeout

Increase timeout in `template.yaml`:
```yaml
Globals:
  Function:
    Timeout: 10  # Increase from 5
```

### CORS Errors

Verify `CorsAllowOrigin` matches your admin UI domain exactly (including `https://`).

### Auth Failures

1. Verify `GoogleOAuthClientId` matches the one in your admin frontend
2. Verify `AllowedDomain` matches your Google Workspace domain
3. Check Lambda logs for specific error messages

## Next Steps

1. **Admin Frontend**: Deploy the admin UI to S3/CloudFront or Cloudflare Pages
2. **Custom Domain**: Set up Route53 + ACM for your short URL domain
3. **Monitoring**: Add CloudWatch alarms for errors and latency
4. **CI/CD**: Add `sam deploy` to GitHub Actions workflow
