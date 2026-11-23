//! lambda-redirect — AWS Lambda entrypoint for public shortlink redirects.
//!
//! Purpose
//! - Handle API Gateway HTTP API (v2) events.
//! - Resolve `/:slug` via the `LinkService` backed by the DynamoDB adapter.
//! - Return `307 Temporary Redirect` with `Location` header on success; map
//!   domain errors to sensible HTTP codes for API Gateway responses.
//!
//! Special URL suffixes:
//! - `/{slug}+` — Preview page with link info instead of redirect
//! - `/{slug}.qr` — QR code image (SVG) for the short URL
//! - `/{slug}+.qr` — QR code that points to the preview page
//!
//! Notes
//! - This crate depends only on the `domain` and `aws-dynamo` adapter for data.
//! - It initializes minimal `tracing` logging compatible with Lambda CloudWatch.

use aws_dynamo::DynamoRepo;
use domain::service::LinkService;
use domain::slug::Base62SlugGenerator;
use domain::{Clock, LinkRepository, Slug};
use http_common::lambda::resp;
use lambda_http::http::header::{HeaderValue, CACHE_CONTROL};
use lambda_http::{run, service_fn, Body, Error, Request, Response};
use qrcode::render::svg;
use qrcode::QrCode;
use std::sync::Arc;
use tracing::{error, info, warn};
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

/// Wrap a response with CORS headers (allow all origins for public endpoints)
fn with_cors(mut response: Response<Body>) -> Response<Body> {
    use lambda_http::http::header::ACCESS_CONTROL_ALLOW_ORIGIN;
    let headers = response.headers_mut();
    headers.insert(ACCESS_CONTROL_ALLOW_ORIGIN, HeaderValue::from_static("*"));
    response
}

fn apply_cache_policy(mut response: Response<Body>) -> Response<Body> {
    response
        .headers_mut()
        .entry(CACHE_CONTROL)
        .or_insert(HeaderValue::from_static("no-store"));
    response
}

#[derive(Clone)]
struct AppState {
    svc: Arc<LinkService<DynamoRepo, Base62SlugGenerator, StdClock>>,
}

#[derive(Clone)]
struct StdClock;
impl Clock for StdClock {
    fn now(&self) -> std::time::SystemTime {
        std::time::SystemTime::now()
    }
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    init_tracing();
    // Build repo from env; if it fails, crash early to surface misconfiguration.
    let repo = DynamoRepo::from_env().map_err(|e| format!("dynamo init error: {e}"))?;
    let state = AppState {
        svc: Arc::new(LinkService::new(
            repo,
            Base62SlugGenerator::new(1),
            StdClock,
        )),
    };

    let handler = service_fn(move |req: Request| {
        let st = state.clone();
        async move { handle_request(&st.svc, req).await }
    });
    run(handler).await?;
    Ok(())
}

fn init_tracing() {
    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::registry()
        .with(env_filter)
        .with(fmt::layer().with_target(true).with_writer(std::io::stdout))
        .init();
}

/// Request mode based on URL suffix
enum RequestMode {
    Redirect,
    Preview,                          // slug+
    QrCode,                           // slug.qr
    CountdownRedirect { delay: u32 }, // Auto-redirect with countdown
}

async fn handle_request(
    svc: &LinkService<impl LinkRepository, Base62SlugGenerator, StdClock>,
    req: Request,
) -> Result<Response<Body>, Error> {
    let raw_path = req.uri().path();
    // API Gateway HTTP API includes stage prefix in rawPath (e.g., /dev/abc123)
    // Strip the stage prefix if present by taking only the last path segment
    let slug_str = raw_path.rsplit('/').next().unwrap_or("");
    let query = req.uri().query().unwrap_or_default();

    // Expect a non-empty slug
    if slug_str.is_empty() {
        warn!(path = %raw_path, "empty slug in redirect");
        return Ok(apply_cache_policy(resp(
            400,
            None,
            Some(http_common::json_err("bad_request")),
        )));
    }

    // Determine request mode based on suffix
    // Supports: /slug, /slug+, /slug.qr, /slug+.qr
    let (actual_slug_str, mode, qr_suffix) = if let Some(stripped) = slug_str.strip_suffix("+.qr") {
        // QR code for preview URL
        (stripped, RequestMode::QrCode, "+")
    } else if let Some(stripped) = slug_str.strip_suffix(".qr") {
        // QR code for direct URL
        (stripped, RequestMode::QrCode, "")
    } else if let Some(stripped) = slug_str.strip_suffix('+') {
        (stripped, RequestMode::Preview, "")
    } else {
        (slug_str, RequestMode::Redirect, "")
    };

    // Track if this is a QR request (needs CORS headers for cross-origin fetch)
    let is_qr_request = matches!(mode, RequestMode::QrCode);

    let slug = match Slug::new(actual_slug_str.to_string()) {
        Ok(s) => s,
        Err(_) => {
            warn!(slug = %actual_slug_str, "invalid slug");
            let r =
                apply_cache_policy(resp(400, None, Some(http_common::json_err("invalid_slug"))));
            return Ok(if is_qr_request { with_cors(r) } else { r });
        }
    };

    // Build the short URL for QR code generation
    let host = req
        .headers()
        .get("host")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("example.com");
    let short_url = format!("https://{}/{}{}", host, slug.as_str(), qr_suffix);

    // Get the full link to check is_active, expiration, and other status
    let response = match svc.get(&slug) {
        Ok(Some(link)) => {
            let now = std::time::SystemTime::now();

            // Check if link is deleted (soft delete)
            if link.deleted_at.is_some() {
                warn!(slug = %slug.as_str(), "link deleted");
                resp(404, None, Some(http_common::json_err("not_found")))
            }
            // Check if link has expired
            else if link.is_expired(now) {
                warn!(slug = %slug.as_str(), "link expired");
                resp(410, None, Some(http_common::json_err("gone")))
            }
            // Check if link is scheduled for future activation
            else if link.activate_at.map(|at| now < at).unwrap_or(false) {
                warn!(slug = %slug.as_str(), "link not yet active");
                resp(404, None, Some(http_common::json_err("not_found")))
            }
            // Check if link is active
            else if !link.is_active {
                warn!(slug = %slug.as_str(), "link inactive");
                resp(404, None, Some(http_common::json_err("not_found")))
            } else {
                // Determine actual mode - check if link has redirect_delay
                let actual_mode = match mode {
                    RequestMode::Redirect => {
                        // Check if link has redirect_delay configured
                        if let Some(delay) = link.redirect_delay {
                            if delay > 0 {
                                RequestMode::CountdownRedirect { delay }
                            } else {
                                RequestMode::Redirect
                            }
                        } else {
                            RequestMode::Redirect
                        }
                    }
                    other => other,
                };

                match actual_mode {
                    RequestMode::Preview => {
                        info!(slug = %slug.as_str(), "preview page");
                        render_preview_page(&link, &short_url)
                    }
                    RequestMode::QrCode => {
                        info!(slug = %slug.as_str(), "qr code");
                        render_qr_code(&short_url)
                    }
                    RequestMode::CountdownRedirect { delay } => {
                        info!(slug = %slug.as_str(), delay = delay, "countdown redirect page");
                        let target_url = redirect_target_url(&link.original_url, query);
                        render_countdown_page(&link, &target_url, delay)
                    }
                    RequestMode::Redirect => {
                        // Fire-and-forget click increment (don't fail redirect on counter error)
                        if let Err(e) = svc.increment_click(&slug) {
                            warn!(slug = %slug.as_str(), err = ?e, "click increment failed");
                        }
                        info!(slug = %slug.as_str(), "resolve ok");
                        let target_url = redirect_target_url(&link.original_url, query);
                        resp(307, Some(("Location", target_url)), None)
                    }
                }
            }
        }
        Ok(None) => {
            warn!(slug = %slug.as_str(), "not found");
            resp(404, None, Some(http_common::json_err("not_found")))
        }
        Err(e) => {
            error!(slug = %slug.as_str(), err = ?e, "resolve error");
            resp(500, None, Some(http_common::json_err("error")))
        }
    };

    let response = apply_cache_policy(response);
    // Wrap QR requests with CORS headers for cross-origin fetch from admin UI
    Ok(if is_qr_request {
        with_cors(response)
    } else {
        response
    })
}

fn redirect_target_url(original_url: &str, query: &str) -> String {
    let incoming: Vec<_> = form_urlencoded::parse(query.as_bytes())
        .filter(|(key, _)| key.starts_with("utm_"))
        .collect();
    if incoming.is_empty() {
        return original_url.to_owned();
    }

    let (before_fragment, fragment) = original_url
        .find('#')
        .map_or((original_url, ""), |index| original_url.split_at(index));
    let (base, existing_query) = before_fragment
        .split_once('?')
        .unwrap_or((before_fragment, ""));

    // Preserve the original encoding of any parameters we keep.
    let retained_query = existing_query
        .split('&')
        .filter(|pair| {
            form_urlencoded::parse(pair.as_bytes())
                .next()
                .is_some_and(|(key, _)| {
                    incoming
                        .iter()
                        .all(|(incoming_key, _)| *incoming_key != key)
                })
        })
        .collect::<Vec<_>>()
        .join("&");
    let merged_query = form_urlencoded::Serializer::new(retained_query)
        .extend_pairs(incoming)
        .finish();
    format!("{base}?{merged_query}{fragment}")
}

fn render_qr_code(url: &str) -> Response<Body> {
    match QrCode::new(url.as_bytes()) {
        Ok(code) => {
            let svg_string = code
                .render()
                .min_dimensions(200, 200)
                .dark_color(svg::Color("#000000"))
                .light_color(svg::Color("#ffffff"))
                .build();

            Response::builder()
                .status(200)
                .header("Content-Type", "image/svg+xml")
                .header("Cache-Control", "public, max-age=86400")
                .body(Body::from(svg_string))
                .expect("response build")
        }
        Err(_) => Response::builder()
            .status(500)
            .header("Content-Type", "application/json")
            .body(Body::from(r#"{"error":"qr_generation_failed"}"#))
            .expect("response build"),
    }
}

fn render_preview_page(link: &domain::ShortLink, _short_url: &str) -> Response<Body> {
    let created_at = http_common::system_time_to_rfc3339(link.created_at);
    let updated_at = link.updated_at.map(http_common::system_time_to_rfc3339);
    let expires_at = link.expires_at.map(http_common::system_time_to_rfc3339);

    let updated_html = if let Some(updated) = updated_at {
        format!(
            r#"<tr><td>Last Modified</td><td>{}</td></tr>"#,
            html_escape(&updated)
        )
    } else {
        String::new()
    };

    let expires_html = if let Some(expires) = expires_at {
        format!(
            r#"<tr><td>Expires</td><td>{}</td></tr>"#,
            html_escape(&expires)
        )
    } else {
        String::new()
    };

    let html = format!(
        r##"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Link Preview - {slug}</title>
    <style>
        * {{ margin: 0; padding: 0; box-sizing: border-box; }}
        body {{
            font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
            background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
            min-height: 100vh;
            display: flex;
            align-items: center;
            justify-content: center;
            padding: 20px;
        }}
        .card {{
            background: white;
            border-radius: 16px;
            box-shadow: 0 25px 50px -12px rgba(0,0,0,0.25);
            max-width: 500px;
            width: 100%;
            overflow: hidden;
        }}
        .header {{
            background: #f8fafc;
            padding: 24px;
            border-bottom: 1px solid #e2e8f0;
        }}
        .header h1 {{
            font-size: 1.25rem;
            color: #334155;
            margin-bottom: 4px;
        }}
        .header .slug {{
            font-family: monospace;
            font-size: 1.5rem;
            color: #6366f1;
            font-weight: 600;
        }}
        .content {{
            padding: 24px;
        }}
        table {{
            width: 100%;
            border-collapse: collapse;
        }}
        td {{
            padding: 12px 0;
            border-bottom: 1px solid #f1f5f9;
        }}
        td:first-child {{
            color: #64748b;
            font-size: 0.875rem;
            width: 120px;
        }}
        td:last-child {{
            color: #1e293b;
            word-break: break-all;
        }}
        tr:last-child td {{
            border-bottom: none;
        }}
        .destination {{
            background: #f8fafc;
            padding: 16px;
            border-radius: 8px;
            margin-top: 16px;
        }}
        .destination-label {{
            font-size: 0.75rem;
            color: #64748b;
            text-transform: uppercase;
            letter-spacing: 0.05em;
            margin-bottom: 8px;
        }}
        .destination-url {{
            color: #6366f1;
            word-break: break-all;
            font-size: 0.9rem;
        }}
        .actions {{
            padding: 24px;
            background: #f8fafc;
            border-top: 1px solid #e2e8f0;
        }}
        .btn {{
            display: block;
            width: 100%;
            padding: 14px 24px;
            background: #6366f1;
            color: white;
            text-decoration: none;
            text-align: center;
            border-radius: 8px;
            font-weight: 600;
            transition: background 0.2s;
        }}
        .btn:hover {{
            background: #4f46e5;
        }}
        .clicks {{
            font-size: 1.5rem;
            font-weight: 600;
            color: #6366f1;
        }}
    </style>
</head>
<body>
    <div class="card">
        <div class="header">
            <h1>Link Preview</h1>
            <div class="slug">{slug}</div>
        </div>
        <div class="content">
            <table>
                <tr>
                    <td>Clicks</td>
                    <td><span class="clicks">{clicks}</span></td>
                </tr>
                <tr>
                    <td>Created</td>
                    <td>{created}</td>
                </tr>
                {updated_row}
                {expires_row}
            </table>
            <div class="destination">
                <div class="destination-label">Destination URL</div>
                <div class="destination-url">{url}</div>
            </div>
        </div>
        <div class="actions">
            <a href="{url}" class="btn">Continue to Destination</a>
        </div>
    </div>
</body>
</html>"##,
        slug = html_escape(link.slug.as_str()),
        clicks = link.click_count,
        created = html_escape(&created_at),
        updated_row = updated_html,
        expires_row = expires_html,
        url = html_escape(&link.original_url),
    );

    Response::builder()
        .status(200)
        .header("Content-Type", "text/html; charset=utf-8")
        .body(Body::from(html))
        .expect("response build")
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#x27;")
}

fn render_countdown_page(
    link: &domain::ShortLink,
    target_url: &str,
    delay_seconds: u32,
) -> Response<Body> {
    let created_at = http_common::system_time_to_rfc3339(link.created_at);
    let description = link.description.as_deref().unwrap_or("");

    let description_html = if !description.is_empty() {
        format!(
            r#"<div class="description">{}</div>"#,
            html_escape(description)
        )
    } else {
        String::new()
    };

    // Escape '<' so the string cannot close the surrounding script element.
    let url_js = serde_json::to_string(target_url)
        .expect("redirect target serialization")
        .replace('<', "\\u003c");

    let html = format!(
        r##"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Redirecting - {slug}</title>
    <style>
        * {{ margin: 0; padding: 0; box-sizing: border-box; }}
        body {{
            font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
            background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
            min-height: 100vh;
            display: flex;
            align-items: center;
            justify-content: center;
            padding: 20px;
        }}
        .card {{
            background: white;
            border-radius: 16px;
            box-shadow: 0 25px 50px -12px rgba(0,0,0,0.25);
            max-width: 500px;
            width: 100%;
            overflow: hidden;
        }}
        .header {{
            background: #f8fafc;
            padding: 24px;
            border-bottom: 1px solid #e2e8f0;
            text-align: center;
        }}
        .header h1 {{
            font-size: 1.25rem;
            color: #334155;
            margin-bottom: 8px;
        }}
        .countdown {{
            font-size: 3rem;
            font-weight: 700;
            color: #6366f1;
        }}
        .countdown-label {{
            font-size: 0.875rem;
            color: #64748b;
            margin-top: 4px;
        }}
        .content {{
            padding: 24px;
        }}
        .description {{
            background: #f0f9ff;
            border: 1px solid #bae6fd;
            border-radius: 8px;
            padding: 12px 16px;
            color: #0369a1;
            margin-bottom: 16px;
        }}
        table {{
            width: 100%;
            border-collapse: collapse;
        }}
        td {{
            padding: 12px 0;
            border-bottom: 1px solid #f1f5f9;
        }}
        td:first-child {{
            color: #64748b;
            font-size: 0.875rem;
            width: 120px;
        }}
        td:last-child {{
            color: #1e293b;
            word-break: break-all;
        }}
        tr:last-child td {{
            border-bottom: none;
        }}
        .destination {{
            background: #f8fafc;
            padding: 16px;
            border-radius: 8px;
            margin-top: 16px;
        }}
        .destination-label {{
            font-size: 0.75rem;
            color: #64748b;
            text-transform: uppercase;
            letter-spacing: 0.05em;
            margin-bottom: 8px;
        }}
        .destination-url {{
            color: #6366f1;
            word-break: break-all;
            font-size: 0.9rem;
        }}
        .actions {{
            padding: 24px;
            background: #f8fafc;
            border-top: 1px solid #e2e8f0;
            display: flex;
            gap: 12px;
        }}
        .btn {{
            flex: 1;
            padding: 14px 24px;
            text-decoration: none;
            text-align: center;
            border-radius: 8px;
            font-weight: 600;
            transition: all 0.2s;
            cursor: pointer;
            border: none;
            font-size: 1rem;
        }}
        .btn-primary {{
            background: #6366f1;
            color: white;
        }}
        .btn-primary:hover {{
            background: #4f46e5;
        }}
        .btn-secondary {{
            background: #e2e8f0;
            color: #475569;
        }}
        .btn-secondary:hover {{
            background: #cbd5e1;
        }}
        .clicks {{
            font-size: 1.5rem;
            font-weight: 600;
            color: #6366f1;
        }}
        .cancelled {{
            display: none;
            background: #fef2f2;
            border: 1px solid #fecaca;
            border-radius: 8px;
            padding: 12px 16px;
            color: #dc2626;
            text-align: center;
            margin-bottom: 16px;
        }}
        .cancelled.show {{
            display: block;
        }}
    </style>
</head>
<body>
    <div class="card">
        <div class="header">
            <h1>Redirecting to destination...</h1>
            <div class="countdown" id="countdown">{delay}</div>
            <div class="countdown-label" id="countdown-label">seconds</div>
        </div>
        <div class="content">
            <div class="cancelled" id="cancelled">
                Redirect cancelled. You can navigate manually using the button below.
            </div>
            {description_row}
            <table>
                <tr>
                    <td>Short Link</td>
                    <td><strong>{slug}</strong></td>
                </tr>
                <tr>
                    <td>Clicks</td>
                    <td><span class="clicks">{clicks}</span></td>
                </tr>
                <tr>
                    <td>Created</td>
                    <td>{created}</td>
                </tr>
            </table>
            <div class="destination">
                <div class="destination-label">Destination URL</div>
                <div class="destination-url">{url}</div>
            </div>
        </div>
        <div class="actions">
            <button class="btn btn-secondary" id="cancel-btn" onclick="cancelRedirect()">Cancel</button>
            <a href="{url}" class="btn btn-primary" id="continue-btn">Continue Now</a>
        </div>
    </div>
    <script>
        let countdown = {delay};
        let cancelled = false;
        const targetUrl = {url_js};

        function updateCountdown() {{
            if (cancelled) return;

            if (countdown <= 0) {{
                window.location.href = targetUrl;
                return;
            }}

            document.getElementById('countdown').textContent = countdown;
            document.getElementById('countdown-label').textContent = countdown === 1 ? 'second' : 'seconds';
            countdown--;
            setTimeout(updateCountdown, 1000);
        }}

        function cancelRedirect() {{
            cancelled = true;
            document.getElementById('countdown').textContent = '—';
            document.getElementById('countdown-label').textContent = 'cancelled';
            document.getElementById('cancel-btn').style.display = 'none';
            document.getElementById('cancelled').classList.add('show');
        }}

        // Start countdown
        setTimeout(updateCountdown, 1000);
    </script>
</body>
</html>"##,
        slug = html_escape(link.slug.as_str()),
        delay = delay_seconds,
        description_row = description_html,
        clicks = link.click_count,
        created = html_escape(&created_at),
        url = html_escape(target_url),
    );

    Response::builder()
        .status(200)
        .header("Content-Type", "text/html; charset=utf-8")
        .body(Body::from(html))
        .expect("response build")
}

// Note: Response builders (resp) and JSON helpers (json_err) are now provided
// by the http-common crate.

#[cfg(test)]
mod tests {
    use super::*;
    use domain::{adapters::memory_repo::InMemoryRepo, ShortLink, UserEmail};
    use serde_json::json;
    use std::time::UNIX_EPOCH;

    async fn request_link(
        original_url: &str,
        delay: Option<u32>,
        path: &str,
        query: &str,
    ) -> Response<Body> {
        let mut link = ShortLink::new(
            Slug::new("sample").unwrap(),
            original_url.to_owned(),
            UNIX_EPOCH,
            UserEmail::new("owner@example.test").unwrap(),
        );
        link.redirect_delay = delay;
        request_fixture(Some(link), path, query).await
    }

    async fn request_fixture(link: Option<ShortLink>, path: &str, query: &str) -> Response<Body> {
        let repo = InMemoryRepo::new();
        if let Some(link) = link {
            repo.put(link).unwrap();
        }
        let svc = LinkService::new(repo, Base62SlugGenerator::new(1), StdClock);

        let event = json!({
            "version": "2.0",
            "rawPath": path,
            "rawQueryString": query,
            "requestContext": {
                "stage": "$default",
                "domainName": "short.example.test",
                "http": { "method": "GET", "path": path }
            },
            "isBase64Encoded": false
        });
        let req = lambda_http::request::from_str(&event.to_string()).unwrap();
        handle_request(&svc, req).await.unwrap()
    }

    #[tokio::test]
    async fn redirect_location_forwards_only_utm_parameters() {
        for (original, query, expected) in [
            (
                "https://example.test/destination#details",
                "utm_source=newsletter&ignored=1&UTM_source=other",
                "https://example.test/destination?utm_source=newsletter#details",
            ),
            (
                "https://example.test/?keep=a%20b&%75tm_source=old&utm_source=older#details?tab=1",
                "%75tm_source=one&utm_source=two&utm_campaign=board%26team%2Bone&keep=other",
                "https://example.test/?keep=a%20b&utm_source=one&utm_source=two&utm_campaign=board%26team%2Bone#details?tab=1",
            ),
            (
                "https://example.test/?utm_source=stored#details",
                "ignored=1",
                "https://example.test/?utm_source=stored#details",
            ),
        ] {
            let response = request_link(original, None, "/sample", query).await;
            assert_eq!(response.status(), 307);
            assert_eq!(response.headers()["location"], expected);
            assert_eq!(response.headers()[CACHE_CONTROL], "no-store");
        }
    }

    #[tokio::test]
    async fn countdown_uses_the_same_target_for_the_link_and_script() {
        let response = request_link(
            "https://example.test/?keep=1&utm_source=old#details",
            Some(1),
            "/sample",
            "utm_source=email%22%26%3C&ignored=1",
        )
        .await;
        assert_eq!(response.status(), 200);
        assert_eq!(response.headers()[CACHE_CONTROL], "no-store");
        let Body::Text(html) = response.body() else {
            panic!("expected HTML countdown response");
        };
        assert!(html.contains(
            r#"href="https://example.test/?keep=1&amp;utm_source=email%22%26%3C#details" class="btn btn-primary" id="continue-btn""#
        ));
        assert!(html.contains(
            r#"const targetUrl = "https://example.test/?keep=1&utm_source=email%22%26%3C#details";"#
        ));
    }

    #[tokio::test]
    async fn public_preview_hides_owner_email() {
        let target = "https://example.test/destination";
        let response = request_link(target, None, "/sample+", "").await;
        assert_eq!(response.status(), 200);
        let Body::Text(html) = response.body() else {
            panic!("expected HTML preview response");
        };
        assert!(html.contains(&format!(r#"href="{target}""#)));
        assert!(!html.contains("owner@example.test"));
    }

    #[tokio::test]
    async fn preview_and_qr_ignore_incoming_parameters() {
        for path in ["/sample+", "/sample.qr", "/sample+.qr"] {
            let original = "https://example.test/?utm_source=stored";
            let baseline = request_link(original, Some(1), path, "").await;
            let tagged = request_link(original, Some(1), path, "utm_source=incoming").await;
            assert_eq!(tagged.status(), 200);
            let cache = if path.ends_with(".qr") {
                "public, max-age=86400"
            } else {
                "no-store"
            };
            assert_eq!(tagged.headers()[CACHE_CONTROL], cache);
            assert_eq!(tagged.headers(), baseline.headers());
            assert_eq!(tagged.body().as_ref(), baseline.body().as_ref());
        }
    }

    #[tokio::test]
    async fn unavailable_links_are_not_cached() {
        let link = ShortLink::new(
            Slug::new("sample").unwrap(),
            "https://example.test".into(),
            UNIX_EPOCH,
            UserEmail::new("owner@example.test").unwrap(),
        );
        let mut inactive = link.clone();
        inactive.is_active = false;
        let mut expired = link;
        expired.expires_at = Some(UNIX_EPOCH);

        for (link, status) in [(None, 404), (Some(inactive), 404), (Some(expired), 410)] {
            let response = request_fixture(link, "/sample", "").await;
            assert_eq!(response.status(), status);
            assert_eq!(response.headers()[CACHE_CONTROL], "no-store");
        }
    }
}
