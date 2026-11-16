// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors

//! The two requests an update makes, both through the `curl` that macOS and
//! Windows ship: asking GitHub for the latest release, and downloading its
//! installer. HTTPS only, also across redirects. See docs/DESKTOP.md.

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

/// Who is asking; GitHub's API refuses requests that don't say.
const USER_AGENT: &str = concat!("Photonation/", env!("CARGO_PKG_VERSION"));

/// GitHub's answer about a release is a few kilobytes; more is not that.
const LARGEST_ANSWER_BYTES: &str = "2000000";
const ANSWER_SECONDS: &str = "30";
/// Long enough for an installer on a slow connection, short enough that a
/// stalled download ends.
const DOWNLOAD_SECONDS: &str = "3600";

/// Separates the answer from the status code `curl` writes after it.
const STATUS_MARKER: &str = "\nphotonation-http-status:";

/// What a server answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Answer {
    pub status: u16,
    pub body: String,
}

/// The system's own `curl`, by its full path where there is one, so that a
/// program of the same name elsewhere on `PATH` isn't run instead.
fn curl_program() -> PathBuf {
    if cfg!(target_os = "macos") {
        PathBuf::from("/usr/bin/curl")
    } else if cfg!(windows) {
        match std::env::var_os("SystemRoot") {
            Some(system_root) => PathBuf::from(system_root).join("System32").join("curl.exe"),
            None => PathBuf::from("curl.exe"),
        }
    } else {
        PathBuf::from("curl")
    }
}

/// `curl` as every request here needs it: no configuration file, nothing on
/// the terminal but errors, HTTPS only (also after a redirect).
fn curl() -> Command {
    let mut command = Command::new(curl_program());
    command
        // Has to come first: ignores ~/.curlrc, which could switch
        // certificate checks off or send the request through a proxy.
        .arg("--disable")
        .args(["--silent", "--show-error", "--location"])
        .args(["--proto", "=https", "--proto-redir", "=https"])
        .args(["--user-agent", USER_AGENT])
        .stdin(Stdio::null());
    hide_console_window(&mut command);
    command
}

/// A console program started by a Windows app without a console opens a
/// window of its own unless told not to.
#[cfg(windows)]
pub fn hide_console_window(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    command.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
pub fn hide_console_window(_command: &mut Command) {}

/// Why `curl` didn't finish, in its own words.
fn failure(output: &Output) -> String {
    let said = String::from_utf8_lossy(&output.stderr);
    let said = said.trim().trim_start_matches("curl: ");
    if said.is_empty() {
        format!("curl stopped with {}", output.status)
    } else {
        said.chars().take(300).collect()
    }
}

fn run(command: &mut Command) -> Result<Output, String> {
    let output = command
        .output()
        .map_err(|error| format!("Couldn't start curl: {error}"))?;
    if output.status.success() {
        Ok(output)
    } else {
        Err(failure(&output))
    }
}

/// Asks `url` for JSON. An answer with an error status is still an answer.
pub fn get(url: &str) -> Result<Answer, String> {
    let output = run(curl()
        .args(["--max-time", ANSWER_SECONDS])
        .args(["--max-filesize", LARGEST_ANSWER_BYTES])
        .args(["--header", "Accept: application/vnd.github+json"])
        .args(["--write-out", &format!("{STATUS_MARKER}%{{http_code}}")])
        .arg(url))?;
    answer_from(&String::from_utf8_lossy(&output.stdout))
        .ok_or_else(|| "curl didn't report the answer's status.".to_owned())
}

/// Splits what `curl` wrote into the answer and its status.
fn answer_from(written: &str) -> Option<Answer> {
    let (body, status) = written.rsplit_once(STATUS_MARKER)?;
    Some(Answer {
        status: status.trim().parse().ok()?,
        body: body.to_owned(),
    })
}

/// Downloads `url` to `destination`, giving up on a file larger than
/// `largest_bytes` where the server says its size first. The caller checks
/// what arrived.
pub fn download(url: &str, destination: &Path, largest_bytes: u64) -> Result<(), String> {
    run(curl()
        .arg("--fail")
        .args(["--max-time", DOWNLOAD_SECONDS])
        .args(["--max-filesize", &largest_bytes.to_string()])
        .arg("--output")
        .arg(destination)
        .arg(url))
    .map(drop)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_status_is_read_from_after_the_answer() {
        let written = format!("{{\"tag_name\": \"v1.2.3\"}}{STATUS_MARKER}200");
        assert_eq!(
            answer_from(&written),
            Some(Answer {
                status: 200,
                body: "{\"tag_name\": \"v1.2.3\"}".into()
            })
        );
        let not_found = format!("{{\"message\": \"Not Found\"}}\n{STATUS_MARKER}404\n");
        assert_eq!(
            answer_from(&not_found).map(|answer| answer.status),
            Some(404)
        );
        // The last marker is curl's, whatever the answer itself contains.
        let echoing = format!("{STATUS_MARKER}200{STATUS_MARKER}500");
        assert_eq!(answer_from(&echoing).map(|answer| answer.status), Some(500));
        assert_eq!(answer_from("no status"), None);
        assert_eq!(answer_from(&format!("body{STATUS_MARKER}soon")), None);
    }

    #[test]
    fn requests_are_https_only_and_ignore_the_users_curl_configuration() {
        let command = curl();
        let arguments: Vec<_> = command
            .get_args()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect();
        assert_eq!(arguments[0], "--disable");
        for pair in [["--proto", "=https"], ["--proto-redir", "=https"]] {
            assert!(
                arguments.windows(2).any(|window| window == pair),
                "{pair:?}"
            );
        }
        assert!(!arguments.iter().any(|argument| argument == "--insecure"));
    }
}
