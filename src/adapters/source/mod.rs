//! Filesystem boundary for read-only document sources.

use std::fs;
use std::path::{Component, Path, PathBuf};

use crate::domain::{ErrorKind, FastSearchError};

#[allow(
    dead_code,
    reason = "B1 private scanner seam is test-covered and consumed by B2 parser."
)]
const EXCLUDED_DIRECTORIES: &[&str] = &[
    ".agents",
    ".cfknowledge",
    ".git",
    "build",
    "generated",
    "service",
    "target",
    "vendor",
];

#[allow(
    dead_code,
    reason = "B1 private scanner seam is test-covered and consumed by B2 parser."
)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ScannedSourceKind {
    Markdown,
    Tsv,
}

/// A verified filesystem source awaiting B2/B3 parsing.
#[allow(
    dead_code,
    reason = "B1 private scanner seam is test-covered and consumed by B2 parser."
)]
#[derive(Debug)]
struct ScannedSource {
    path: PathBuf,
    locator: String,
    bytes: Vec<u8>,
    kind: ScannedSourceKind,
}

#[allow(
    dead_code,
    reason = "B1 private scanner seam is test-covered and consumed by B2 parser."
)]
fn scan_sources(root: &Path) -> Result<Vec<ScannedSource>, FastSearchError> {
    let root = root
        .canonicalize()
        .map_err(|error| source_failure("canonicalize source root", error))?;
    if !root.is_dir() {
        return Err(FastSearchError::new(
            ErrorKind::SourceFailure,
            "source root must be a directory",
        ));
    }

    let exclusions = RootExclusions::read(&root)?;
    let mut sources = Vec::new();
    scan_directory(&root, &root, &exclusions, &mut sources)?;
    sources.sort_by(|left, right| left.locator.cmp(&right.locator));
    Ok(sources)
}

#[allow(
    dead_code,
    reason = "B1 private scanner seam is test-covered and consumed by B2 parser."
)]
fn scan_directory(
    root: &Path,
    directory: &Path,
    exclusions: &RootExclusions,
    sources: &mut Vec<ScannedSource>,
) -> Result<(), FastSearchError> {
    let entries =
        fs::read_dir(directory).map_err(|error| source_failure("read source directory", error))?;
    for entry in entries {
        let entry = entry.map_err(|error| source_failure("read source directory entry", error))?;
        let file_type = entry
            .file_type()
            .map_err(|error| source_failure("read source entry type", error))?;
        if file_type.is_symlink() {
            continue;
        }

        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_str().ok_or_else(|| {
            FastSearchError::new(ErrorKind::SourceFailure, "source entry name is not UTF-8")
        })?;
        if file_type.is_dir() {
            if EXCLUDED_DIRECTORIES.contains(&name)
                || exclusions
                    .directories
                    .iter()
                    .any(|excluded| excluded == name)
            {
                continue;
            }
            scan_directory(root, &path, exclusions, sources)?;
        } else if file_type.is_file()
            && !exclusions.files.iter().any(|excluded| excluded == name)
            && let Some(kind) = source_kind(&path)
        {
            sources.push(read_source(root, path, kind)?);
        }
    }
    Ok(())
}

#[allow(
    dead_code,
    reason = "B1 private scanner seam is test-covered and consumed by B2 parser."
)]
fn read_source(
    root: &Path,
    path: PathBuf,
    kind: ScannedSourceKind,
) -> Result<ScannedSource, FastSearchError> {
    let canonical_path = path
        .canonicalize()
        .map_err(|error| source_failure("canonicalize source file", error))?;
    let relative = canonical_path.strip_prefix(root).map_err(|_| {
        FastSearchError::new(
            ErrorKind::SourceFailure,
            "source file escapes configured root",
        )
    })?;
    if !relative
        .components()
        .all(|component| matches!(component, Component::Normal(_)))
    {
        return Err(FastSearchError::new(
            ErrorKind::SourceFailure,
            "source locator is not a contained relative path",
        ));
    }
    let bytes =
        fs::read(&canonical_path).map_err(|error| source_failure("read source file", error))?;
    std::str::from_utf8(&bytes)
        .map_err(|_| FastSearchError::new(ErrorKind::SourceFailure, "source file is not UTF-8"))?;

    Ok(ScannedSource {
        locator: relative.to_string_lossy().replace('\\', "/"),
        path: canonical_path,
        bytes,
        kind,
    })
}

#[allow(
    dead_code,
    reason = "B1 private scanner seam is test-covered and consumed by B2 parser."
)]
fn source_kind(path: &Path) -> Option<ScannedSourceKind> {
    match path.extension().and_then(|extension| extension.to_str()) {
        Some("md") => Some(ScannedSourceKind::Markdown),
        Some("tsv") => Some(ScannedSourceKind::Tsv),
        _ => None,
    }
}

#[allow(
    dead_code,
    reason = "B1 private scanner seam is test-covered and consumed by B2 parser."
)]
#[derive(Default)]
struct RootExclusions {
    files: Vec<String>,
    directories: Vec<String>,
}

#[allow(
    dead_code,
    reason = "B1 private scanner seam is test-covered and consumed by B2 parser."
)]
impl RootExclusions {
    fn read(root: &Path) -> Result<Self, FastSearchError> {
        let path = root.join(".gitignore");
        if !path.exists() {
            return Ok(Self::default());
        }
        let content = fs::read_to_string(path)
            .map_err(|error| source_failure("read root .gitignore", error))?;
        let mut exclusions = Self::default();
        for line in content
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
        {
            let (literal, is_directory) = parse_root_gitignore_literal(line)?;
            if is_directory {
                exclusions.directories.push(literal);
            } else {
                exclusions.files.push(literal);
            }
        }
        Ok(exclusions)
    }
}

#[allow(
    dead_code,
    reason = "B1 private scanner seam is test-covered and consumed by B2 parser."
)]
fn parse_root_gitignore_literal(line: &str) -> Result<(String, bool), FastSearchError> {
    let line = line.strip_prefix('/').unwrap_or(line);
    let is_directory = line.ends_with('/');
    let literal = line.trim_end_matches('/');
    let unsupported = literal.is_empty()
        || literal.contains(['/', '\\', '*', '?', '[', ']', '!'])
        || literal == "."
        || literal == "..";
    if unsupported {
        return Err(FastSearchError::new(
            ErrorKind::SourceFailure,
            format!("unsupported root .gitignore rule: {line}"),
        ));
    }
    Ok((literal.to_owned(), is_directory))
}

#[allow(
    dead_code,
    reason = "B1 private scanner seam is test-covered and consumed by B2 parser."
)]
fn source_failure(context: &str, error: std::io::Error) -> FastSearchError {
    FastSearchError::new(ErrorKind::SourceFailure, format!("{context}: {error}"))
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use crate::domain::ErrorKind;

    use super::{ScannedSourceKind, scan_sources};

    static NEXT_FIXTURE: AtomicUsize = AtomicUsize::new(0);

    #[test]
    fn scanner_returns_only_allowed_contained_utf8_sources_in_locator_order() {
        let fixture = Fixture::new();
        fixture.write("docs/zeta.md", "zeta");
        fixture.write("docs/alpha.md", "alpha");
        fixture.write("registry.tsv", "id\ttitle");
        fixture.write("ignored.md", "ignored");
        fixture.write("ignored-dir/hidden.tsv", "hidden");
        fixture.write("target/build.md", "generated");
        fixture.write("notes.txt", "unsupported");
        fixture.write(".gitignore", "ignored.md\nignored-dir/\n");

        let scanned = scan_sources(fixture.path()).expect("allowed fixture must scan");
        let locators = scanned
            .iter()
            .map(|source| source.locator.as_str())
            .collect::<Vec<_>>();

        assert_eq!(
            locators,
            ["docs/alpha.md", "docs/zeta.md", "registry.tsv"],
            "scanner must exclude ignored/build/unsupported files and sort lexically"
        );
        let canonical_root = fixture
            .path()
            .canonicalize()
            .expect("canonical fixture root");
        assert!(
            scanned
                .iter()
                .all(|source| source.path.starts_with(&canonical_root))
        );
        assert!(
            scanned
                .iter()
                .all(|source| std::str::from_utf8(&source.bytes).is_ok())
        );
        assert_eq!(
            scanned.iter().map(|source| source.kind).collect::<Vec<_>>(),
            [
                ScannedSourceKind::Markdown,
                ScannedSourceKind::Markdown,
                ScannedSourceKind::Tsv
            ]
        );
    }

    #[test]
    fn scanner_rejects_non_utf8_allowed_source_without_partial_result() {
        let fixture = Fixture::new();
        fixture.write("allowed.md", "valid");
        fs::write(fixture.path().join("invalid.tsv"), [0xff, 0xfe]).expect("invalid UTF-8 fixture");

        let error =
            scan_sources(fixture.path()).expect_err("invalid source must reject complete scan");

        assert_eq!(error.kind(), &ErrorKind::SourceFailure);
        assert!(error.message().contains("not UTF-8"));
    }

    #[test]
    fn scanner_rejects_unsupported_root_gitignore_rule() {
        let fixture = Fixture::new();
        fixture.write("allowed.md", "valid");
        fixture.write(".gitignore", "*.md\n");

        let error = scan_sources(fixture.path())
            .expect_err("glob rule must not be misrepresented as supported");

        assert_eq!(error.kind(), &ErrorKind::SourceFailure);
        assert!(error.message().contains("unsupported root .gitignore rule"));
    }

    #[test]
    fn scanner_does_not_follow_outside_root_junction_sentinel() {
        let fixture = Fixture::new();
        let outside = fixture.root.with_extension("outside");
        fs::create_dir(&outside).expect("outside sentinel directory");
        fs::write(outside.join("sentinel.md"), "outside sentinel").expect("outside sentinel");
        create_directory_junction(&outside, &fixture.path().join("outside-junction"));

        let scanned = scan_sources(fixture.path()).expect("junction must not compromise scan");

        assert!(
            scanned.is_empty(),
            "outside sentinel must not become source input"
        );
        fs::remove_dir(fixture.path().join("outside-junction")).expect("remove junction");
        fs::remove_dir_all(outside).expect("remove outside sentinel");
    }

    #[cfg(windows)]
    fn create_directory_junction(target: &std::path::Path, link: &std::path::Path) {
        let output = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(link)
            .arg(target)
            .output()
            .expect("start mklink /J for Windows containment fixture");
        assert!(
            output.status.success(),
            "mklink /J failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[cfg(unix)]
    fn create_directory_junction(target: &std::path::Path, link: &std::path::Path) {
        std::os::unix::fs::symlink(target, link).expect("create symlink sentinel");
    }

    struct Fixture {
        root: PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let serial = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
            let root =
                std::env::temp_dir().join(format!("fastsearch-b1-{}-{serial}", std::process::id()));
            fs::create_dir_all(&root).expect("fixture root");
            Self { root }
        }

        fn path(&self) -> &std::path::Path {
            &self.root
        }

        fn write(&self, relative: &str, content: &str) {
            let path = self.root.join(relative);
            fs::create_dir_all(path.parent().expect("fixture parent")).expect("fixture parent");
            fs::write(path, content).expect("fixture file");
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }
}
