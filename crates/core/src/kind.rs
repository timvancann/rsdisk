//! What sort of thing a file is, from its name alone (spec §3.6). Eight kinds, one per
//! terminal colour of a theme.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Kind {
    Video,
    Audio,
    Image,
    Archive,
    Document,
    Code,
    App,
    Other,
}

impl Kind {
    pub const ALL: [Kind; 8] = [
        Kind::Video,
        Kind::Audio,
        Kind::Image,
        Kind::Archive,
        Kind::Document,
        Kind::Code,
        Kind::App,
        Kind::Other,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Kind::Video => "video",
            Kind::Audio => "audio",
            Kind::Image => "image",
            Kind::Archive => "archive",
            Kind::Document => "document",
            Kind::Code => "code",
            Kind::App => "app",
            Kind::Other => "other",
        }
    }
}

/// The kind of a file or directory name. Only the extension matters, case-insensitively; a
/// leading dot is part of the name, not an extension (`.zshrc` is Other).
pub fn kind_of(name: &str) -> Kind {
    let Some((stem, ext)) = name.rsplit_once('.') else {
        return Kind::Other;
    };
    if stem.is_empty() {
        return Kind::Other;
    }
    let ext = ext.to_ascii_lowercase();
    match ext.as_str() {
        "mp4" | "mov" | "mkv" | "avi" | "m4v" | "webm" | "wmv" | "flv" | "mpg" | "mpeg" | "mts"
        | "m2ts" | "3gp" => Kind::Video,
        "mp3" | "m4a" | "aac" | "flac" | "wav" | "aif" | "aiff" | "ogg" | "opus" | "wma"
        | "alac" | "caf" | "mid" | "midi" => Kind::Audio,
        "jpg" | "jpeg" | "png" | "gif" | "heic" | "heif" | "webp" | "tif" | "tiff" | "bmp"
        | "raw" | "cr2" | "cr3" | "nef" | "arw" | "dng" | "psd" | "svg" | "ico" | "icns"
        | "avif" => Kind::Image,
        "zip" | "tar" | "gz" | "tgz" | "bz2" | "xz" | "zst" | "7z" | "rar" | "dmg" | "iso"
        | "pkg" | "img" | "sparseimage" | "sparsebundle" | "xip" | "jar" | "whl" => Kind::Archive,
        "pdf" | "doc" | "docx" | "xls" | "xlsx" | "ppt" | "pptx" | "pages" | "numbers" | "key"
        | "odt" | "ods" | "odp" | "rtf" | "txt" | "md" | "csv" | "epub" | "log" => Kind::Document,
        "rs" | "c" | "h" | "cc" | "cpp" | "hpp" | "m" | "mm" | "swift" | "go" | "py" | "pyc"
        | "js" | "mjs" | "cjs" | "ts" | "tsx" | "jsx" | "java" | "class" | "kt" | "rb" | "php"
        | "cs" | "json" | "toml" | "yaml" | "yml" | "xml" | "html" | "css" | "scss" | "svelte"
        | "vue" | "sh" | "zsh" | "lua" | "sql" | "o" | "a" | "rlib" | "rmeta" | "d" | "map"
        | "wasm" | "lock" => Kind::Code,
        "app" | "dylib" | "so" | "framework" | "bundle" | "kext" | "appex" | "exe" | "dll" => {
            Kind::App
        }
        _ => Kind::Other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_per_kind() {
        assert_eq!(kind_of("holiday.mov"), Kind::Video);
        assert_eq!(kind_of("song.flac"), Kind::Audio);
        assert_eq!(kind_of("IMG_0001.heic"), Kind::Image);
        assert_eq!(kind_of("Xcode.xip"), Kind::Archive);
        assert_eq!(kind_of("invoice.pdf"), Kind::Document);
        assert_eq!(kind_of("main.rs"), Kind::Code);
        assert_eq!(kind_of("libfoo.dylib"), Kind::App);
        assert_eq!(kind_of("data.bin"), Kind::Other);
    }

    #[test]
    fn extensions_are_case_insensitive() {
        assert_eq!(kind_of("IMG_0001.JPG"), Kind::Image);
        assert_eq!(kind_of("Movie.MP4"), Kind::Video);
    }

    #[test]
    fn no_extension_is_other() {
        assert_eq!(kind_of("Makefile"), Kind::Other);
        assert_eq!(kind_of("trailing."), Kind::Other);
    }

    #[test]
    fn dotfiles_are_other() {
        assert_eq!(kind_of(".zshrc"), Kind::Other);
        assert_eq!(kind_of(".DS_Store"), Kind::Other);
    }

    #[test]
    fn last_extension_wins() {
        assert_eq!(kind_of("backup.tar.gz"), Kind::Archive);
        assert_eq!(kind_of("notes.md.pdf"), Kind::Document);
    }

    #[test]
    fn app_bundles_by_directory_name() {
        assert_eq!(kind_of("Safari.app"), Kind::App);
    }
}
