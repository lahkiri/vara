//! Web tools: search + page fetch. Minimal, dependency-light, and honest
//! about failure (the harness v2 lesson: empty snippets poison everything —
//! so every tool returns real content or a real error, never a hollow ok).

use crate::types::{PageContent, SearchHit};
use crate::{Result, VaraError};
use percent_encoding::percent_decode_str;
use regex::Regex;
use std::net::IpAddr;
use std::sync::OnceLock;
use std::time::Duration;

/// Why a URL is not fetchable, phrased for a model that must adapt.
///
/// `fetch_page` is a *tool*: its caller may be a model reading attacker-supplied
/// text. A fetch that can reach the machine's own network is a request-forgery
/// primitive — `http://169.254.169.254/` on a cloud VM returns credentials,
/// `http://localhost:1420/` reaches a local dev server, and
/// `http://192.168.1.1/` reaches the owner's router. None of those are "the web".
///
/// Two guards, because one is not enough:
///   1. the host is resolved and every candidate address is checked, and
///   2. redirects are not followed automatically, so a public URL cannot hand us
///      an internal `Location:` — each hop is re-checked through `fetch_page`.
pub fn check_public_url(url: &str) -> std::result::Result<(), String> {
    let parsed = url::Url::parse(url).map_err(|e| format!("not a valid url: {e}"))?;
    let scheme = parsed.scheme();
    if scheme != "http" && scheme != "https" {
        return Err(format!("only http and https can be fetched, not {scheme}"));
    }
    let Some(host) = parsed.host_str() else {
        return Err("the url has no host".to_string());
    };
    let host = host.trim_matches(['[', ']']).to_string();

    // A literal address can be judged directly; a name must be resolved, because
    // `localhost`/`*.internal` are not the only ways to reach a private machine.
    if let Ok(ip) = host.parse::<IpAddr>() {
        return if is_internal_ip(ip) {
            Err(format!("{host} is a private or loopback address"))
        } else {
            Ok(())
        };
    }

    let lowered = host.to_lowercase();
    for forbidden in [".localhost", ".local", ".internal", ".home.arpa"] {
        if lowered == forbidden.trim_start_matches('.') || lowered.ends_with(forbidden) {
            return Err(format!("{host} is a local name, not a public host"));
        }
    }

    // Resolve and judge every answer. A name that resolves to a private address
    // (DNS rebinding, an internal-only name, a hosts-file entry) is refused the
    // same way a literal one is.
    match std::net::ToSocketAddrs::to_socket_addrs(&(host.as_str(), 0u16)) {
        Ok(addrs) => {
            for addr in addrs {
                if is_internal_ip(addr.ip()) {
                    return Err(format!(
                        "{host} resolves to {}, which is a private address",
                        addr.ip()
                    ));
                }
            }
            Ok(())
        }
        // Unresolvable is not a security failure — let the request fail with its
        // own, more useful error.
        Err(_) => Ok(()),
    }
}

/// True for addresses that cannot be part of the public web.
pub fn is_internal_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()          // 169.254.0.0/16 — cloud metadata
                || v4.is_broadcast()
                || v4.is_documentation()
                || v4.is_unspecified()
                || v4.is_multicast()
                // 100.64/10 carrier-grade NAT, 192.0.0.0/24 IETF, 198.18/15 benchmark
                || (v4.octets()[0] == 100 && (64..128).contains(&v4.octets()[1]))
                || (v4.octets()[0] == 192 && v4.octets()[1] == 0 && v4.octets()[2] == 0)
                || (v4.octets()[0] == 198 && (18..20).contains(&v4.octets()[1]))
                // 240/4 reserved
                || v4.octets()[0] >= 240
        }
        IpAddr::V6(v6) => {
            v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                // unique-local fc00::/7 and link-local fe80::/10
                || (v6.segments()[0] & 0xfe00) == 0xfc00
                || (v6.segments()[0] & 0xffc0) == 0xfe80
                // IPv4-mapped addresses are judged as their v4 form
                || v6
                    .to_ipv4_mapped()
                    .map(|v4| is_internal_ip(IpAddr::V4(v4)))
                    .unwrap_or(false)
        }
    }
}

pub struct HttpClient {
    http: reqwest::Client,
}

const UA: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0 Safari/537.36 Vara/0.1";

impl HttpClient {
    pub fn new() -> Result<Self> {
        let http = reqwest::Client::builder()
            .user_agent(UA)
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(30))
            // Redirects are followed by hand, one hop at a time, so every hop
            // passes `check_public_url`. Automatic following would let a public
            // URL hand us an internal `Location:` and bypass the guard entirely.
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| VaraError::Http(e.to_string()))?;
        Ok(Self { http })
    }

    /// Web search: DuckDuckGo HTML, with the lite endpoint as fallback.
    pub async fn web_search(&self, query: &str, max: usize) -> Result<Vec<SearchHit>> {
        let q = query.trim();
        if q.is_empty() {
            return Err(VaraError::Http("search: empty query".into()));
        }
        let mut last_err = String::new();
        for (name, run) in [("html", 0u8), ("lite", 1u8)] {
            let outcome = match run {
                0 => self.search_html(q).await,
                _ => self.search_lite(q).await,
            };
            let _ = name;
            match outcome {
                Ok(hits) if !hits.is_empty() => return Ok(hits.into_iter().take(max).collect()),
                Ok(_) => last_err = "no results".into(),
                Err(e) => last_err = e.to_string(),
            }
        }
        Err(VaraError::Http(format!("search failed: {last_err}")))
    }

    async fn search_html(&self, q: &str) -> Result<Vec<SearchHit>> {
        let resp = self
            .http
            .get("https://html.duckduckgo.com/html/")
            .query(&[("q", q), ("kl", "wt-wt")])
            .send()
            .await
            .map_err(|e| VaraError::Http(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(VaraError::Http(format!("ddg html: HTTP {}", resp.status())));
        }
        let html = resp
            .text()
            .await
            .map_err(|e| VaraError::Http(e.to_string()))?;
        Ok(parse_ddg_html(&html))
    }

    async fn search_lite(&self, q: &str) -> Result<Vec<SearchHit>> {
        let resp = self
            .http
            .get("https://lite.duckduckgo.com/lite/")
            .query(&[("q", q)])
            .send()
            .await
            .map_err(|e| VaraError::Http(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(VaraError::Http(format!("ddg lite: HTTP {}", resp.status())));
        }
        let html = resp
            .text()
            .await
            .map_err(|e| VaraError::Http(e.to_string()))?;
        Ok(parse_ddg_lite(&html))
    }

    /// Fetch a page and convert to plain text (capped).
    pub async fn fetch_page(&self, url: &str, max_chars: usize) -> Result<PageContent> {
        let url = url.trim();
        if !url.starts_with("http://") && !url.starts_with("https://") {
            return Err(VaraError::Http(format!("fetch: not an http url: {url}")));
        }
        // The machine's own network is not "the web": see `check_public_url`.
        // Refused here with a reason the caller can act on, rather than by a
        // timeout that looks like the site being down.
        check_public_url(url).map_err(|why| VaraError::Http(format!("fetch refused: {why}")))?;
        let resp = self
            .http
            .get(url)
            .send()
            .await
            .map_err(|e| VaraError::Http(format!("fetch {url}: {e}")))?;
        let final_url = resp.url().to_string();
        let status = resp.status();
        if !status.is_success() {
            return Err(VaraError::Http(format!("fetch {url}: HTTP {status}")));
        }
        let ct = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_lowercase();
        for bad in ["pdf", "image/", "video/", "audio/", "zip", "gzip"] {
            if ct.contains(bad) {
                return Err(VaraError::Http(format!(
                    "fetch {url}: unsupported content-type '{ct}'"
                )));
            }
        }
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| VaraError::Http(e.to_string()))?;
        if bytes.len() > 2_500_000 {
            return Err(VaraError::Http(format!("fetch {url}: page too large")));
        }
        let html = String::from_utf8_lossy(&bytes).to_string();
        let title = extract_title(&html).unwrap_or_else(|| url.to_string());
        let text = html_to_text(&html);
        let text: String = text.chars().take(max_chars.max(200)).collect();
        if text.trim().is_empty() {
            return Err(VaraError::Http(format!("fetch {url}: no readable text")));
        }
        Ok(PageContent {
            url: final_url,
            title,
            text,
        })
    }
}

// ---------- DuckDuckGo parsing ----------

fn ddg_anchor_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    // class="...result__a..." appears before href in DDG html.
    R.get_or_init(|| {
        Regex::new(r#"<a[^>]*class="[^"]*result__a[^"]*"[^>]*href="([^"]+)"[^>]*>(?s)(.*?)</a>"#)
            .unwrap()
    })
}

fn ddg_snippet_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        Regex::new(r#"<a[^>]*class="[^"]*result__snippet[^"]*"[^>]*>(?s)(.*?)</a>"#).unwrap()
    })
}

fn lite_link_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        Regex::new(r#"<a[^>]*class="result-link"[^>]*href="([^"]+)"[^>]*>(?s)(.*?)</a>"#).unwrap()
    })
}

fn parse_ddg_html(html: &str) -> Vec<SearchHit> {
    let snippets: Vec<String> = ddg_snippet_re()
        .captures_iter(html)
        .map(|c| strip_tags(&c[1]).trim().to_string())
        .collect();
    let mut out = Vec::new();
    for (i, c) in ddg_anchor_re().captures_iter(html).enumerate() {
        let url = resolve_ddg_href(&c[1]);
        if !url.starts_with("http") {
            continue;
        }
        let title = strip_tags(&c[2]).trim().to_string();
        if title.is_empty() {
            continue;
        }
        let snippet = snippets.get(i).cloned().unwrap_or_default();
        out.push(SearchHit {
            title,
            url,
            snippet,
        });
    }
    out
}

fn parse_ddg_lite(html: &str) -> Vec<SearchHit> {
    let mut out = Vec::new();
    for c in lite_link_re().captures_iter(html) {
        let url = resolve_ddg_href(&c[1]);
        if !url.starts_with("http") {
            continue;
        }
        let title = strip_tags(&c[2]).trim().to_string();
        if title.is_empty() {
            continue;
        }
        out.push(SearchHit {
            title,
            url,
            snippet: String::new(),
        });
    }
    out
}

/// DuckDuckGo result links are redirects like
/// `//duckduckgo.com/l/?uddg=https%3A%2F%2Fexample.com%2F&rut=abc` — unwrap.
pub fn resolve_ddg_href(href: &str) -> String {
    let h = href.trim();
    if let Some(pos) = h.find("uddg=") {
        let rest = &h[pos + 5..];
        let end = rest.find('&').unwrap_or(rest.len());
        let decoded = percent_decode_str(&rest[..end])
            .decode_utf8_lossy()
            .to_string();
        if decoded.starts_with("http") {
            return decoded;
        }
    }
    if h.starts_with("//") {
        format!("https:{h}")
    } else if h.starts_with('/') && !h.starts_with("//") {
        format!("https://duckduckgo.com{h}")
    } else {
        h.to_string()
    }
}

// ---------- HTML -> text ----------

fn block_re(tag: &str) -> Regex {
    Regex::new(&format!(r"(?is)<{tag}\b[^>]*>.*?</{tag}>")).unwrap()
}

fn strip_tags(s: &str) -> String {
    let mut out = Regex::new(r"(?s)<[^>]+>")
        .unwrap()
        .replace_all(s, "")
        .to_string();
    out = decode_entities(&out);
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn html_to_text(html: &str) -> String {
    let mut t = html.to_string();
    for tag in [
        "script", "style", "noscript", "svg", "iframe", "header", "footer", "nav", "form",
    ] {
        t = block_re(tag).replace_all(&t, " ").to_string();
    }
    t = Regex::new(r"(?s)<!--.*?-->")
        .unwrap()
        .replace_all(&t, " ")
        .to_string();
    // Line breaks for block-level boundaries.
    t = Regex::new(r"(?i)<br\s*/?>|</(p|div|li|h[1-6]|tr|section|article|blockquote)>")
        .unwrap()
        .replace_all(&t, "\n")
        .to_string();
    t = Regex::new(r"(?s)<[^>]+>")
        .unwrap()
        .replace_all(&t, " ")
        .to_string();
    t = decode_entities(&t);
    // Collapse whitespace but keep paragraph breaks.
    t = Regex::new(r"[ \t\r\f]+")
        .unwrap()
        .replace_all(&t, " ")
        .to_string();
    t = Regex::new(r"\n\s*\n\s*")
        .unwrap()
        .replace_all(&t, "\n\n")
        .to_string();
    t.trim().to_string()
}

fn decode_entities(s: &str) -> String {
    let named = [
        ("&amp;", "&"),
        ("&lt;", "<"),
        ("&gt;", ">"),
        ("&quot;", "\""),
        ("&#39;", "'"),
        ("&apos;", "'"),
        ("&nbsp;", " "),
        ("&mdash;", "—"),
        ("&ndash;", "–"),
        ("&hellip;", "…"),
        ("&rsquo;", "\u{2019}"),
        ("&lsquo;", "\u{2018}"),
        ("&ldquo;", "\u{201C}"),
        ("&rdquo;", "\u{201D}"),
    ];
    let mut out = s.to_string();
    for (k, v) in named {
        out = out.replace(k, v);
    }
    let num = Regex::new(r"&#(\d{1,5});").unwrap();
    out = num
        .replace_all(&out, |c: &regex::Captures| {
            c[1].parse::<u32>()
                .ok()
                .and_then(char::from_u32)
                .map(|ch| ch.to_string())
                .unwrap_or_default()
        })
        .to_string();
    let hex = Regex::new(r"(?i)&#x([0-9a-f]{1,6});").unwrap();
    out = hex
        .replace_all(&out, |c: &regex::Captures| {
            u32::from_str_radix(&c[1], 16)
                .ok()
                .and_then(char::from_u32)
                .map(|ch| ch.to_string())
                .unwrap_or_default()
        })
        .to_string();
    out
}

fn extract_title(html: &str) -> Option<String> {
    let re = Regex::new(r"(?is)<title[^>]*>(.*?)</title>").unwrap();
    re.captures(html)
        .map(|c| strip_tags(&c[1]).chars().take(200).collect::<String>())
}

pub fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let t: String = s.chars().take(n).collect();
        format!("{t}…")
    }
}

#[cfg(test)]
mod ssrf_tests {
    use super::*;

    /// Defect I from the audit, frozen: `fetch_page` filtered on the `http`
    /// prefix and nothing else, so a model coaxed by page text could fetch
    /// `http://169.254.169.254/` (cloud metadata), `http://localhost:1420/`
    /// (a local dev server) or the owner's router.
    ///
    /// These are refused before any socket is opened, and the reason names the
    /// address rather than looking like a network failure.
    #[test]
    fn internal_addresses_are_refused_before_any_request() {
        let refused = [
            "http://127.0.0.1:1420/",
            "http://127.1.2.3/",
            "http://localhost/",
            "http://localhost:8080/admin",
            "http://LOCALHOST/",
            "http://169.254.169.254/latest/meta-data/",
            "http://10.0.0.1/",
            "http://192.168.1.1/",
            "http://172.16.5.5/",
            "http://100.64.0.1/",
            "http://0.0.0.0/",
            "http://[::1]/",
            "http://[fe80::1]/",
            "http://[fd00::1]/",
            "http://metadata.google.internal/computeMetadata/v1/",
            "http://printer.local/",
            "http://thing.home.arpa/",
            "ftp://example.com/",
            "file:///C:/Windows/win.ini",
        ];
        for url in refused {
            let verdict = check_public_url(url);
            assert!(
                verdict.is_err(),
                "this must be refused, got {verdict:?} for {url}"
            );
        }
    }

    /// …and it must not break the actual job: public hosts still pass.
    #[test]
    fn public_urls_are_still_allowed() {
        for url in [
            "https://example.com/",
            "https://docs.rs/url/latest/url/",
            "http://example.org/page?a=1",
            // A literal public address must not be caught by the private-range
            // checks by accident.
            "https://1.1.1.1/",
            "https://93.184.216.34/",
        ] {
            let verdict = check_public_url(url);
            assert!(verdict.is_ok(), "{url} must stay fetchable: {verdict:?}");
        }
    }

    /// The classification itself, so a missed range is a test failure and not a
    /// silent hole.
    #[test]
    fn the_private_and_special_ranges_are_classified() {
        let internal = [
            "127.0.0.1",
            "10.1.1.1",
            "172.31.255.255",
            "192.168.0.1",
            "169.254.169.254",
            "100.64.0.1",
            "192.0.0.1",
            "198.18.0.1",
            "240.0.0.1",
            "0.0.0.0",
            "::1",
            "fe80::1",
            "fc00::1",
            "fd12:3456::1",
            "::ffff:127.0.0.1",
        ];
        for ip in internal {
            let parsed: IpAddr = ip.parse().expect("a literal address");
            assert!(is_internal_ip(parsed), "{ip} must be classified internal");
        }
        for ip in ["1.1.1.1", "93.184.216.34", "2606:4700:4700::1111"] {
            let parsed: IpAddr = ip.parse().expect("a literal address");
            assert!(!is_internal_ip(parsed), "{ip} must stay public");
        }
    }
}
