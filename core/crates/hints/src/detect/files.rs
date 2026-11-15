//! The filesystem as the detector sees it: whether a path exists (and is a
//! directory), behind a trait so tests use a fake and each platform its
//! own. Paths resolve without touching the filesystem; each pass asks
//! about every distinct path once.

use std::collections::HashMap;

/// What the detector asks of the filesystem.
pub trait Filesystem {
    /// `Some(is_directory)` when something exists at the absolute `path`.
    fn kind(&self, path: &str) -> Option<bool>;
    /// The user's home directory, for `~/…`.
    fn home(&self) -> Option<String>;
}

/// The real filesystem through `std::fs` (macOS and Linux alike).
#[derive(Debug, Default, Clone, Copy)]
pub struct StdFilesystem;

impl Filesystem for StdFilesystem {
    fn kind(&self, path: &str) -> Option<bool> {
        std::fs::metadata(path).ok().map(|m| m.is_dir())
    }

    fn home(&self) -> Option<String> {
        std::env::var("HOME").ok().filter(|h| !h.is_empty())
    }
}

/// One detection pass's view: the filesystem, the working directory, and
/// a cache so a screen that repeats a path stats it once.
pub struct Files<'a> {
    fs: &'a dyn Filesystem,
    cwd: Option<&'a str>,
    local: bool,
    cache: HashMap<String, Option<String>>,
}

impl<'a> Files<'a> {
    pub fn new(fs: &'a dyn Filesystem, cwd: Option<&'a str>, local: bool) -> Files<'a> {
        Files {
            fs,
            cwd,
            local,
            cache: HashMap::new(),
        }
    }

    pub fn local(&self) -> bool {
        self.local
    }

    /// A `file://` URL for `path` (relative to the working directory) if
    /// it exists on this machine; directories end in a slash. Remote
    /// panes never get one.
    pub fn open_target(&mut self, path: &str) -> Option<String> {
        let resolved = self.resolve(path)?;
        // macOS automounts /home and /net: one lookup there can take
        // milliseconds, and Linux paths seen over ssh land there.
        if resolved.starts_with("/home/")
            || resolved.starts_with("/net/")
            || resolved == "/home"
            || resolved == "/net"
        {
            return None;
        }
        if let Some(known) = self.cache.get(&resolved) {
            return known.clone();
        }
        let url = self
            .fs
            .kind(&resolved)
            .map(|directory| file_url(&resolved, directory));
        self.cache.insert(resolved, url.clone());
        url
    }

    /// The absolute, standardised path, worked out without touching the
    /// filesystem.
    fn resolve(&self, path: &str) -> Option<String> {
        if !self.local {
            return None;
        }
        let expanded = if path == "~" || path.starts_with("~/") {
            match self.fs.home() {
                Some(home) => format!("{}{}", home.trim_end_matches('/'), &path[1..]),
                None => path.to_owned(),
            }
        } else {
            path.to_owned()
        };
        let absolute = if expanded.starts_with('/') {
            expanded
        } else {
            let cwd = self.cwd?;
            format!("{}/{}", cwd.trim_end_matches('/'), expanded)
        };
        Some(standardize(&absolute))
    }
}

/// `.` and `..` resolved, repeated and trailing slashes dropped.
pub fn standardize(path: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            _ => parts.push(part),
        }
    }
    format!("/{}", parts.join("/"))
}

/// A `file://` URL for an absolute path, percent-encoded as a URL path.
pub fn file_url(path: &str, directory: bool) -> String {
    let mut url = String::with_capacity(path.len() + 8);
    url.push_str("file://");
    for &b in path.as_bytes() {
        let keep = b.is_ascii_alphanumeric() || b"-._~!$&'()*+,;=:@/".contains(&b);
        if keep {
            url.push(b as char);
        } else {
            url.push_str(&format!("%{b:02X}"));
        }
    }
    if directory && !path.ends_with('/') {
        url.push('/');
    }
    url
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_and_paths() {
        assert_eq!(
            file_url("/tmp/two words.txt", false),
            "file:///tmp/two%20words.txt"
        );
        assert_eq!(
            file_url("/tmp/a%b#c?d[e]界", true),
            "file:///tmp/a%25b%23c%3Fd%5Be%5D%E7%95%8C/"
        );
        assert_eq!(standardize("/a/b/../../../c//d/."), "/c/d");
        assert_eq!(standardize("/a/b/"), "/a/b");
    }
}
