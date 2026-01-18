### Phase 11 — Infrastructure as Code (IaC) Implementation Plan

Purpose
- Define AWS infrastructure for the serverless URL shortener so the two Lambda apps from Phase 10 can be deployed consistently. Keep scope tight (2–4 h): author and validate IaC; actual deployment and packaging can follow in a later phase.

Primary choice (recommended)
- AWS SAM (Serverless Application Model) with a single `template.yaml` in `infra/sam/`.
  - Pros: purpose-built for Lambda + API Gateway; fast authoring; `sam validate` for acceptance; easy to evolve to `sam build`/`sam deploy` later.
  - Cons: AWS‑specific (acceptable for this project).

Alternatives (documented, not implemented in Phase 11)
- Terraform: place in `infra/tf/`, keep module split by `api`, `dynamodb`, `lambda`.
- AWS CDK (TypeScript): place in `infra/cdk/`, synth to CloudFormation.

Outcomes
- New directory `infra/sam/` with a validated `template.yaml` describing:
  - DynamoDB tables: `ShortLinks`, `Counters` (PAY_PER_REQUEST, on‑demand capacity).
  - Two Lambda functions: `lambda-redirect`, `lambda-admin`.
  - API Gateway HTTP API with routes:
    - GET `/{slug}` → `lambda-redirect`
    - GET `/api/links` → `lambda-admin`
    - POST `/api/links` → `lambda-admin`
  - IAM role/policies with least privilege to the specific table ARNs.
  - Function environment wired to existing app expectations.
- Makefile targets to run `sam validate` and basic lint.

Acceptance
- `sam validate` succeeds locally without deployment.

— — —

Step‑by‑step plan

1) Create folder layout
- Create `infra/sam/`.
- Add `README.md` with short how‑to (optional in Phase 11; can be added in Phase 12 Docs & DX).

2) Author base SAM template
- File: `infra/sam/template.yaml`
- Globals:
  - Runtime: `provided.al2` (compatible with Rust custom runtime) or `provided.al2023`.
  - Architecture: `x86_64` (switch to `arm64` later if desired).
  - Tracing: disabled for now; enable in later perf/ops phase.
- Parameters:
  - `StageName` (Default: `dev`).
  - `AllowedDomain` (string, no default).
  - `GoogleOAuthClientId` (string, no default).
  - `ShortlinksTableName` (Default: `shortlinks-${StageName}`).
  - `CountersTableName` (Default: `counters-${StageName}`).
  - `ApiName` (Default: `url-shortener-${StageName}`).

3) Define DynamoDB tables (least‑surprise schema)
- `ShortLinks` table:
  - Partition key: `slug` (String).
  - PAY_PER_REQUEST billing.
  - TTL: none for now (future stretch goal).
- `Counters` table:
  - Partition key: `name` (String).
  - PAY_PER_REQUEST billing.

4) Define IAM policies (least privilege)
- Create execution role policies for each function with access only to the two table ARNs (no wildcards beyond the table):
  - `dynamodb:GetItem`, `PutItem`, `Query`, `UpdateItem`, `Scan` (limit to table ARNs).
  - `logs:CreateLogGroup`, `logs:CreateLogStream`, `logs:PutLogEvents` for function logs.

5) Define Lambda functions
- `RedirectFunction` (apps/lambda-redirect):
  - Handler: set to `bootstrap` (custom runtime) with a Zip artifact produced later; in Phase 11 we do not build or deploy, only validate template schema.
  - Environment:
    - `DYNAMO_TABLE_SHORTLINKS` = Ref `ShortlinksTable` name
    - `DYNAMO_TABLE_COUNTERS` = Ref `CountersTable` name
- `AdminFunction` (apps/lambda-admin):
  - Environment:
    - `DYNAMO_TABLE_SHORTLINKS` = Ref `ShortlinksTable`
    - `DYNAMO_TABLE_COUNTERS` = Ref `CountersTable`
    - `GOOGLE_OAUTH_CLIENT_ID` = Param `GoogleOAuthClientId`
    - `ALLOWED_DOMAIN` = Param `AllowedDomain`
    - `GOOGLE_AUTH_INSECURE_SKIP_SIGNATURE` = optional, default empty

6) Define HTTP API and routes
- Create an `AWS::Serverless::HttpApi` (HTTP API v2) named from `ApiName`.
- Add `Events` on functions:
  - `RedirectFunction` → route key `GET /{slug}` (simple; if we want to support nested slugs later, change to `GET /{slug+}`).
  - `AdminFunction` → route keys `GET /api/links` and `POST /api/links`.
- CORS: allow `ADMIN_ORIGIN` is managed at app layer today; for API we can enable permissive dev CORS or skip in Phase 11.

7) Outputs
- Output API base URL, table names, and function ARNs.

8) Local validation wiring
- Add a top‑level `Makefile` snippet or `infra/sam/Makefile` with:
  - `iac-validate`: `sam validate -t infra/sam/template.yaml`
- Document minimal prerequisites: `pipx install aws-sam-cli`.

9) Optional stubs for future packaging (document only in this phase)
- Note how `sam build` will pick up function code:
  - For Rust, we typically provide a prebuilt `bootstrap` in `target/lambda/<fn-name>/bootstrap` and point `CodeUri` to that folder in the template, or use `Metadata BuildMethod: makefile` with `Makefile` targets per function to compile using `cargo`/`cargo-lambda`.
  - This repo will defer to a later phase (deployment) to add those Makefile build hooks.

— — —

Reference SAM template (validated structure)

```yaml
AWSTemplateFormatVersion: '2010-09-09'
Transform: AWS::Serverless-2016-10-31
Description: URL Shortener — Lambdas, API, and DynamoDB

Parameters:
  StageName:
    Type: String
    Default: dev
  AllowedDomain:
    Type: String
  GoogleOAuthClientId:
    Type: String
  ShortlinksTableName:
    Type: String
    Default: !Sub 'shortlinks-${StageName}'
  CountersTableName:
    Type: String
    Default: !Sub 'counters-${StageName}'
  ApiName:
    Type: String
    Default: !Sub 'url-shortener-${StageName}'

Globals:
  Function:
    Runtime: provided.al2
    Architectures: [x86_64]
    Timeout: 5
    MemorySize: 128
    Tracing: Disabled

Resources:
  ShortlinksTable:
    Type: AWS::DynamoDB::Table
    Properties:
      TableName: !Ref ShortlinksTableName
      BillingMode: PAY_PER_REQUEST
      AttributeDefinitions:
        - AttributeName: slug
          AttributeType: S
      KeySchema:
        - AttributeName: slug
          KeyType: HASH

  CountersTable:
    Type: AWS::DynamoDB::Table
    Properties:
      TableName: !Ref CountersTableName
      BillingMode: PAY_PER_REQUEST
      AttributeDefinitions:
        - AttributeName: name
          AttributeType: S
      KeySchema:
        - AttributeName: name
          KeyType: HASH

  HttpApi:
    Type: AWS::Serverless::HttpApi
    Properties:
      Name: !Ref ApiName

  RedirectFunction:
    Type: AWS::Serverless::Function
    Properties:
      FunctionName: !Sub 'url-shortener-redirect-${StageName}'
      CodeUri: ./artifacts/lambda-redirect/
      Handler: bootstrap
      Events:
        GetSlug:
          Type: HttpApi
          Properties:
            ApiId: !Ref HttpApi
            Method: GET
            Path: '/{slug}'
      Policies:
        - Version: '2012-10-17'
          Statement:
            - Effect: Allow
              Action:
                - dynamodb:GetItem
                - dynamodb:PutItem
                - dynamodb:UpdateItem
                - dynamodb:Query
                - dynamodb:Scan
              Resource:
                - !GetAtt ShortlinksTable.Arn
                - !GetAtt CountersTable.Arn
            - Effect: Allow
              Action:
                - logs:CreateLogGroup
                - logs:CreateLogStream
                - logs:PutLogEvents
              Resource: '*'
      Environment:
        Variables:
          DYNAMO_TABLE_SHORTLINKS: !Ref ShortlinksTableName
          DYNAMO_TABLE_COUNTERS: !Ref CountersTableName

  AdminFunction:
    Type: AWS::Serverless::Function
    Properties:
      FunctionName: !Sub 'url-shortener-admin-${StageName}'
      CodeUri: ./artifacts/lambda-admin/
      Handler: bootstrap
      Events:
        GetLinks:
          Type: HttpApi
          Properties:
            ApiId: !Ref HttpApi
            Method: GET
            Path: /api/links
        PostLinks:
          Type: HttpApi
          Properties:
            ApiId: !Ref HttpApi
            Method: POST
            Path: /api/links
      Policies:
        - Version: '2012-10-17'
          Statement:
            - Effect: Allow
              Action:
                - dynamodb:GetItem
                - dynamodb:PutItem
                - dynamodb:UpdateItem
                - dynamodb:Query
                - dynamodb:Scan
              Resource:
                - !GetAtt ShortlinksTable.Arn
                - !GetAtt CountersTable.Arn
            - Effect: Allow
              Action:
                - logs:CreateLogGroup
                - logs:CreateLogStream
                - logs:PutLogEvents
              Resource: '*'
      Environment:
        Variables:
          DYNAMO_TABLE_SHORTLINKS: !Ref ShortlinksTableName
          DYNAMO_TABLE_COUNTERS: !Ref CountersTableName
          GOOGLE_OAUTH_CLIENT_ID: !Ref GoogleOAuthClientId
          ALLOWED_DOMAIN: !Ref AllowedDomain
          GOOGLE_AUTH_INSECURE_SKIP_SIGNATURE: ''

Outputs:
  ApiEndpoint:
    Description: HTTP API endpoint base URL
    Value: !Sub 'https://${HttpApi}.execute-api.${AWS::Region}.amazonaws.com'
  ShortlinksTableOut:
    Description: Shortlinks table name
    Value: !Ref ShortlinksTableName
  CountersTableOut:
    Description: Counters table name
    Value: !Ref CountersTableName
```

— — —

Makefile additions (top‑level)

```makefile
.PHONY: iac-validate
iac-validate:
	sam validate -t infra/sam/template.yaml
```

— — —

Notes and follow‑ups
- Packaging/build hooks (e.g., `cargo-lambda build --release --target x86_64-unknown-linux-gnu`) are intentionally deferred; Phase 11 focuses on IaC shape and validation only.
- When the Dynamo adapter gains real calls (Phase 12+), no infra change is required; permissions already cover read/write/scan/update on the two tables.
- If we later adopt ARM (`arm64`), ensure Rust artifacts are compiled accordingly and update `Architectures`.
