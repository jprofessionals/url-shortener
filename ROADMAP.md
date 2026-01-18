# URL Shortener Roadmap

Suggested features and improvements for the URL shortening service.

---

## End User Features

### Link Preview Page (Magic Character)
Append `+` to any short URL to view link information instead of redirecting.

**Example:** `https://sc.jpro.dev/abc123+` shows a landing page with:
- Target URL (where it redirects)
- Created date
- Click count
- Last modified date (if edited)
- Creator (optional, configurable)
- "Continue to destination" button

**Implementation:** Add route `/{slug}+` to lambda-redirect that returns HTML page instead of 308.

---

### QR Code Generation
Generate QR codes for short links.

- Auto-generate QR code when link is created
- Download as PNG/SVG from admin panel
- Optionally embed QR in link preview page
- Consider: store QR in S3 or generate on-demand

---

### Link Expiration
Set optional expiration date/time for links.

- Add `expires_at` field to ShortLink
- Redirect returns 410 Gone after expiration
- Admin can set/extend expiration
- Optional: auto-cleanup expired links

---

### Password-Protected Links
Require password to access certain links.

- Add optional `password_hash` field
- Show password prompt page before redirect
- Rate-limit password attempts

---

### Custom Slug Suggestions
When creating a link, suggest available slugs based on the target URL.

- Parse domain/path from URL
- Suggest shortened versions
- Check availability before suggesting

---

### Link Groups / Tags
Organize links with tags or folders.

- Add `tags` field (string array)
- Filter by tag in admin
- Useful for campaigns, projects, etc.

---

## Admin Features

### Analytics Dashboard
Detailed metrics beyond click count.

**Click Timeline:**
- Store click events with timestamp in separate table
- Graph clicks per hour/day/week/month
- Filter by date range

**Geographic Data:**
- Log country/region from CloudFront headers or IP geolocation
- Show click distribution map

**Referrer Tracking:**
- Log `Referer` header
- Show top referrers per link

**Device/Browser Stats:**
- Parse User-Agent
- Show device type breakdown (mobile/desktop)

---

### Bulk Operations
Manage multiple links at once.

- Select multiple links in admin table
- Bulk activate/deactivate
- Bulk delete
- Bulk export to CSV

---

### Search
Full-text search across links.

- Search by slug, target URL, creator
- DynamoDB GSI or OpenSearch for complex queries

---

### Audit Log
Track all changes to links.

- Log create/update/delete events
- Store who made the change and when
- View history per link

---

### API Keys
Allow programmatic access without Google OAuth.

- Generate API keys per user
- Scope keys (read-only, create, full access)
- Rate limiting per key

---

### Webhook Notifications
Notify external systems on events.

- Webhook on link create/update/delete
- Webhook on click (with debounce/batching)
- Configure per-link or global webhooks

---

### Custom Domains per User
Let users bring their own domains.

- Each user can configure their short domain
- Route based on Host header
- SSL via Cloudflare or ACM

---

## Infrastructure Improvements

### CDN Caching for Redirects
Cache redirects at edge for faster response.

- CloudFront in front of redirect Lambda
- Short TTL (e.g., 60s) to balance speed vs freshness
- Invalidate on link update

---

### Click Counting via Kinesis/SQS
Decouple click counting from redirect response.

- Redirect Lambda publishes to Kinesis/SQS
- Separate Lambda aggregates counts
- Reduces redirect latency

---

### Multi-Region Deployment
Deploy to multiple AWS regions for lower latency.

- DynamoDB Global Tables
- Region-aware routing via Route53

---

## Priority Suggestions

| Priority | Feature | Effort | Impact |
|----------|---------|--------|--------|
| 1 | Link Preview Page (`+`) | Low | High |
| 2 | QR Code Generation | Low | Medium |
| 3 | Link Expiration | Low | Medium |
| 4 | Click Timeline Analytics | Medium | High |
| 5 | Bulk Operations | Medium | Medium |
| 6 | Search | Medium | Medium |
| 7 | API Keys | Medium | High |
| 8 | Tags/Groups | Low | Medium |
