//! Getting a cookie export into the shape the engine reads.
//!
//! Every browser extension that exports cookies produces one of two things:
//! the Netscape table yt-dlp wants, or a JSON array. The JSON kind is at
//! least as common — it is what EditThisCookie hands you — and yt-dlp
//! rejects it outright. A user who exports cookies, points the app at the
//! file and gets the same login wall has no way to tell that the file was
//! the problem.
//!
//! This is a format rule, so it lives with the other rules and does no I/O.

use serde::Deserialize;

/// One cookie as the JSON exporters write it.
#[derive(Debug, Deserialize)]
struct JsonCookie {
    domain: Option<String>,
    name: Option<String>,
    value: Option<String>,
    path: Option<String>,
    secure: Option<bool>,
    #[serde(rename = "expirationDate")]
    expiration_date: Option<f64>,
    /// Some exporters use this name instead.
    expires: Option<f64>,
    #[serde(rename = "hostOnly")]
    host_only: Option<bool>,
}

const HEADER: &str = "# Netscape HTTP Cookie File";

/// Turns whatever the user exported into a Netscape cookie file.
///
/// Returns `None` when the text is neither a JSON export nor anything that
/// looks like a cookie table — better to refuse than to hand the engine a
/// file that will fail for a reason nobody can see.
pub fn to_netscape(text: &str) -> Option<String> {
    let trimmed = text.trim_start_matches('\u{feff}').trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.starts_with('[') || trimmed.starts_with('{') {
        return from_json(trimmed);
    }
    if looks_like_a_table(trimmed) {
        let mut out = String::new();
        if !trimmed.starts_with('#') {
            out.push_str(HEADER);
            out.push('\n');
        }
        out.push_str(trimmed);
        out.push('\n');
        return Some(out);
    }
    None
}

fn from_json(text: &str) -> Option<String> {
    // A single cookie, an array of them, or an object wrapping the array:
    // all three appear in the wild.
    let value: serde_json::Value = serde_json::from_str(text).ok()?;
    let array = match &value {
        serde_json::Value::Array(items) => items.clone(),
        serde_json::Value::Object(map) => map
            .values()
            .find_map(|v| v.as_array().cloned())
            .unwrap_or_else(|| vec![value.clone()]),
        _ => return None,
    };

    let mut out = String::from(HEADER);
    out.push('\n');
    let mut written = 0usize;
    for item in array {
        let Ok(cookie) = serde_json::from_value::<JsonCookie>(item) else {
            continue;
        };
        let (Some(domain), Some(name)) = (cookie.domain, cookie.name) else {
            continue;
        };
        if domain.is_empty() {
            continue;
        }
        // A leading dot is how the file says "and every subdomain". Exporters
        // that drop it also set hostOnly, so trust that when it is there.
        let subdomains = if cookie.host_only == Some(false) && !domain.starts_with('.') {
            true
        } else {
            domain.starts_with('.')
        };
        let domain = if subdomains && !domain.starts_with('.') {
            format!(".{domain}")
        } else {
            domain
        };
        let expiry = cookie
            .expiration_date
            .or(cookie.expires)
            .filter(|e| e.is_finite() && *e > 0.0)
            .map(|e| e as i64)
            .unwrap_or(0);
        out.push_str(&format!(
            "{domain}\t{}\t{}\t{}\t{expiry}\t{name}\t{}\n",
            if subdomains { "TRUE" } else { "FALSE" },
            cookie.path.unwrap_or_else(|| "/".into()),
            if cookie.secure.unwrap_or(false) {
                "TRUE"
            } else {
                "FALSE"
            },
            cookie.value.unwrap_or_default(),
        ));
        written += 1;
    }
    (written > 0).then_some(out)
}

/// A Netscape line is seven tab-separated fields, the second of which says
/// TRUE or FALSE. Comments and blank lines are allowed around them.
fn looks_like_a_table(text: &str) -> bool {
    text.lines()
        .filter(|line| !line.trim().is_empty() && !line.starts_with('#'))
        .any(|line| {
            let fields: Vec<&str> = line.split('\t').collect();
            fields.len() >= 7 && matches!(fields[1], "TRUE" | "FALSE")
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shape EditThisCookie exports, which is what the operator actually
    /// had in hand — and which yt-dlp refuses.
    #[test]
    fn a_json_export_becomes_a_table() {
        let json = r#"[
          {"domain":".youtube.com","expirationDate":1789000000.51,"hostOnly":false,
           "httpOnly":true,"name":"SID","path":"/","secure":true,"session":false,
           "value":"abc"},
          {"domain":"www.youtube.com","hostOnly":true,"name":"TEMP","path":"/watch",
           "secure":false,"session":true,"value":"xyz"}
        ]"#;
        let out = to_netscape(json).expect("a JSON export must convert");
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines[0], HEADER);
        assert_eq!(
            lines[1],
            ".youtube.com\tTRUE\t/\tTRUE\t1789000000\tSID\tabc"
        );
        // A session cookie has no expiry, and a host-only one does not claim
        // subdomains.
        assert_eq!(
            lines[2],
            "www.youtube.com\tFALSE\t/watch\tFALSE\t0\tTEMP\txyz"
        );
    }

    #[test]
    fn a_table_is_kept_and_gains_a_header_if_it_has_none() {
        let table = ".youtube.com\tTRUE\t/\tTRUE\t1789000000\tSID\tabc";
        let out = to_netscape(table).unwrap();
        assert!(out.starts_with(HEADER));
        assert!(out.contains(table));

        let with_header = format!("{HEADER}\n{table}\n");
        let kept = to_netscape(&with_header).unwrap();
        assert_eq!(kept.matches(HEADER).count(), 1);
    }

    #[test]
    fn something_that_is_not_cookies_is_refused() {
        assert!(to_netscape("").is_none());
        assert!(to_netscape("hello there").is_none());
        assert!(to_netscape("[]").is_none());
        assert!(to_netscape("[{\"no\":\"domain\"}]").is_none());
    }

    /// A byte-order mark at the front of a file saved from Notepad must not
    /// make the whole export unrecognisable.
    #[test]
    fn a_byte_order_mark_does_not_hide_the_export() {
        let json = "\u{feff}[{\"domain\":\".youtube.com\",\"name\":\"SID\",\"value\":\"a\"}]";
        assert!(to_netscape(json).is_some());
    }
}
