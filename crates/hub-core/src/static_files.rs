//! The static file rule, ported from `static_allowed` (`hub/server.py:886-898`), and the
//! per-page Content-Security-Policy computed once at boot.
//!
//! Only top-level files and files directly under `vendor/` with a web extension are served, so a
//! secret placed in the static folder (`.env`, logs, caches, Python sources) answers 404. The same
//! rule serves GET and HEAD (the Python stdlib let `HEAD` bypass the check).
//!
//! The pages are static files with inline `<script>` blocks, so the CSP is written with `sha256`
//! hashes rather than nonces: [`CspSet::scan`] reads each top-level `.html` at boot, hashes the
//! body of every bare `<script>` block, and builds that page's policy. A page edited after boot
//! keeps its boot-time policy, so the drift surfaces as a blocked script in the browser instead of
//! a silent allow.

use axum::body::Body;
use axum::http::{header, HeaderValue, Method, Request, StatusCode};
use axum::response::{IntoResponse, Response};
use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use percent_encoding::percent_decode_str;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tower::ServiceExt;
use tower_http::services::ServeFile;

const STATIC_EXT: [&str; 9] = [
    ".html", ".js", ".css", ".png", ".svg", ".ico", ".woff", ".woff2", ".txt",
];

/// The Google Identity Services script origin (`hub/auth.js:9`), reachable only from `hub.html`.
const GIS: &str = "https://accounts.google.com";
/// The web-font CSS (`hub/theme.js:53`) and the files it points at.
const FONTS_CSS: &str = "https://fonts.googleapis.com";
const FONTS_FILES: &str = "https://fonts.gstatic.com";

/// Every page's policy, in the order the directives are written.
///
/// `frame-src` is `'none'` unless the page frames something: `hub.html` frames the engine pages on
/// this origin and, with `gis`, the One Tap iframe. `worker-src 'self' blob:` covers both Vela
/// worker shapes (`new Worker(same-origin)` and `new Worker(URL.createObjectURL(blob))`).
/// `frame-ancestors` is `'self'`, not `'none'`: `hub.html:274` frames `vela.html`,
/// `lightweight.html` and `klinechart.html` in its own stage, so `'none'` would block the hub's own
/// chart. `'self'` still refuses every other origin, which is the clickjacking rule that matters.
/// `style-src` keeps `'unsafe-inline'`: the charts inject `<style>` elements at runtime, which a
/// hash cannot cover. `script-src` keeps `'unsafe-eval'`: the Pine compiler in `vela.bundle.js`
/// uses `new Function` (named residual risk in SECURITY.md).
fn policy(hashes: &[String], gis: bool, https_origin: bool) -> String {
    let mut script = String::from("'self' 'unsafe-eval'");
    for h in hashes {
        script.push_str(" '");
        script.push_str(h);
        script.push('\'');
    }
    if gis {
        script.push(' ');
        script.push_str(GIS);
    }
    let frame = if gis {
        format!("'self' {GIS}")
    } else {
        "'none'".to_string()
    };
    let connect = if gis {
        format!("'self' {GIS}")
    } else {
        "'self'".to_string()
    };
    let mut p = format!(
        "default-src 'none'; script-src {script}; style-src 'self' 'unsafe-inline' {FONTS_CSS}; \
         font-src 'self' data: {FONTS_FILES}; img-src 'self' data:; connect-src {connect}; \
         frame-src {frame}; worker-src 'self' blob:; object-src 'none'; base-uri 'none'; \
         form-action 'none'; frame-ancestors 'self'"
    );
    if https_origin {
        p.push_str("; upgrade-insecure-requests");
    }
    p
}

/// The bodies of every bare `<script>` block, in document order.
///
/// A byte scan, not an HTML parser: the pages have exactly one shape of inline block (open tag
/// `<script>`, no attributes), and a parser would be a new dependency for a page we own. A block
/// with any attribute is skipped whole, body included, so a `<script>` inside a string cannot be
/// mistaken for a block. Pure.
pub fn inline_scripts(html: &str) -> Vec<&str> {
    let bytes = html.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(at) = find(bytes, i, b"<script") {
        let Some(gt) = find(bytes, at, b">") else {
            break;
        };
        let end = find(bytes, gt, b"</script>");
        let Some(end) = end else { break };
        if &bytes[at..=gt] == b"<script>" {
            // The slice is ASCII delimiters around UTF-8: both offsets are char boundaries.
            out.push(&html[gt + 1..end]);
        }
        i = end + b"</script>".len();
    }
    out
}

/// The first `needle` at or after `from`.
fn find(hay: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    if from >= hay.len() {
        return None;
    }
    hay[from..]
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|p| p + from)
}

/// `sha256-<base64>` of one inline block, the form `script-src` names.
pub fn script_hash(body: &str) -> String {
    let digest = Sha256::digest(body.as_bytes());
    format!("sha256-{}", STANDARD.encode(digest))
}

/// The per-page policies, computed once at boot by [`CspSet::scan`].
///
/// A page edited after boot keeps its boot-time policy: the drifted block is blocked in the browser
/// (fail closed), never silently allowed.
pub struct CspSet {
    default: HeaderValue,
    per_file: HashMap<PathBuf, HeaderValue>,
}

impl CspSet {
    /// Reads every top-level `.html` under `root` and builds its policy. A missing or unreadable
    /// root, or one unreadable page, leaves the default policy in place: never an error, because a
    /// server that refuses to start over a page is worse than a page under the strict policy.
    pub fn scan(root: &Path, https_origin: bool) -> CspSet {
        let default = header(&policy(&[], false, https_origin));
        let mut per_file = HashMap::new();
        let Ok(dir) = std::fs::read_dir(root) else {
            return CspSet { default, per_file };
        };
        let mut files: Vec<PathBuf> = dir
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.is_file() && p.extension().is_some_and(|x| x == "html"))
            .collect();
        files.sort(); // deterministic, so two boots build the same set
        for path in files {
            let Ok(html) = std::fs::read_to_string(&path) else {
                continue;
            };
            let hashes: Vec<String> = inline_scripts(&html).into_iter().map(script_hash).collect();
            // Only `hub.html` loads `auth.js`, the one file that inserts the GIS script.
            let gis = path.file_name().is_some_and(|n| n == "hub.html");
            let Some(name) = path.file_name() else {
                continue;
            };
            per_file.insert(
                PathBuf::from(name),
                header(&policy(&hashes, gis, https_origin)),
            );
        }
        CspSet { default, per_file }
    }

    /// The policy for a file relative to the static root: its own, or the default.
    pub fn header_for(&self, rel: &Path) -> &HeaderValue {
        self.per_file.get(rel).unwrap_or(&self.default)
    }

    /// The strict policy every non-page response carries.
    pub fn default(&self) -> &HeaderValue {
        &self.default
    }
}

fn header(policy: &str) -> HeaderValue {
    HeaderValue::from_str(policy).unwrap_or_else(|_| HeaderValue::from_static("default-src 'none'"))
}

/// The file a request path may serve, relative to the static root, or `None` (404).
///
/// Percent-decodes first, drops the query, maps `/` to `index.html`, refuses any segment that
/// starts with `.` and any backslash, and allows one segment, or `vendor/<file>`, with a web
/// extension. Pure.
pub fn resolve(raw_path: &str) -> Option<PathBuf> {
    let path_only = raw_path.split(['?', '#']).next().unwrap_or("");
    let decoded = percent_decode_str(path_only).decode_utf8().ok()?;
    if decoded.is_empty() || decoded == "/" {
        return Some(PathBuf::from("index.html"));
    }
    if decoded.contains('\\') {
        return None;
    }
    let parts: Vec<&str> = decoded.split('/').filter(|p| !p.is_empty()).collect();
    if parts.iter().any(|p| p.starts_with('.')) {
        return None;
    }
    let allowed_shape = parts.len() == 1 || (parts.len() == 2 && parts[0] == "vendor");
    if !allowed_shape {
        return None;
    }
    let last = parts.last()?.to_ascii_lowercase();
    if !STATIC_EXT.iter().any(|ext| last.ends_with(ext)) {
        return None;
    }
    Some(parts.iter().collect())
}

/// Serves `resolve(path)` from `root` for GET and HEAD; anything else is the JSON 404.
///
/// A served page carries its own policy from `csp`; the 404 paths carry the default, stamped by
/// the security-headers middleware (insert-if-absent) further out.
pub async fn serve(root: &Path, csp: &CspSet, req: Request<Body>) -> Response {
    if req.method() != Method::GET && req.method() != Method::HEAD {
        return crate::routes::api_error(StatusCode::NOT_FOUND, "not found");
    }
    let Some(rel) = resolve(req.uri().path()) else {
        return crate::routes::api_error(StatusCode::NOT_FOUND, "not found");
    };
    let file = root.join(&rel);
    if !file.is_file() {
        return crate::routes::api_error(StatusCode::NOT_FOUND, "not found");
    }
    let policy = csp.header_for(&rel).clone();
    match ServeFile::new(file).oneshot(req).await {
        Ok(resp) => {
            let mut resp = resp.into_response();
            resp.headers_mut().insert(
                header::CACHE_CONTROL,
                HeaderValue::from_static("no-cache, must-revalidate"),
            );
            resp.headers_mut()
                .insert(header::CONTENT_SECURITY_POLICY, policy);
            resp
        }
        Err(_) => crate::routes::api_error(StatusCode::NOT_FOUND, "not found"),
    }
}

#[cfg(test)]
mod tests {
    use super::{inline_scripts, policy, resolve, script_hash, CspSet};
    use std::path::{Path, PathBuf};

    /// A temp dir with the given files (L-008: tests never write into the repo).
    fn fixture(files: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        for (name, body) in files {
            std::fs::write(dir.path().join(name), body).expect("write fixture");
        }
        dir
    }

    #[test]
    fn inline_scripts_matches_only_bare_blocks() {
        let html = concat!(
            "<script src=\"a.js\"></script>\n",
            "<script>\nlet x = 1;\n</script>\n",
            "<script type=\"module\">let y = 2;</script>\n",
            "<p>no script here</p>\n",
            "<script>let z = 3;</script>\n",
        );
        assert_eq!(inline_scripts(html), vec!["\nlet x = 1;\n", "let z = 3;"]);
        // The attributed block is skipped whole, body included: no partial hash.
        assert!(!inline_scripts(html).iter().any(|b| b.contains("let y")));
        assert!(inline_scripts("").is_empty());
        assert!(inline_scripts("<script>unterminated").is_empty());
    }

    #[test]
    fn script_hash_is_the_csp_form() {
        // sha256 of the UTF-8 bytes, standard base64 with padding.
        assert_eq!(
            script_hash("let x = 1;"),
            "sha256-qtc3jf64UXA5J6U02kfQ0Cvn9Uz4wXUaN3NNUVgR2v4="
        );
        assert_eq!(
            script_hash(""),
            "sha256-47DEQpj8HBSa+/TImW+5JCeuQeRkm5NMpJWZG3hSuFU="
        );
    }

    #[test]
    fn scan_hashes_each_page_and_falls_back_to_the_default() {
        let dir = fixture(&[
            ("page.html", "<script>\nlet x = 1;\n</script>"),
            ("plain.html", "<p>no inline block</p>"),
            ("not-a-page.js", "<script>let x = 1;</script>"),
        ]);
        let csp = CspSet::scan(dir.path(), false);
        let page = csp.header_for(Path::new("page.html")).to_str().unwrap();
        assert!(
            page.contains("script-src 'self' 'unsafe-eval' 'sha256-PCA06x5AClOd4MjKmg3RZDFBTufKR2vjtf2Rt+9Kt3s=';"),
            "{page}"
        );
        // A page with no inline block, a .js file and an unknown path all get the default.
        assert_eq!(
            csp.header_for(Path::new("plain.html")),
            csp.default(),
            "no inline block means the default"
        );
        assert_eq!(csp.header_for(Path::new("app.js")), csp.default());
        assert_eq!(csp.header_for(Path::new("missing.html")), csp.default());
        assert!(!csp.default().to_str().unwrap().contains("sha256-"));
    }

    #[test]
    fn a_missing_root_never_errors() {
        let csp = CspSet::scan(Path::new("no/such/static/root"), false);
        assert!(csp
            .header_for(Path::new("hub.html"))
            .to_str()
            .unwrap()
            .starts_with("default-src 'none'; script-src 'self' 'unsafe-eval';"));
    }

    #[test]
    fn the_policy_names_exactly_what_the_pages_use() {
        let plain = policy(&["sha256-abc=".to_string()], false, false);
        for want in [
            "default-src 'none'",
            "script-src 'self' 'unsafe-eval' 'sha256-abc='",
            "style-src 'self' 'unsafe-inline' https://fonts.googleapis.com",
            "font-src 'self' data: https://fonts.gstatic.com",
            "img-src 'self' data:",
            "connect-src 'self'",
            "frame-src 'none'",
            "worker-src 'self' blob:",
            "object-src 'none'",
            "base-uri 'none'",
            "form-action 'none'",
            "frame-ancestors 'self'",
        ] {
            assert!(plain.contains(want), "{want} missing from {plain}");
        }
        assert!(!plain.contains("upgrade-insecure-requests"), "http origin");
        // The hub frames its own engine pages (`hub.html:274`): 'none' here would block the chart.
        assert!(
            !plain.contains("frame-ancestors 'none'"),
            "frame-ancestors 'none' breaks the hub's own <iframe>"
        );

        // hub.html: GIS in script-src and connect-src, and it frames the engine pages.
        let hub = policy(&[], true, true);
        assert!(hub.contains("'unsafe-eval' https://accounts.google.com;"));
        assert!(hub.contains("connect-src 'self' https://accounts.google.com"));
        assert!(hub.contains("frame-src 'self' https://accounts.google.com"));
        assert!(hub.ends_with("; upgrade-insecure-requests"), "{hub}");

        // The strict default carries no hash and no exception for the GIS or frame sources.
        let default = policy(&[], false, false);
        assert!(!default.contains("accounts.google.com"));
        assert!(!default.contains("sha256-"));
    }

    #[test]
    fn hub_html_is_the_only_page_with_gis() {
        let dir = fixture(&[
            ("hub.html", "<script>let a = 1;</script>"),
            ("vela.html", "<script>let a = 1;</script>"),
        ]);
        let csp = CspSet::scan(dir.path(), true);
        let hub = csp.header_for(Path::new("hub.html")).to_str().unwrap();
        let vela = csp.header_for(Path::new("vela.html")).to_str().unwrap();
        assert!(hub.contains("accounts.google.com"));
        assert!(!vela.contains("accounts.google.com"));
        assert!(vela.contains("; upgrade-insecure-requests"));
    }

    #[test]
    fn table() {
        let allowed = [
            ("/", "index.html"),
            ("", "index.html"),
            ("/hub.html", "hub.html"),
            ("/hub.html?x=1", "hub.html"),
            ("/vendor/klinecharts.min.js", "vendor/klinecharts.min.js"),
            ("/VELA.BUNDLE.JS", "VELA.BUNDLE.JS"),
            ("/SOURCE.txt", "SOURCE.txt"),
            ("/NOTICE-vela.txt", "NOTICE-vela.txt"),
        ];
        for (raw, want) in allowed {
            assert_eq!(resolve(raw), Some(PathBuf::from(want)), "{raw}");
        }
        let refused = [
            "/.env",
            "/%2e%2e/LICENSE",
            "/%2e%2e%2fLICENSE",
            "/a%5cb.js",
            "/a\\b.js",
            "/vendor/a/b.js",
            "/server.py",
            "/x.py",
            "/logs/hub.log",
            "/.git/config",
            "/vendor/.hidden.js",
            "/tests/pine_convert_harness.js",
            "/../SOURCE.txt",
            "/a/b/SOURCE.txt",
        ];
        for raw in refused {
            assert_eq!(resolve(raw), None, "{raw}");
        }
    }
}
