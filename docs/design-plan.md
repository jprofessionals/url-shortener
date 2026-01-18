# Serverless URL Shortener – Design and Implementation Plan

## Overview and Goals

We aim to build a **serverless URL shortener** application for internal use. The system will let authenticated users from a specific Google Workspace domain (e.g. `yourcompany.com`) create shortened URLs, which redirect anyone to the original long URLs. By leveraging AWS managed services (Lambda, API Gateway, DynamoDB, etc.), the solution will be highly scalable, secure, and require minimal maintenance. Key goals include:

- **Private Shortening Service:** Only authorized company users can create or manage short URLs (via Google Sign-In domain restriction). Short link creation and management will be gated behind authentication.
- **Public Redirection:** The generated short links can be used by anyone (no login required to follow a link). Hitting a short URL will redirect the client to the stored long URL.
- **Ease of Deployment (AWS-first):** The primary target environment is AWS (using serverless services). We will use infrastructure as code for repeatable deployment. A local/self-hosted mode will also be provided for testing and development.
- **Modern & Maintainable:** Use the latest stable technologies and best practices (e.g. AWS Lambda + API Gateway, modern OAuth2 for Google login, etc.) to ensure the solution is robust, easy to extend, and cost-effective.

## Requirements

1. **Shorten URLs:** Provide an interface for authorized users to input a long URL (and an optional custom alias) and receive a short URL. The mapping is stored for future redirects.
2. **Redirect**: For any incoming request to a short URL, the system returns an HTTP redirect (e.g. 301/308) to the original long URL. If the short code does not exist, a user-friendly error or 404 is returned.
3. **Admin UI with Authentication:** Provide a web-based admin page for link creation and management. Users must sign in with Google OAuth using a corporate domain email. Only users from the allowed domain can access admin features. (e.g. “Sign in with Google” should allow *@yourcompany.com emails only.)
4. **Link Management:** Authorized users should be able to view the list of links they (or all company users) have created, for reference or copying. (Deletion of links can be a future enhancement but is not in initial scope.)
5. **Technology Constraints:** Solution should use AWS serverless services (Lambda, DynamoDB, etc.) and remain cloud-native. It should also support a local mode (or self-hosted mode) for development and testing without deploying to AWS.
6. **Security & Privacy:** Only domain users can create links. The system should verify auth tokens on the backend to enforce domain restriction. Use least-privilege IAM permissions for AWS resources. Data (URL mappings) should be stored securely (in a private DynamoDB table). All network communication must be over HTTPS.
7. **Performance & Scalability:** The redirect endpoint should be very fast and able to scale to handle many requests with low latency. The design should minimize cold-start impact and allow easy scaling (serverless handles scaling implicitly). Optionally, use caching or global distribution (CloudFront) if necessary for performance, but keep initial design simple.
8. **Maintainability:** All important structures (data models, config) will be clearly defined. Code should be modular (separating concerns like auth, DB access, business logic) to allow easy updates. Infrastructure as Code will be used to manage resources in a reproducible way.

## Architecture Design

### System Components

The system will follow a classic serverless web service pattern, split into a **frontend (admin client)** and **backend (HTTP API)**:

- **Admin Frontend (chosen stack: Svelte + Vite, client-only SPA):** A static web application (HTML/JavaScript) that contains the admin interface. It is built with Svelte and Vite (no SSR) and shipped as static assets. Host on AWS S3 (static website) or any static host (Cloudflare Pages, GitHub Pages, Firebase Hosting). It includes:
    - **Google Sign-In integration:** to authenticate the user. Only users from the allowed Google Workspace domain can sign in.
    - **Link Management UI:** a form to submit new URLs (and optional custom short codes), and a listing section to display existing short links.
    - The frontend calls the backend Admin API via HTTP and passes the Google ID token in `Authorization: Bearer <token>`.

- **Backend API:** A set of serverless HTTP endpoints (AWS API Gateway + AWS Lambda) that implement the application logic:
    - **POST `/api/links`** – *Create Short Link.* Accepts a long URL (and optional desired short code) from the admin UI, verifies the request is authenticated, then generates a new short code if none provided, stores the mapping, and returns the short link info.
    - **GET `/api/links`** – *List Short Links.* Returns a list of all stored short links (or those created by the user/domain) for display in the admin UI. Requires authentication.
    - **GET `/<shortCode>`** – *Redirect.* This is the public endpoint for anyone hitting a short URL. Looks up `<shortCode>` in the database and responds with a 301/308 redirect to the original URL. No auth required (open to all). If not found, returns 404 or a simple error page.

These components communicate as follows:
- An **admin user** opens the Admin UI in their browser. They sign in via Google; the frontend obtains a **Google ID token** proving their identity/email.
- When the user submits a URL to shorten, the frontend JavaScript calls the **Create API** (`POST /api/links`) with the long URL and passes along the Google ID token (in an `Authorization` header or as a bearer token). The backend Lambda verifies the token and the user’s domain, then processes the request.
- The **Create Lambda** generates a unique short code (either random or via an ID counter as discussed below), stores the mapping in **DynamoDB**, and returns the new short URL (or short code) to the frontend. The frontend can then display it.
- When someone (could be any end-user) visits the short URL (e.g. `https://short.company.com/abc123`), the request hits API Gateway which triggers the **Redirect Lambda**. The Lambda looks up `abc123` in DynamoDB. If found, it returns an HTTP redirect response pointing to the long URL (setting the `Location` header). If not found, it returns a 404 or a message like "Short URL not found."
- The **List Lambda** (for `GET /api/links`) will scan or query the DynamoDB table for link entries (optionally filtered by creator domain if needed) and return the data (short code, long URL, creator, etc.). The admin UI calls this to show the list of existing links after user is authenticated.

All API calls (except the open redirect) require a valid Google ID token from the allowed domain. The token is verified by our backend on each request.

**Diagram (Conceptual):**

- *Client (Browser, Admin User)* → [**Google OAuth2**] (for sign-in) → *ID Token* → [**API Gateway**] → [**Lambda (Create/List)**] → [**DynamoDB**] (store or fetch data).
- *Client (any user clicking short link)* → [**API Gateway**] → [**Lambda (Redirect)**] → [**DynamoDB**] (lookup) → *HTTP Redirect* → *Original URL*.

*(For production, an Amazon CloudFront distribution can optionally front the API and static site under one domain for convenience. Initially, use separate origins with CORS to minimize cost/complexity.)*

### Frontend stack and deployment strategy

- Stack: Svelte + Vite, compiled to static assets (no server runtime, no SSR).
- Local development: `npm create vite@latest admin -- --template svelte`, set Vite env variables `VITE_API_BASE` and `VITE_GOOGLE_CLIENT_ID`, then `npm run dev` (default http://localhost:5173). When running the backend locally with SAM, set the `CorsAllowOrigin` parameter to `http://localhost:5173`.
- Minimal-cost AWS deployment: build with `npm run build` → `dist/`, upload to an S3 bucket with static website hosting enabled. For HTTPS and custom domains, add CloudFront (small additional cost). Alternatively, for zero-cost hosting, use Cloudflare Pages or GitHub Pages and point `CorsAllowOrigin` to that origin.
- Auth: Use Google Identity Services (GIS) in the browser to obtain an ID token; backend verifies signature (JWKS), audience, issuer, expiry, and domain.

### Data Model and Structures

All core data structures (“structs”) are defined up front for clarity:

- **ShortLink** – represents a stored short URL mapping. Fields:
    - `shortCode` (string): **Primary Key** (e.g. `"abc123"`). Unique short identifier used in the short URL.
    - `originalUrl` (string): The original long URL that the short link redirects to.
    - `createdAt` (timestamp): Date/time when this mapping was created.
    - `createdBy` (string): Identifier of the user who created the link (e.g. their email address). This helps with auditing and listing. *(Since all users are from one domain, we might store the full email or just the local part.)*
    - *Optional:* `expiration` (timestamp or TTL): If we want links to expire after some time. (Not in initial requirements, but DynamoDB TTL could be used in future.)
    - *Optional:* `customAlias` (boolean or a flag): To mark if the shortCode was user-specified. (Alternatively, we infer this if `shortCode` was provided vs generated.)

- **ShortLinkCounter** – (if using an ID-based code generation approach) a structure to keep track of the last used ID for generating new short codes:
    - `idName` (string, primary key): Name of the counter (e.g. `"globalCounter"` for all links, or separate counters per something if needed).
    - `value` (number): The current counter value. This will be atomically incremented to generate new IDs.
    - *Usage:* We will use a separate DynamoDB table (or a reserved item) to store this counter. Each new link creation does an atomic increment on this value to get a unique ID, which is then converted to a base62 short code.

- **AuthToken (Google ID Token) Claims** – We don’t store this in DB, but we parse it in memory:
    - Notably, we extract `email` (string) and/or `hd` (hosted domain) from the token to verify domain.
    - The token’s `aud` (audience) will be our OAuth client ID, and `iss` (issuer) should be Google’s authorized issuers. These will be checked in verification.

- **Config/Settings**: (likely as constants or environment variables)
    - `ALLOWED_DOMAIN` (string): The Google Workspace domain allowed (e.g. `"yourcompany.com"`). All user emails must match this.
    - `GOOGLE_OAUTH_CLIENT_ID` (string): The Client ID for Google Sign-In, used by the frontend and also by backend to verify token audience.
    - `SHORTLINK_DOMAIN` (string): (Optional) The domain where short links will be hosted (for constructing the full short URL to return, e.g. `"https://short.company.com"`). This can also be derived from an environment or API Gateway domain.
    - Other configuration like table names, etc., can be set in environment or a config file.

These structures will guide the implementation. By defining them upfront, we ensure consistency across components. The **DynamoDB schema** corresponds to the structures:
- DynamoDB *Table*: `ShortLinks` (Name TBD) with primary key `shortCode` (string). No sort key needed (each short code is unique). This stores items in the shape of **ShortLink**.
- DynamoDB *Table*: `Counters` with primary key `idName` (string). It will have one item with `idName="global"` (or similar) storing a numeric `value`. (If using this for ID generation.)
- Both tables will be configured with on-demand capacity (for simplicity) or provisioned with auto-scaling. The `ShortLinks` table could use a TTL on `expiration` if that feature is used later (not now).

### Short Code Generation Strategy

To ensure each short link code is unique, we will use one of two strategies (with a preference for the second for modern best practice):

1. **Random Code Generation:** Generate a random alphanumeric string of a fixed length (e.g. 6-8 characters). Check in the database if that code already exists; if yes, generate a new one (collision unlikely but possible). This is simple and effective given the domain-limited usage and the huge space of combinations (for 6 chars [a-zA-Z0-9] there are 56 billion possibilities). Collisions can be handled by retrying a few times. The code length could be extended if needed to further reduce probability of collision.

2. **Sequential ID with Base Conversion (Preferred):** Use an auto-incrementing counter to generate short codes. Each new link gets the next numeric ID, which is then converted to a base-62 (0-9, A-Z, a-z) string. This ensures no collisions and produces the shortest possible codes for each new link (starting from “0-9”, “A-Z”, “a-z”, then multi-length). DynamoDB doesn’t have built-in auto-increment, but we can simulate it with an **atomic counter** update. We maintain a `Counters` table as described; each creation request does an `UpdateItem` with `ADD value :increment` (where increment=1) to atomically get a new ID value, and we use that. (This costs one extra write per link but guarantees uniqueness and monotonic IDs.) We will implement a function to convert the numeric ID to a base-62 code string.

   *Rationale:* Using a DynamoDB-managed counter is a robust solution recommended by AWS experts to generate unique short codes without running into collisions or needing complex coordination. It trades a tiny bit of extra write throughput for guaranteed uniqueness and potentially more user-friendly short links (no accidental inappropriate words, and length grows only as needed).

We will implement strategy (2) for the final design (“latest and greatest”), as it is deterministic and scalable. However, the system is designed such that the generation logic is modular – it could be switched to random or another approach if needed without affecting other parts (just the function that produces a new code).

### Security & Authentication

**Google Authentication & Domain Restriction:** We use Google OAuth 2.0 for user login on the admin page. Users will click “Sign in with Google”, and our application will request an ID token from Google’s OAuth endpoint. We will specify the hosted domain (using the `hd` parameter set to our company domain) in the OAuth request to hint that only that domain is allowed. This causes the Google login UI to restrict or simplify selection to accounts from that domain. *However, this is not foolproof by itself – we will **always validate the domain on the backend** as well.* The `hd` param is merely a UI filter; the server must check the token’s `hd` or email claim to enforce the restriction.

On the backend, for any admin API request, we will perform these steps to authenticate/authorize:

- **Verify ID Token Signature and Audience:** Use Google's public keys or a Google API client library to verify the ID token JWT. Ensure it is issued by Google (`iss` == accounts.google.com or googleapis.com) and intended for our app (`aud` matches our Google client ID). This guarantees the token is valid and not tampered with.
- **Verify Domain Claim:** Parse the token payload for the `hd` (hosted domain) claim or the email field. Confirm that the email ends with `@yourcompany.com` (or exactly matches the domain). If the domain doesn’t match the allowed one, **reject** the request (HTTP 403 Forbidden).
- If all checks pass, extract the user’s email (and possibly name) from the token and consider the request authenticated as an allowed user. We do **not** maintain separate application sessions or user records – the Google token is our source of truth. Each request must present a valid token (stateless auth).
- **Authorization:** All authenticated domain users are considered “admins” for this app by requirement, so they can create links or list links. We don’t differentiate roles beyond the domain check. (Further role-based control could be added later if needed.)

We will use a library or write a utility for token verification. For instance, in Node.js we might use the Google Auth Library or in Python use `google.oauth2.id_token.verify_oauth2_token`. This simplifies verifying the JWT’s signature against Google’s public certs. The library will also give us the token claims to check the domain. This verification logic will be implemented as a **middleware function** or helper that our Lambda handlers call at the start of execution for protected routes.

**IAM Roles & Permissions:** In keeping with the principle of least privilege, our AWS Lambda functions will have IAM roles that only allow necessary actions. For example:
- The function that accesses DynamoDB will have permissions scoped only to the specific DynamoDB table (or tables) it needs (e.g., allow GetItem/PutItem/UpdateItem on `ShortLinks` table and UpdateItem on `Counters` table). No broad wildcard access to all tables.
- If the Lambda needs to call other AWS services or if we use AWS SDK, those will be granted minimally. (For our scope, mainly DynamoDB access is needed. We might also allow CloudWatch Logs permission for logging.)
- The admin static site S3 bucket will be configured to only allow public read of the static files (or restricted via CloudFront). The API Gateway will be the entry for dynamic actions.

**CORS Configuration:** If the admin frontend is hosted on a different domain than the API (which is likely, e.g., static files on S3 or GitHub pages and API on some AmazonAPIGateway domain or custom domain), we must enable CORS on the API. We will configure API Gateway to allow the admin origin domain for requests to the `/api/*` endpoints (or use `*` during development). Specifically, we’ll ensure the `Access-Control-Allow-Origin` header is set appropriately (and allow credentials = false since we use token auth, not cookies). This will be handled either via API Gateway configuration or in Lambda responses if using Lambda proxy integration.

**HTTPS:** All interactions will occur over HTTPS. API Gateway provides HTTPS endpoints by default. For custom domains (short link domain or admin domain), we will use SSL certificates (AWS Certificate Manager) and Route 53 if needed to ensure links are secure.

### Deployment Considerations (AWS & Local)

On AWS, the stack will be deployed primarily in a single region (e.g. us-east-1 or eu-west-1 depending on the company’s choice) using the following services:

- **Amazon API Gateway (HTTP API):** to define our endpoints (`/api/links`, etc., and the redirect catch-all). It will integrate with Lambda functions. We choose HTTP API (v2) for lower latency and cost compared to REST API, as it still meets our needs. We’ll set up routes and methods accordingly, and enable CORS on the `/api` routes.
- **AWS Lambda Functions:** Three main Lambda functions: `RedirectFunction`, `CreateLinkFunction`, and `ListLinksFunction` (names tentative). They will be deployed via code package (or container image if we prefer) with environment variables for configuration (allowed domain, etc.). We’ll use the latest runtime (e.g. Node.js 18 or Python 3.10) for performance and long-term support.
- **Amazon DynamoDB:** Two tables as described (`ShortLinks`, `Counters`). We will enable on-demand capacity initially for simplicity. We might enable a TTL attribute on `ShortLinks` if expiration feature is desired later (not now). The `Counters` table will have the one item for global counter – we’ll initialize it with a starting value (e.g. 1000 or 0).
- **Amazon S3 (Static Website):** One S3 bucket to host the admin UI files (HTML, CSS, JS). This bucket can be set to static website hosting or accessed via CloudFront. We will store the compiled frontend assets here.
- **Amazon CloudFront (optional for unified domain):** As an enhancement, we can use CloudFront to serve both the static site and API under one domain. For example, requests to `https://short.company.com/admin/*` could be routed to the S3 bucket, and `https://short.company.com/api/*` and `/*` (other) to API Gateway or Lambda origins. This avoids needing CORS and allows the short links to share the domain with the admin UI. In the initial implementation, we may skip CloudFront and use separate domains to reduce complexity (using CORS). CloudFront can be configured later if a friendly unified domain is required.

- **Infrastructure as Code:** We will create a CloudFormation/SAM template or use the Serverless Framework to define all the above resources (API, Lambdas, DynamoDB, S3, roles, etc.) in code. This ensures repeatable deployment and easier management. (E.g., define DynamoDB tables and Lambda functions in an AWS SAM template, along with necessary IAM policies and environment variables.)

**Local Development Mode:** To ease development and testing, we will also implement a way to run the system locally without deploying to AWS:
- We can create a simple Express.js server (if using Node) or a Flask app (if using Python) that mimics the API Gateway + Lambda behavior. For example, run on `http://localhost:3000` and handle routes `/api/links` and `/{shortCode}` similarly. This local server can use an **in-memory store or a lightweight database** (like an in-memory Python dict or SQLite) instead of DynamoDB for testing. This allows quick iteration without incurring AWS calls.
- We will structure the code so that the core logic for creating and redirecting URLs is in functions that can be invoked in either environment. The local server can import the same logic but use a different storage backend (perhaps abstract storage behind an interface so we can plug in DynamoDB vs local store).
- Google authentication in local dev: We can still use Google’s OAuth in development (it requires that the OAuth client is configured with the local app’s origin or a test domain). Alternatively, for local testing we might bypass Google Auth by accepting a dummy token or allowing a development mode override. But ideally, we test the full flow: we can create a Google OAuth client ID for "http://localhost" origin and use it for testing sign-in with a test domain account.
- AWS SAM CLI or Serverless framework offline mode could also be used to simulate API Gateway and Lambda locally. Using SAM CLI, one can invoke API Gateway locally and have Lambdas run on local Docker. This is another route, but for simplicity, a custom lightweight server might be easier for an AI agent to execute.

The combination of AWS deployment and local mode ensures we can verify functionality quickly and run the solution in production reliably.
