## Implementation Plan

We will implement the system in a series of clear, self-contained tasks. Each task is designed to be achievable in a focused development session (roughly 1–8 hours each). The tasks are ordered logically, building up the system from setup, through backend, then frontend, then deployment. All data structures from the design are used consistently throughout the implementation.

Below is the step-by-step implementation plan with details:

1. **Project Setup and Repository Initialization** – *Goal: Scaffold the project and ensure all necessary tools and configs are in place.*
    - Set up a new source control repository (e.g. Git) for the project. Create base directories for `backend/` (Lambda functions and related code) and `frontend/` (admin UI code), and possibly an `infrastructure/` folder for deployment templates.
    - Initialize the project with appropriate build configurations:
        - If using Node.js/TypeScript: create a `package.json` and install dependencies (AWS SDK, a web framework if needed, Google auth library, etc.). Set up a TypeScript config (`tsconfig.json`) if using TS.
        - If using Python: set up a virtual environment and requirements file for needed packages (boto3, google-auth, Flask for local, etc.).
    - Create a basic **hello-world Lambda function** in the backend to verify the toolchain. For example, a simple function that returns “OK” can be configured. If using Serverless Framework or SAM, configure a dummy function and test deployment locally.
    - Set up a build and deployment workflow:
        - Define how code will be packaged for Lambda (for Node, probably webpack or just bundling if simple; for Python, ensure zip creation).
        - Optionally, set up a testing framework (e.g. Jest for Node or PyTest for Python) to enable writing tests in later tasks.
    - Create a `.gitignore` for node_modules, build artifacts, etc.
    - **Outcome:** A structured project skeleton with the necessary configs. You should be able to run a dummy function locally or invoke a sample test. This ensures that subsequent tasks have a foundation to build on.

2. **Define Configuration and Data Models** – *Goal: Define all important constants and data structures in code.*
    - In the backend project, define a configuration module or environment setup:
        - Define `ALLOWED_DOMAIN` (the email domain string). For now, set a placeholder (e.g. `yourcompany.com`) which can be configured via env variable or a config file.
        - Define `GOOGLE_OAUTH_CLIENT_ID` (from Google API Console for our app). This will be used in front-end and back-end validation. Store it in a config file or env var.
        - Define `DYNAMO_TABLE_SHORTLINKS` and `DYNAMO_TABLE_COUNTERS` names as constants (to ensure consistency between code and infrastructure).
    - Define the **data model classes/structs** in code to mirror the design:
        - For instance, create a TypeScript interface or a Python dataclass for `ShortLink` with fields `shortCode: string; originalUrl: string; createdAt: string (or Date); createdBy: string`.
        - If using an object-relational mapper or DynamoDB Document Client, you might not need a full class, but having a TypeScript type or schema validation is helpful for clarity.
        - Define a simple structure for the counter as well (e.g. an interface `CounterRecord { idName: string; value: number; }`).
    - Implement a **base62 encoding utility** (if using the counter approach):
        - Write a function `encodeBase62(number: number): string` that converts a number to a base-62 string. Include the character set (0-9, A-Z, a-z). You can implement this by repeatedly dividing the number by 62 and mapping remainders to chars.
        - Write a counterpart `decodeBase62(string): number` if needed (not strictly required unless we want to decode, but for completeness or testing).
    - Set up any **common utility** structures:
        - e.g. If using Node, install `aws-sdk` (or specifically DynamoDB DocumentClient v3) and maybe set up a DynamoDB client instance that other modules can import.
        - If using Python, prepare `boto3.resource('dynamodb')` usage.
        - A helper for getting current timestamp (for `createdAt`).
    - **Outcome:** The project has clearly defined data models and config. This makes the following implementation tasks easier and less error-prone, since all team members (or AI agents) refer to the same struct definitions.

3. **DynamoDB Setup and Data Access Layer** – *Goal: Prepare the database tables and abstract low-level access.*
    - Define the **DynamoDB tables** in infrastructure (if using IaC) or create them manually for development:
        - `ShortLinks` table with primary key `shortCode` (String). No sort key.
        - `Counters` table with primary key `idName` (String). (One item for "global" counter).
        - (If using CloudFormation/SAM, start writing the template defining these resources. If using Serverless framework, add to `serverless.yml` under resources.)
    - Write a small **data access module** in the code to interact with DynamoDB:
        - For Node/TS, perhaps create `db.ts` which exports functions like `getLink(shortCode): ShortLink|None`, `putLink(shortLink: ShortLink): void`, `incrementCounter(name): number`.
        - For Python, maybe a class `ShortLinkRepository` with methods `get(shortCode)`, `create(shortCode, originalUrl, createdBy)`, etc.
        - Implement `getLink(shortCode)` to do a DynamoDB GetItem on the `ShortLinks` table key. Return the item (or `None` if not found). This will be used by the redirect logic.
        - Implement `createLink(shortCode, originalUrl, createdBy)` to do a PutItem in the table. This should include `createdAt` (set to now) and the provided fields. Use conditional write if we want to ensure not overwriting existing code (DynamoDB supports condition expressions, e.g., attribute_not_exists(shortCode) to avoid collisions). This is especially useful for custom aliases to fail if taken.
        - Implement `listLinks()` to scan the table and return all entries (or later we can use queries if we add indices). For now, scanning is acceptable for moderate sizes since internal use (we note that this could be inefficient at large scale, but okay for MVP).
            - Optionally, if we only want to list links by the current user, we could add a GSI on `createdBy` and query by user email. This is a stretch goal; initially, listing all links is fine if user base is the same domain.
        - Implement `incrementCounter(name)` if using counter:
            - Use DynamoDB UpdateItem on `Counters` table where `idName = name` (e.g., "global"), with `ExpressionAttributeValues: {":inc": 1}` and `UpdateExpression: "SET value = value + :inc"`, plus `ReturnValues: "UPDATED_NEW"` to get the new value in one call.
            - Parse the returned value (the new counter) and return it.
            - Make sure to initialize the counter in the table (if not exists, perhaps handle a condition or assume we create a "global" item at deploy time with initial value 0 or 1000).
        - **Error handling:** The data layer should throw or return errors if something unexpected happens (e.g., DynamoDB exceptions). For example, if `createLink` gets ConditionalCheckFailed (shortCode already exists), handle that so we can return a friendly error to the user.
    - Test the data access functions locally (you can use a local DynamoDB emulator or actual AWS test environment):
        - You might write a small script or unit test that calls `incrementCounter` twice to see if it returns incrementing values, and that `createLink` then stores a link and `getLink` retrieves it.
        - If no DynamoDB available locally, consider using a mock or simply skip actual calls (to be integrated when Lambdas run). But ideally, setting up DynamoDB Local (or using AWS SAM’s local Dynamo) could be done for a thorough test.
    - **Outcome:** We have functional code to interact with the database, and the tables are defined. This layer isolates database specifics from business logic and will be used by the Lambda handlers. It also confirms the Dynamo schema works as intended.

4. **Implement the Redirect Lambda Function** – *Goal: Users can be redirected via short URL through this function.*
    - Create a Lambda function (file) for **redirect** logic, e.g. `redirect.js`/`redirect.ts` or `redirect.py`. This function will be triggered by an HTTP GET request on the short URL path.
    - **Function Handler Logic:**
        - Parse the incoming request to extract the `shortCode` from the path. (In API Gateway, this might be a path parameter or part of the route; with Lambda proxy integration, the event will contain the path.)
        - Use the data access layer: call `getLink(shortCode)` to retrieve the record from DynamoDB.
        - If a link is found:
            - Construct a response that is an HTTP redirect. Typically, we return a 301 or 308 status code with a `Location` header pointing to `originalUrl`.
            - In Lambda proxy integration (API Gateway), we return an object like:
              ```json
              { "statusCode": 301, "headers": { "Location": "<originalUrl>" }, "body": "" }
              ```
              (No body needed for redirect, or we could include a message "Redirecting...").
            - Use 301 (Moved Permanently) or 308 (Permanent Redirect) for permanence. 308 is similar to 301 but preserves HTTP method; since this is GET, either is fine. We can choose 301 for wide compatibility or 308 as modern (theburningmonk example used 308).
        - If not found:
            - Return a 404 status. Possibly return a simple HTML or JSON body saying "Short URL not found".
            - Alternatively, we could redirect to a generic error page or the admin UI. A simple approach is an HTTP 404 with a plain text "Not found".
        - Ensure the function properly catches exceptions (e.g., if DynamoDB call fails, return a 500 with an error message).
    - **Testing:** Write unit tests for the handler logic:
        - Case 1: valid code present in DB -> expect a response with 301 and correct Location.
        - Case 2: code not in DB -> expect 404.
        - We can simulate the `getLink` call by mocking the data access or using a test database entry.
    - **Configure API Gateway route:** In the infra config, map GET `/{shortCode}` (any path that isn’t caught by another route) to this Lambda. If using SAM, you might use an event source like:
      ```yaml
      Events:
        Redirect:
          Type: Api
          Properties:
            Path: /{shortCode}
            Method: GET
      ```
      making sure it handles all patterns except perhaps reserved paths (like `/api/*` which will go elsewhere).
    - (If using Serverless Framework, configure an HTTP event with path `/{shortCode}`.)
    - At this point, you can deploy or run this function locally to verify:
        - Try invoking it with a known test code. If using SAM local, you can pass an event with pathParameters. If local Express, hit the route.
        - Ensure that the HTTP response is correctly formed for redirect (some frameworks require base64 encoding for binary, but since this is text headers, we’re fine).
    - **Outcome:** The redirect endpoint is implemented. It doesn’t require authentication and should be as lightweight as possible. When a short URL is hit, this lambda will fetch from DynamoDB and quickly return a redirect. This is the core feature for end users.

5. **Implement the Authentication Verification Module** – *Goal: Create a re-usable component to validate Google ID tokens and enforce domain restriction.*
    - Implement a utility function (or class) in the backend, e.g. `auth.js`/`auth.ts` or `auth.py`, that will be used by the protected Lambdas (create and list):
        - Function `verifyGoogleToken(idToken: string): AuthResult`. This function takes the ID token (JWT) as input.
        - Use Google’s libraries or JWT verification:
            - If using Node, install `google-auth-library` (`@google-auth-library/oauth2` for example) and use `OAuth2Client.verifyIdToken` with the token and expected CLIENT_ID.
            - If using Python, use `google.oauth2.id_token.verify_oauth2_token` providing a `requests.Request()` to fetch Google’s certs.
            - Alternatively, manually fetch Google's public keys (from https://www.googleapis.com/oauth2/v3/certs) and use a JWT library to verify signature. Using a library is simpler.
        - Verify the token:
            - Check signature validity and expiration.
            - Check `aud` (audience) matches our `GOOGLE_OAUTH_CLIENT_ID`.
            - Check issuer is Google (`accounts.google.com` or `https://accounts.google.com`).
            - Extract the payload claims (`email`, `email_verified`, `hd` etc.).
        - Verify domain:
            - Ensure `hd` claim (hosted domain) equals the `ALLOWED_DOMAIN` (e.g. "yourcompany.com"). *Note:* Some tokens might not have `hd` if not a Google Workspace account? In our case, since we restrict login via Google, we expect it for Workspace accounts. If not present, we could alternatively parse email domain from the email claim as a fallback.
            - Ensure `email_verified` is true (just to avoid unverified email edge case).
            - If domain is incorrect, consider this token invalid for our purposes.
        - Return an object or result:
            - On success: return an object containing the user’s email (and maybe name) and a flag authorized = true.
            - On failure: throw an error or return a result with authorized = false and reason.
        - This function will not itself send any HTTP response; it just validates and returns info. The Lambdas will use it to decide to proceed or return 401/403.
    - **Integrate with Lambdas (framework):** If using an API Gateway authorizer could be an option, but we keep it simple:
        - We will call `verifyGoogleToken` at the start of the handler for any admin API request. If it fails, the handler returns 401 Unauthorized or 403 Forbidden.
        - Ensure that the Lambdas can retrieve the token from the request. Typically, the admin will send the ID token in the `Authorization: Bearer <token>` header. In a Lambda proxy event, this is `event.headers.Authorization`. We should document that the frontend must send this header.
    - **Testing:** Write tests for `verifyGoogleToken` with various scenarios:
        - Perhaps use a known valid token from Google (could generate one manually for a test account) and test that it passes. This is tricky without an actual token. Alternatively, mock the Google library call. For now, trust the library and test mainly the domain check logic by simulating payload input.
        - Test that a token with wrong domain is rejected (simulate by calling the function with a dummy payload).
        - Test that a malformed token or invalid signature triggers an error (if library provides an exception, catch it and ensure our function propagates it as a false result).
    - **Outcome:** We have a robust authentication verifier that ensures only company domain users are allowed. This will be used in subsequent tasks to protect the create and list functionality.

6. **Implement the Create Short Link Lambda** – *Goal: Allow authenticated users to create new short links.*
    - Create the Lambda function handler for **create link** (e.g. `createLink.js` or `create_link.py`). This will handle HTTP POST requests to `/api/links`.
    - **Handler Logic:**
        - **Authentication**: Extract the ID token from the request headers (e.g. `Authorization` header). If not present or not in expected format, return 401. If present, call `verifyGoogleToken(token)` (from Task 5).
            - If verification fails (throws or returns not authorized), return an HTTP 403 response. The response can be JSON with an error message like `{"error": "Unauthorized"}`.
            - If success, get the user email info (we might use it to set `createdBy`).
        - **Parse Input**: The request body is expected to contain the URL to shorten, and optionally a desired short code:
            - If using API Gateway with Lambda proxy, the event will have a `body` (JSON string). Parse it (JSON.parse in Node or use `json.loads` in Python).
            - Expect fields like `url` (the long URL), and optionally `alias` (the desired short code alias).
            - Validate the `url`: ensure it’s a proper URL format (we can use regex or a library to check the scheme is http/https). Possibly ensure it’s not empty. We might also reject obviously malicious URLs or non-http(s) if desired.
            - If an `alias` is provided (custom short code), validate it:
                * It should be alphanumeric (and maybe allow `-` or `_` if we want). We can define a regex for allowed codes. Also decide on a max length (perhaps 10 or 15).
                * Ensure it’s not a reserved word like "admin" or "api" which we use for routes.
                * You can maintain a small list of disallowed aliases (like "admin", "api", "login", etc., to avoid conflicts).
        - **Generate Short Code**:
            - If the user provided a custom alias and it passed validation, use that as `shortCode` (after perhaps lowercasing it or preserving case depending on if our system is case-sensitive. Many short URLs are case-sensitive to maximize space, so we can allow case sensitivity).
            - If no alias provided, generate one:
                * Use the **counter method**: call the `incrementCounter("global")` function from the data layer (Task 3) to get the next ID number. Then call `encodeBase62(id)` to get a code string. Use that as the `shortCode`.
                * Or if we were using random: generate random string and check DB. (Since we chose counter, we'll implement that path.)
            - It's good to ensure the generated code (or provided alias) doesn’t accidentally collide with existing entry:
                * If using counter, collisions won’t happen (unique by design).
                * If using provided alias or in a rare scenario of counter re-use (shouldn’t happen), we handle it:
                    - For provided alias: attempt to put item with ConditionExpression attribute_not_exists. If it fails, return an error to user like 409 Conflict "Alias already in use".
                    - For generated code: if by some chance the put fails with collision (shouldn’t with counter as long as counter is strictly increasing and table has no item with that code yet; if using random, you’d loop and retry).
        - **Store in DB**: Create a `ShortLink` object with `shortCode`, `originalUrl`, `createdAt = now`, `createdBy = userEmail`. Use `createLink()` data function to put it into DynamoDB:
            * If `createLink` throws a conditional check error (meaning the code was taken), and if we were using a custom alias scenario, catch it and return a 409 to the client.
            * Any other DB error return 500.
        - **Construct Response**: On success, return HTTP 201 Created with a JSON body containing the short link info. For example:
            ```json
            { "shortCode": "abc123", "shortUrl": "https://short.company.com/abc123", "originalUrl": "https://long.url/...." }
            ```
          The `shortUrl` can be constructed using the known domain (if we have `SHORTLINK_DOMAIN` config) or, if not, we can return just the code and the frontend can prepend the domain it knows. We’ll include it for convenience if possible.
        - Set appropriate headers like `Content-Type: application/json`. Also, if CORS is needed (and not handled globally), add `Access-Control-Allow-Origin: *` or specific domain in the response headers.
    - **Testing the Create Function:**
        - Write unit tests for logic pieces:
            * Test with a valid token (could mock `verifyGoogleToken` to return a user email) and valid URL input -> expect a success response JSON with a shortCode.
            * Test that it calls the data layer correctly. Possibly mock the DB functions (`incrementCounter` and `createLink`).
            * Test with a custom alias that is taken: mock `createLink` to throw a ConditionalCheckFailed exception -> function should return 409 status.
            * Test with invalid inputs: missing URL or malformed URL -> function should return 400 Bad Request with error message.
            * Test unauthorized: no token or bad token -> 401/403.
        - If possible, test end-to-end locally: run a local instance (or after deploying to a test stage), send an HTTP POST to `/api/links` with a test token (perhaps disabled auth for local or use a dummy token if we allow that in dev).
    - **Connect to API Gateway:** In the API config, set up the POST `/api/links` route integration to this Lambda. If using SAM, define an Api event:
      ```yaml
      Path: /api/links
      Method: POST
      ```
      Possibly enable CORS (SAM has an `Cors: true` option or define in OpenAPI). Ensure that `Authorization` header is forwarded to Lambda (with proxy integration it is by default).
    - **Outcome:** The system now supports creating short links securely. This is a critical piece where most business logic resides. After this task, authorized users can generate new short codes that are saved in the database.

7. **Implement the List Links Lambda** – *Goal: Allow an admin user to fetch a list of existing short links.*
    - Create the Lambda handler for **listing links** (e.g. `listLinks.js` or `list_links.py`) for GET requests to `/api/links` (same path, different method).
    - **Handler Logic:**
        - **Auth**: Similar to create, parse the `Authorization` token from headers. Use `verifyGoogleToken`. If fails, return 401/403.
        - (We could refactor common auth logic between create and list into a shared helper to avoid duplication – e.g., a function `requireAuth(event)` that returns user info or sends back an error. But in an AI step-by-step context, duplicating is okay for clarity. Ideally, refactor if possible.)
        - Since the user is authorized, determine if we want to filter by user or not:
            - If we only want to show the logged-in user’s own links, we’d use their email to query. However, the requirement sounds like the whole domain can see all short links (common in an internal tool – everyone can see all created links). We will list all links for now.
            - If later needed, we could filter by `createdBy` matching the current user, but let’s assume domain users trust each other and volume isn’t too high.
        - Call the data access to get all links: e.g. `listLinks()` which does a DynamoDB Scan on `ShortLinks` table.
            - If performance is a concern with many links, we could use pagination or limit, but probably not needed initially.
            - If using a GSI by createdBy in future, we’d query that index with key = user email for per-user view.
        - Get the result list (array of ShortLink items). Sort them if desired (maybe by createdAt descending so newest first). DynamoDB scan doesn’t guarantee order, so sorting in memory by timestamp could be nice for UI.
        - Construct a response JSON, e.g. `{ "links": [ {shortCode, originalUrl, createdAt, createdBy}, ... ] }`.
            - We might omit `createdBy` or other details if not needed on UI, but including could be fine (especially if later multiple users, to identify who made which link).
        - Return 200 OK with that JSON. Include `Content-Type: application/json` and CORS headers if needed.
    - **Testing:**
        - Test auth required: no token -> 401.
        - Test token valid -> returns list (if DB returns items). We might need to simulate a few items:
            * Mock `listLinks` to return a couple of ShortLink objects, ensure the response contains them.
            * Possibly test that the results are sorted by date if we implement sorting.
        - If filtering by user was implemented, test that only that user’s links show (could simulate by having two links with different createdBy and ensure only one appears if filtering).
    - **API Gateway config:** Map GET `/api/links` to this Lambda. Enable CORS as well.
    - **Outcome:** Admin users can retrieve the list of all short links. This will be used to display data on the admin interface. At this point, the backend functionality (redirect, create, list) is complete and secured. Next, we focus on the frontend.

8. **Admin Frontend - Basic UI & Google Sign-In Integration** – *Goal: Set up the admin page structure and user authentication.*
    - Create an `index.html` (and possibly a couple of JS/CSS files) for the admin interface under the `frontend/` directory.
    - Include the **Google Sign-In script**. Google now provides the "Google Identity Services" for web. We’ll use the latest method:
        - Add a `<script src="https://accounts.google.com/gsi/client" async defer></script>` in the HTML.
        - In HTML, include a **Sign-In button** provided by Google. For example, a `<div id="g_id_onload"...>` and a `<div class="g_id_signin" data-type="standard">` as per Google documentation, configured with our client ID and domain. We can set `data-client_id="YOUR_GOOGLE_CLIENT_ID"` and `data-ux_mode="popup"` (or redirect) and `data-login_uri` if needed (but since we handle in JS, we might use the callback method).
        - Alternatively, use the older gapi auth2 library – but “latest and greatest” is the new Identity Services. So we will use `google.accounts.id.initialize` in JS.
    - In a separate JS file (say `app.js`), write the logic to handle Google sign-in:
        - Use `google.accounts.id.initialize({ client_id: GOOGLE_CLIENT_ID, callback: handleCredentialResponse, hosted_domain: "yourcompany.com" })` to configure the one-tap sign in. (The `hosted_domain` ensures only our domain can sign in here, per Google's new API).
        - Use `google.accounts.id.renderButton(...)` to render the button or just rely on a custom button and call `prompt`.
        - Implement the `handleCredentialResponse(response)` function to receive the ID token from Google after login. Google Identity API will provide `response.credential` which is the JWT.
            - In that function, verify that `response.clientId` is our expected client as a sanity check (should be), and then store the `response.credential` (the ID token) for use in API calls.
            - We likely want to parse the JWT to get user info to display (like user’s name or email on the UI). We can decode JWT on the client (since it’s not sensitive to decode, the signature we trust because it came from Google directly). To decode, we can do a simple base64 decode of the JWT payload or use a library. For simplicity, maybe just display a welcome message like "Signed in as [email]" using the `email` claim from the token (which is base64 JSON in the token).
            - After successful sign-in, hide or remove the Google sign-in button and show the main app interface (the form and list).
        - Also handle if the token is not from the correct domain on client side (though Google might already filter due to hosted_domain, but just in case, check the payload’s hd claim matches our domain; if not, alert error and do not proceed).
    - Structure the HTML:
        - Have a section for the sign-in (the Google button).
        - Have a main app section (initially hidden) that contains:
            * A form with an input field for the long URL, an optional input for custom alias, and a submit button "Shorten".
            * An area (table or list) to display existing short links (populated after we fetch them).
            * Maybe some basic styling (doesn’t need to be fancy; could use a minimal CSS or framework like Bootstrap or Tailwind for nicer look if desired).
        - Keep it simple initially: a vertical column: input field(s) and a button, and below that a list.
    - **Local config for the front-end:**
        - Insert the correct `GOOGLE_OAUTH_CLIENT_ID` in the JS or HTML (this is public info, so it’s okay to embed).
        - Define the API endpoint base URL. If the frontend is served on the same domain as API (with CloudFront), we can call relative `/api/links`. If not, and for local dev, we might need the API URL. Perhaps define `API_BASE_URL = "https://<api-id>.execute-api.<region>.amazonaws.com/..."` or leave it configurable (maybe as a JS global or a comment to replace with deployed URL).
        - For local testing, `API_BASE_URL` could be `http://localhost:3000` if we run a local server.
    - **Testing the sign-in flow:**
        - Host the `index.html` on a local static server (or just open in browser with a live server plugin) to test.
        - Make sure the Google button appears. When clicked, use a test Google account from the domain to sign in. If configured correctly with Google Cloud Console (the OAuth client must list the origin http://localhost if testing locally), you should get a credential in the callback.
        - Check that after sign-in, your JS callback fires, token is stored, and UI transitions (e.g., you hide the login and show the form).
        - (At this stage, the form submission and list won’t work until we implement those, but we can log the token to console to verify it’s working.)
    - **Outcome:** We now have an admin UI that can authenticate the user via Google and is ready to use the token for authorized requests. The next task will handle the form submission and listing functionality.

9. **Admin Frontend - Link Creation and Feedback** – *Goal: Enable the admin UI to call the create API and show the new short link result.*
    - Implement JavaScript logic to handle the "Shorten" form submission:
        - Add an event listener for the form submit or button click. Prevent the default HTML form submission since we will do an AJAX call.
        - On click, retrieve the values from the input fields: the long URL, and optional custom alias field.
        - Basic validation on client side: ensure the long URL field is not empty. Optionally check it starts with http:// or https:// and if not, maybe prepend `http://` or show an error to user (for usability, possibly auto-prepend "http://").
            - Also if alias field is provided, ensure it meets criteria (length, allowed chars). We should mirror the backend validation regex to give early feedback (e.g. only letters/digits, no spaces, etc.).
        - Prepare the request payload, e.g.:
          ```js
          const payload = { url: longUrl };
          if(alias) payload.alias = alias;
          ```
        - Read the stored ID token (from Task 8, after sign-in we stored `window.idToken` or similar).
        - Make a `fetch` call to the Create API endpoint:
            - URL: if using same domain and path, `/api/links`; otherwise full API URL (configured as noted).
            - Method: POST, body: `JSON.stringify(payload)`.
            - Headers: set `Content-Type: application/json`, and `Authorization: Bearer <idToken>`.
        - Handle the response:
            - If success (201), parse JSON. It should contain the new short link info (`shortCode` or `shortUrl`).
            - Display the result to the user. For example, show a success message like "Short URL created: https://...". This could be a new element or an alert on the page. Perhaps add it to the list of links displayed (so the user sees it immediately in the table of links).
            - Maybe auto-select or copy the short URL to clipboard for convenience (optional nice feature).
            - Clear or reset the form fields for next entry.
            - If the API returned an error (like 409 alias exists, 400 bad input, 500 server error, or 401 if token expired):
                * If 401/403, possibly the token expired or is invalid – we might need to prompt login again. For now, log the user out (clear token) and show a message to re-login.
                * If 409 (alias exists), show an error message near the form, e.g., "That custom alias is already taken, please choose another."
                * If 400 (validation), show the validation error (if the API provides a message).
                * If other errors, display a generic error "Failed to shorten URL. Please try again."
            - Ensure the error messages are user-friendly.
        - (Optional) disable the submit button while request in flight to prevent duplicate submissions, re-enable after response.
    - Update the UI list with the new link:
        - We might maintain an array of links in JS state. Initially, after login, we will fetch existing links (Task 10). But here, after creating a new link, we can just append it to the DOM.
        - For example, if using a table, insert a new row with columns: shortCode (or a clickable short URL), original URL (maybe truncated if long), and possibly a "created at" or "by" if needed.
        - The short URL can be displayed as a clickable link (`<a href="shortUrl" target="_blank">shortUrl</a>`) so the admin can test it immediately.
    - **Testing:**
        - After implementing Task 10 (list retrieval), test the full flow:
            * Log in, fill the form with a URL (and maybe alias), click shorten.
            * Observe the network request in browser dev tools to ensure it went out with correct headers and payload.
            * Simulate responses: you might temporarily point `API_BASE_URL` to a mock server or use the deployed API if available. If not yet deployed, you could test by running the local backend in parallel (if we set that up in Task 11) so that `localhost:3000/api/links` works.
            * Check that on success, the new link appears in the list and any success message is shown.
            * Test with an alias that is already in list (if our local allows duplicates currently, or simulate a conflict response) to see error handling.
            * Test with an invalid URL (like "abcd") to see if our front-end validation catches it or if the backend returns 400 and we display error.
        - Ensure that the ID token is sent and accepted. If the token expires (Google ID tokens usually expire after 1 hour), our UI should ideally detect that (calls will start failing with 401). We might not implement refresh in this MVP; instruct users to sign in again by refreshing the page if needed. In future, could integrate refresh tokens or just rely on them signing in again.
    - **Outcome:** The admin UI can create new short links seamlessly. This covers the main user story for the admin: input a URL, get a short link. The user feedback loop is complete. Next, we will implement loading the existing links upon login.

10. **Admin Frontend - Display Existing Links** – *Goal: Fetch and show the list of all short links on the admin UI when logged in.*
    - When the user signs in (in the callback after token received, Task 8), or alternatively when the page loads in an already-signed-in state, we want to fetch the list of links from the API.
    - In the `handleCredentialResponse` (or after showing the main interface), add a call to GET `/api/links`:
        - Use `fetch(API_BASE_URL + '/api/links', { method: 'GET', headers: { Authorization: 'Bearer ' + idToken } })`.
        - On response, if 200 OK, parse the JSON. It should contain an array of link objects (as implemented in Task 7).
        - Populate the links table or list in the UI:
            * Iterate over the array, and for each link, create a table row or list item displaying the short URL and original URL. Possibly also show who created it and when (depending on what data we have; at least we have createdAt).
            * Format the `createdAt` timestamp to a readable format (e.g. local date/time string) if showing.
            * Provide a clickable short URL (similar to above).
            * You might also include a "copy" button for each short link to copy the URL to clipboard (optional enhancement).
        - If the list is large, consider just showing the first N or making the section scrollable. But likely it’s manageable.
        - If the GET request returns an error (like 401 if token invalid), handle similar to above: perhaps require re-auth. If 500, show an error message "Unable to load links."
    - It might be good to show a loading indicator while fetching, especially if it takes a moment. Possibly show "Loading links..." text until the fetch completes.
    - Also, handle the case of no links yet: display a friendly message "No short URLs created yet." if the list is empty.
    - **Testing:**
        - After signing in, verify that the network call to GET is made and the list populates.
        - You can test with dummy data: perhaps manually insert some items in DynamoDB or if using a mock server, return a static list to ensure UI renders correctly.
        - Test that the list updates when a new link is created via the form (we already append the new link in Task 9, but ensure no duplication: maybe decide if you will refetch the entire list after creating a link or just append. Either way should be fine. Simpler to just append to avoid another API call).
        - If possible, test with multiple users (or at least multiple entries with different createdBy) to see if we want to highlight the creator. But since one domain, maybe not needed to show the email in UI.
    - **Outcome:** Upon login, the admin user immediately sees all existing short links and can browse or copy them. This provides context and confirms the system’s stored data. At this stage, the core functionalities (shorten, redirect, view links) are all implemented both backend and frontend.

11. **Local Testing and Self-Hosted Mode** – *Goal: Ensure the application can run locally for development and testing.*
    - Implement a simple local server that ties everything together for testing without AWS:
        - Option A (Node/Express): Create an `index.js` (or similar) in a dev folder that uses Express to mimic API Gateway:
            * Install express (`npm install express`).
            * In the script, import the core logic from the Lambda handlers (we should refactor the Lambda code slightly so that their logic can be called programmatically). For example, export the handler functions or better, factor out the actual logic into functions that accept parameters and call them from both the Lambda and this server.
            * Define routes:
                - `app.post('/api/links', express.json(), async (req,res) => { ... }` to call the createLink handler logic. For the `req`, you have `req.headers.authorization` for token, `req.body` for payload. You might call `verifyGoogleToken` (possibly we need credentials or a mock for Google – maybe in local dev, skip actual verification or allow a test token).
                - One approach: For local ease, you could disable actual Google token verification and simply trust any token or have a dummy secret. But that diverges from real behavior. Alternatively, instruct developers to obtain a Google token manually. A compromise: allow an environment flag like `DEV_MODE=true` which if set, will skip verifying the token and just accept any token as long as it contains an email ending with allowed domain (we can decode without verifying signature in dev). This speeds testing without Google overhead.
                - `app.get('/api/links', ... )` for list: call list logic.
                - `app.get('/:shortCode', ... )` for redirect: call redirect logic, and then perform `res.redirect(originalUrl)` if found, or `res.status(404).send("Not found")`.
            * Use an in-memory store instead of Dynamo:
                - For dev mode, you can have a global object or Map for ShortLinks, and a counter variable for the ID.
                - Implement functions similar to data access but using this in-memory store. Or leverage dependency injection: modify the data layer to be able to use different backends. For instance, the data layer could check an environment flag and either use DynamoDB or a simple in-memory structure.
                - Since our data layer is likely tied to Dynamo, perhaps easier: create a separate stub implementation for local:
                    + `let linksStore = {}; let counterVal = 1000;`
                    + `getLink(code)` -> return linksStore[code] or null.
                    + `createLink(entry)` -> if linksStore[code] exists, throw error; else store it.
                    + `listLinks()` -> return Object.values(linksStore).
                    + `incrementCounter()` -> return ++counterVal.
                - Use these in the Express handlers if in DEV_MODE.
            * Start the server on a port (3000).
        - Option B (Python/Flask): Similar approach using Flask routes calling the Python logic.
        - Option C: Use AWS SAM CLI:
            * Write a `template.yaml` for SAM with the resources and run `sam local start-api`. This will serve the APIs on localhost. This requires Docker etc., might be heavier but closer to real. However, since we already wrote a lot of code, using Express might be simpler for iterative dev.
    - Document in a README how to run locally (install dependencies, run `node devServer.js` or so, and open the frontend via a simple file or local http server).
    - **Test locally:**
        - Start the local server. Open the admin HTML (maybe just open the file or serve it via `http-server` on port 8080).
        - Ideally, host the frontend also on the same origin to avoid CORS in local (e.g. place the built frontend files in a static directory in Express and serve them on `/` route). This way, `http://localhost:3000` can serve both the UI and the API (mimicking CloudFront unified domain). This simplifies local dev (no CORS issues or having to configure CORS for localhost).
            * For example, in Express do: `app.use(express.static('frontend'))` to serve static files, so `http://localhost:3000/index.html` is accessible. Then the same origin calls to `/api/links` will work.
        - Go through the whole flow: sign in (for local, Google sign-in should still work if the OAuth client is set to allow localhost origin). Get a token, the UI calls the local endpoints, the Express handlers use in-memory store, and verify things:
            * Shorten a URL -> check logs or data structure that it was stored, UI shows result.
            * Click the short link -> it should redirect via Express route.
            * List links -> should show what was created.
        - If not testing Google in local (maybe complicated to set up OAuth for localhost), you could temporarily bypass auth as mentioned. E.g., in dev mode, skip verify and just assume `req.headers.authorization` contains a dummy token with email. Or manually copy a token from a production login and paste it in an Authorization header via Postman.
        - This testing ensures our logic works end-to-end and helps catch any integration issues (like CORS, path mismatches, etc.) before deploying.
    - **Outcome:** A developer can run the entire application locally, which greatly eases testing and debugging. It also provides a path to run this app in a self-hosted scenario (though for production self-host, one would need to consider persistence beyond memory, but that’s beyond our current scope). At this point, we are confident in the functionality.

12. **Infrastructure as Code & Deployment** – *Goal: Deploy the solution to AWS using a repeatable process.*
    - Finalize the AWS CloudFormation/SAM template or Serverless Framework configuration to deploy all components:
        - **DynamoDB Tables:** Define `ShortLinks` and `Counters` tables (through AWS::DynamoDB::Table resources in CloudFormation). For `Counters`, initialize with a seed item:
            * CloudFormation doesn’t natively insert data, so instead we might run a one-time Lambda (or just manually insert via AWS console after deploy). Alternatively, we can set a default in our create function: if incrementCounter fails because item doesn’t exist, handle the exception by creating the item with initial value. Simpler: ensure to manually create a "global" counter item with value at least 0 if needed.
        - **Lambda Functions:**
            * Package the code (SAM can point to code in a subfolder and build, or Serverless can bundle automatically).
            * For each function (CreateLinkFunction, ListLinksFunction, RedirectFunction), set runtime (e.g. Node 18), handler, memory (128MB should suffice, maybe 256MB for crypto in token verification just in case), timeout (a few seconds is enough).
            * Set environment variables for each: `ALLOWED_DOMAIN`, `GOOGLE_OAUTH_CLIENT_ID`, table names, etc. Also, if using the counter, maybe an env var for counter name (but we can just use "global" in code).
        - **API Gateway (HTTP API):**
            * Define routes:
                - GET `/api/links` -> ListLinksFunction (with JWT verification done in function, so no authorizer).
                - POST `/api/links` -> CreateLinkFunction.
                - GET `/{proxy+}` (or specifically a greedy path to catch anything not starting with `/api`) -> RedirectFunction. We must be careful that it doesn’t catch `/api` paths. In HTTP API, we might define a route like GET `/ {proxy+}` and then explicitly have the others, which should work since `/api/links` will take precedence for that path.
                - Alternatively, use two stages or domain mappings. But simplest is one API with route prefix for API and catch-all for redirect. If using Serverless framework, one can define an HTTP event with `path: /{shortCode}` for redirect and ensure it doesn’t collide with /api.
            * Enable CORS on the `/api` routes: HTTP API allows a simple CORS config (allowed origins, headers, etc.). We can set `AllowOrigins` to the domain of the admin site (or `*` for now if security is less concern since only authorized calls succeed anyway). Allow `Authorization` header, allow methods GET,POST.
        - **S3 Bucket for Frontend:**
            * Define an S3 bucket resource (AWS::S3::Bucket). Set a bucket policy or PublicAccessBlock to allow public read of objects (if not using CloudFront). If using CloudFront, we keep bucket private and let CloudFront fetch.
            * Optionally, enable website hosting on it (so it has a website endpoint, especially if not using CloudFront).
        - **(Optional) CloudFront Distribution:**
            * If we want the unified domain: define a CloudFront distribution with:
                - Origin for S3 (frontend bucket).
                - Origin for API Gateway (there is a slightly complex setup to call APIGW from CF; often done via custom domain for APIGW or using regional APIGW and domain name).
                - Cache behaviors: one for path pattern `/admin/*` -> S3 origin, another for `/api/*` and maybe `/api/*` -> API Gateway origin, and default behavior for `/*` -> either API or S3 depending on design (in AWS blog they stored redirects in S3, but in our design redirects are handled by API too).
                - Actually, in our case, since redirect is via API, we would route default `GET /<shortCode>` to API as well. So we might not need S3 for redirect objects.
                - So CloudFront usage might be just to serve the static files and forward everything else to API Gateway. If so, then:
                    * Behavior 1: Path `/admin/*` -> S3 (for HTML/JS files).
                    * Behavior 2: Path `/api/*` -> API Gateway.
                    * Behavior 3: Default (`/*` which covers short codes since they won't have /api or /admin) -> API Gateway as well (so CloudFront will forward requests like `/XYZ123` to the API Gateway, which will trigger the redirect Lambda).
                - This would unify everything under one domain (like `short.company.com`). We need an ACM certificate for that domain and a Route53 CNAME to the CloudFront distribution.
                - This is a bit complex to set up in CloudFormation but doable. If time is limited, we may skip actual CloudFront deployment and just note it as future.
            * For initial deployment, we might skip CloudFront and simply use:
                - A CloudFormation output for the API Gateway URL (or custom domain if we set one).
                - The S3 static website URL for the frontend (or instruct to open S3 site).
                - Then the admin UI would call the APIGW URL (with CORS configured).
        - **IAM Roles:** For each Lambda, define an IAM role (AWS::IAM::Role) with necessary policies:
            * DynamoDB policy to allow:
                - For CreateLinkFunction: `dynamodb:UpdateItem` on `Counters` (for increment), `dynamodb:PutItem` on `ShortLinks`, `dynamodb:ConditionCheckItem` if used for conditional (or just let PutItem with condition fail).
                - For ListLinksFunction: `dynamodb:Scan` or `dynamodb:Query` on `ShortLinks`.
                - For RedirectFunction: `dynamodb:GetItem` on `ShortLinks`.
            * Also allow CloudWatch Logs writes for all Lambdas (usually AWSLambdaBasicExecutionRole managed policy).
        - **Deployment process:** Use AWS SAM CLI or `sls deploy` if Serverless framework. Ensure to include environment variables and correct replacements (e.g., pass the domain name and Google Client ID as parameters or hardcode if single environment).
        - Verify template by deploying to a dev/test stack:
            * After deployment, note the API endpoint and test the functions:
                + Use curl or Postman to call the POST create (with a valid Google token) to see if it stores in DynamoDB. (This requires obtaining a Google token manually or hosting the frontend).
                + Alternatively, deploy the frontend: upload `index.html` and assets to the S3 bucket (this can be done manually or automated by S3 sync in a script).
                + Open the hosted admin page (e.g., http://<bucket-name>.s3-website.region.amazonaws.com/admin.html if that's the entry point) – ensure your Google OAuth client allowed that domain or use http://localhost with a local file.
                + Test the full flow in the real environment.
            * Fix any issues (CORS problems, missing permissions, etc.) as observed.
    - **Outcome:** The entire application is deployed on AWS. We have infrastructure as code that can be reused for staging/production. The admin interface is available (possibly at an S3 URL or a friendly domain if configured), and the short links are functional at the API’s domain or custom domain.

13. **Testing, Monitoring, and Optimization** – *Goal: Final validation of the system and setup of monitoring.*
    - **Integration Testing:** Perform end-to-end tests on the deployed environment:
        - Create a few short links using the admin UI. Then try accessing them (in different browsers or via curl) to confirm redirection works for public users.
        - Test edge cases: try a short code that doesn’t exist -> ensure you get a 404 response. Try creating a link with a custom alias that already exists in DB (you can manually attempt the same alias twice) -> the second time should yield an error.
        - If possible, test with another Google account not in the domain to ensure they cannot log in or cannot create links (they might still get a token if using a personal account but our backend should reject it).
    - **Security check:** Ensure that without a token, the API calls are indeed protected. e.g. curl the POST endpoint without auth, should get 401. With an invalid token, also rejected.
    - **Performance check:** Although likely fine, consider the cold start of Lambdas – ensure subsequent calls are quick. If any noticeable latency in redirect, consider enabling Provisioned Concurrency on redirect Lambda for production (especially if traffic is steady). At low volumes, not needed.
    - **Monitoring:** Set up basic logging and monitoring:
        - CloudWatch Logs: Each Lambda by default logs to CloudWatch. We should make sure to log important events (like errors with context, or successful creation logs with shortCode maybe).
        - Setup CloudWatch Alarms if needed: for example, on Lambda errors > X, or DynamoDB throttles (shouldn't happen in on-demand).
        - If using CloudFront, enable standard logging or CloudFront metrics to see requests.
        - Possibly integrate AWS X-Ray for tracing if deep analysis required (not essential now).
    - **Cleanup and Documentation:**
        - Remove any dev-only allowances (like if we had a dev mode skipping auth, ensure it's off in prod).
        - Write a **README** documenting how to deploy and configure:
            * How to set up Google OAuth client (the steps to register the app and get client ID).
            * How to deploy the CloudFormation/SAM (commands to run).
            * Configuration parameters (domain, etc.).
            * Usage instructions for admin users.
        - Document future improvements:
            * e.g., adding a delete link feature, analytics on link clicks (we could log each redirect or integrate with CloudFront logs or an AWS Lambda@Edge for counting).
            * Possibly mention migrating to a fully “functionless” approach where API Gateway directly writes to DynamoDB (as an exploration), or using CloudFront Functions for redirect (if low-latency global redirect needed).
            * But clarify those are beyond current scope.
    - **Outcome:** The system is thoroughly tested and documented. The team (or AI agents) can confidently hand over the solution. The application should meet the original requirements: internal users can create short URLs (with authentication) and anyone can use those short URLs to reach the intended destinations, all running on a scalable, serverless infrastructure. The design is modular and uses modern best practices, positioning the project for easy maintenance and extension.
