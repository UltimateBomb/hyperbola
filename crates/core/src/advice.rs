//! Turns an engine failure into something the user can act on.
//!
//! "unable to download video data: HTTP Error 403: Forbidden" is true and
//! useless. The message stays — it is what to search for and report — but a
//! sentence of advice goes next to it, because most failures have exactly one
//! sensible next step.

/// A short instruction for a failure, or `None` when there is nothing useful
/// to add and the message should stand alone.
pub fn advice_for(message: &str) -> Option<&'static str> {
    let text = message.to_ascii_lowercase();

    if text.contains("sign in to confirm") || text.contains("confirm you're not a bot") {
        return Some("The site wants a signed-in account. In Settings, choose the browser you are signed into and try again.");
    }
    if text.contains("403") || text.contains("unable to download video data") {
        return Some("The site refused the download. Updating yt-dlp in Updates usually fixes this — sites change what they accept.");
    }
    if text.contains("requested format is not available") {
        return Some("That quality is not offered for this video. Pick another quality, or leave it on best available.");
    }
    if text.contains("video unavailable") || text.contains("removed by the uploader") {
        return Some("This video is gone from the site. Nothing to download.");
    }
    if text.contains("private video") || text.contains("members-only") {
        return Some("This video is private or for members. It needs an account that can see it — set your browser in Settings.");
    }
    if text.contains("is not available in your country") {
        return Some(
            "The site blocks this video in your country. A proxy set in Settings would be needed.",
        );
    }
    if text.contains("no space left") || text.contains("enospc") {
        return Some("The device is out of space.");
    }
    if text.contains("timed out")
        || text.contains("connection")
        || text.contains("network")
        || text.contains("temporary failure")
    {
        return Some(
            "The connection dropped. Try again — the download continues from where it stopped.",
        );
    }
    if text.contains("ffmpeg") {
        return Some("The step that joins video and sound failed. Check ffmpeg in Updates.");
    }
    None
}

#[cfg(test)]
mod tests {
    use super::advice_for;

    #[test]
    fn a_refused_download_points_at_the_update() {
        let advice =
            advice_for("unable to download video data: HTTP Error 403: Forbidden").unwrap();
        assert!(advice.contains("Updating yt-dlp"));
    }

    #[test]
    fn a_login_wall_points_at_cookies() {
        let advice = advice_for("ERROR: Sign in to confirm you're not a bot").unwrap();
        assert!(advice.contains("signed-in account"));
    }

    #[test]
    fn a_dead_video_says_so_plainly() {
        assert!(advice_for("[youtube] abc: Video unavailable")
            .unwrap()
            .contains("gone"));
    }

    #[test]
    fn a_dropped_connection_says_it_will_continue() {
        let advice = advice_for("The read operation timed out").unwrap();
        assert!(advice.contains("continues from where it stopped"));
    }

    #[test]
    fn nothing_is_invented_for_an_unknown_failure() {
        assert_eq!(advice_for("something nobody has seen before"), None);
    }
}
