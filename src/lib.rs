//! Convert between Wine and native file paths without spawning a `winepath` process.
//!
//! This crate implements the conversion logic in much the same way as Wine itself.
//!
//! > Only for use on systems that have Wine!
use std::{
    fmt::{self, Debug, Display, Formatter},
    path::{Component, Path, PathBuf},
};

/// A native path on the host system.
type NativePath = Path;

/// A file path within Wine. Wrapper around a string.
///
/// ```rust
/// use winepath::WinePath;
/// let wine_path = WinePath(r"C:\windows\system32\ddraw.dll".to_string());
/// ```
#[derive(Debug, Clone)]
pub struct WinePath(pub String);
impl AsRef<str> for WinePath {
    fn as_ref(&self) -> &str {
        &self.0
    }
}
impl From<String> for WinePath {
    fn from(string: String) -> Self {
        Self(string)
    }
}
impl From<&str> for WinePath {
    fn from(string: &str) -> Self {
        Self(string.to_string())
    }
}
impl Display for WinePath {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Error type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WinePathError {
    /// Could not determine the wine prefix to use.
    PrefixNotFound,
    /// No drive letter → file path mapping is available for the given path.
    NoDrive,
}

impl Display for WinePathError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            WinePathError::PrefixNotFound => write!(f, "could not determine wine prefix"),
            WinePathError::NoDrive => write!(f, "native path is not mapped to a wine drive"),
        }
    }
}

impl std::error::Error for WinePathError {}

fn default_wineprefix() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from).map(|mut home| {
        home.push(".wine");
        home
    })
}

const ASCII_A: u8 = 0x61;
fn drive_to_index(drive: char) -> usize {
    assert!(drive.is_ascii_alphabetic());
    (drive.to_ascii_lowercase() as u8 - ASCII_A) as usize
}

fn index_to_drive(index: usize) -> char {
    assert!(index < 26);
    char::from(ASCII_A + index as u8)
}

/// Stringify a native path, Windows-style.
fn stringify_path(drive_prefix: &str, path: &NativePath) -> String {
    let parts = path.components().map(|c| match c {
        Component::RootDir => "",
        // `path` is not a windows path
        Component::Prefix(_) => unreachable!(),
        Component::CurDir => ".",
        Component::ParentDir => "..",
        Component::Normal(part) => part.to_str().expect("path is not utf-8"),
    });

    std::iter::once(drive_prefix)
        .chain(parts)
        .collect::<Vec<&str>>()
        .join(r"\")
}

#[derive(Default)]
struct DriveCache {
    drives: [Option<PathBuf>; 26],
}

impl DriveCache {
    fn from_prefix(prefix: &NativePath) -> Self {
        let drives_dir = prefix.join("dosdevices");
        let mut drive_cache = Self::default();

        for letter in b'a'..=b'z' {
            let drive_name = [letter, b':'];
            let drive_name = std::str::from_utf8(&drive_name).unwrap();
            let drive_dir = drives_dir.join(drive_name);
            if let Ok(target) = drive_dir.read_link() {
                if let Ok(resolved_path) = drives_dir.join(target).canonicalize() {
                    drive_cache.drives[drive_to_index(char::from(letter))] = Some(resolved_path);
                }
            }
        }
        drive_cache
    }

    fn iter(&self) -> impl Iterator<Item = (char, &Path)> {
        self.drives.iter().enumerate().filter_map(|(index, path)| {
            path.as_ref()
                .map(|path| (index_to_drive(index), path.as_ref()))
        })
    }

    fn get(&self, drive_letter: char) -> Option<&Path> {
        self.drives
            .get(drive_to_index(drive_letter))
            .and_then(|path| path.as_ref().map(|path| path.as_ref()))
    }
}

impl Debug for DriveCache {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let mut s = f.debug_struct("DriveCache");
        for (drive_letter, path) in self.iter() {
            s.field(std::str::from_utf8(&[drive_letter as u8]).unwrap(), &path);
        }
        s.finish()
    }
}

/// The main conversion struct: create one of these to do conversions.
///
/// Tracks the WINEPREFIX and the drive letter mappings so they don't have to be recomputed every
/// time you convert a path.
#[derive(Debug)]
pub struct WineConfig {
    prefix: PathBuf,
    drive_cache: DriveCache,
}

impl WineConfig {
    /// Determine the wine prefix from the environment.
    pub fn from_env() -> Result<Self, WinePathError> {
        let prefix = std::env::var_os("WINEPREFIX")
            .map(PathBuf::from)
            .or_else(default_wineprefix)
            .ok_or(WinePathError::PrefixNotFound)?;

        let drive_cache = DriveCache::from_prefix(&prefix);

        Ok(Self {
            prefix,
            drive_cache,
        })
    }

    /// Create a config assuming that the given path is a valid WINEPREFIX.
    ///
    /// Note that this is not validated, and you will end up with empty drive mappings if it is not
    /// actually a wine prefix.
    ///
    /// You can manually validate if a directory is Wine-y *enough* by doing:
    /// ```rust,ignore
    /// use std::path::Path;
    /// fn is_wineprefix_like(some_path: &Path) -> bool {
    ///     some_path.join("dosdevices").is_dir()
    /// }
    /// ```
    pub fn from_prefix(path: impl Into<PathBuf>) -> Self {
        let prefix: PathBuf = path.into();
        let drive_cache = DriveCache::from_prefix(&prefix);

        Self {
            prefix,
            drive_cache,
        }
    }

    /// Get the current wine prefix.
    pub fn prefix(&self) -> &NativePath {
        &self.prefix
    }

    fn find_drive_root<'p>(
        &self,
        path: &'p NativePath,
    ) -> Result<(String, &'p NativePath), WinePathError> {
        for (letter, root) in self.drive_cache.iter() {
            // Returns `err` if `root` is not a parent of `path`.
            if let Ok(remaining) = path.strip_prefix(root) {
                let mut drive = String::new();
                drive.push(letter);
                drive.push(':');
                return Ok((drive, remaining));
            }
        }

        Err(WinePathError::NoDrive)
    }

    fn to_wine_path_inner(&self, path: &NativePath) -> Result<String, WinePathError> {
        let (root, remaining) = self.find_drive_root(path)?;

        Ok(stringify_path(&root, remaining))
    }

    fn to_native_path_inner(&self, path: &str) -> Result<PathBuf, WinePathError> {
        // TODO resolve the path…maybe?
        assert!(path.len() >= 2);
        assert!(
            char::from(path.as_bytes()[0]).is_ascii_alphabetic()
                && char::from(path.as_bytes()[1]) == ':'
        );
        let full_path = path;

        let drive_letter = full_path.chars().next().unwrap();
        if let Some(native_root) = self.drive_cache.get(drive_letter) {
            let mut path = native_root.to_path_buf();
            for part in full_path[2..].split('\\') {
                path.push(part);
            }
            Ok(path)
        } else {
            Err(WinePathError::NoDrive)
        }
    }

    /// Convert a native file path to a Wine path.
    ///
    /// ```rust,no_run
    /// use winepath::WineConfig;
    /// let config = WineConfig::from_env().unwrap();
    /// let path = config.to_wine_path("/home/username/.wine/drive_c/Program Files/CoolApp/start.exe").unwrap();
    /// assert_eq!(path.to_string(), r"c:\Program Files\CoolApp\start.exe");
    /// let path = config.to_wine_path("/home/username/some-path/some-file").unwrap();
    /// assert_eq!(path.to_string(), r"z:\home\username\some-path\some-file");
    /// ```
    #[inline]
    pub fn to_wine_path(&self, path: impl AsRef<NativePath>) -> Result<WinePath, WinePathError> {
        let native = path.as_ref();
        self.to_wine_path_inner(native).map(WinePath)
    }

    /// Convert a Wine path to a native file path.
    ///
    /// ```rust,no_run
    /// use winepath::WineConfig;
    /// use std::path::PathBuf;
    /// let config = WineConfig::from_env().unwrap();
    /// let path = config.to_native_path(r"c:\Program Files\CoolApp\start.exe").unwrap();
    /// assert_eq!(path, PathBuf::from("/home/username/.wine/drive_c/Program Files/CoolApp/start.exe"));
    /// let path = config.to_native_path(r"z:\home\username\some-path\some-file").unwrap();
    /// assert_eq!(path, PathBuf::from("/home/username/some-path/some-file"));
    /// ```
    #[inline]
    pub fn to_native_path(&self, path: impl Into<WinePath>) -> Result<PathBuf, WinePathError> {
        let wine_path = path.into();
        self.to_native_path_inner(wine_path.0.as_ref())
    }
}
