//! What the blob view says about a binary file it does not preview: the format, named from
//! the magic number in its first bytes or else from its extension. The template adds the size
//! and the download link. No parser runs on the content, only prefix compares.

/// Magic numbers: (offset, bytes, format). The first match wins, so a container sits after
/// the formats built on it (EPUB and the Office formats are ZIP files).
const MAGIC: &[(usize, &[u8], &str)] = &[
    (0, b"SQLite format 3\0", "SQLite database"),
    (30, b"mimetypeapplication/epub+zip", "EPUB book"),
    (0, b"PK\x03\x04", "ZIP archive"),
    (0, b"PK\x05\x06", "ZIP archive"),
    (0, b"\x1f\x8b", "gzip archive"),
    (0, b"BZh", "bzip2 archive"),
    (0, b"\xfd7zXZ\0", "xz archive"),
    (0, b"(\xb5/\xfd", "Zstandard archive"),
    (0, b"7z\xbc\xaf\x27\x1c", "7z archive"),
    (0, b"Rar!\x1a\x07", "RAR archive"),
    (257, b"ustar", "tar archive"),
    (0, b"\x7fELF", "ELF executable"),
    (0, b"MZ", "Windows executable"),
    (0, b"\xcf\xfa\xed\xfe", "Mach-O executable"),
    (
        0,
        b"\xca\xfe\xba\xbe",
        "Mach-O universal binary or Java class",
    ),
    (0, b"\0asm", "WebAssembly module"),
    (0, b"glTF", "glTF binary model"),
    (0, b"wOFF", "WOFF font"),
    (0, b"wOF2", "WOFF2 font"),
    (0, b"OTTO", "OpenType font"),
    (0, b"\0\x01\0\0", "TrueType font"),
    (0, b"%PDF-", "PDF document"),
    (0, b"\x89PNG", "PNG image"),
    (0, b"GIF8", "GIF image"),
    (0, b"\xff\xd8\xff", "JPEG image"),
    (
        0,
        b"\xd0\xcf\x11\xe0\xa1\xb1\x1a\xe1",
        "Microsoft Office (legacy) document",
    ),
    (0, b"PAR1", "Parquet file"),
    (0, b"\x93NUMPY", "NumPy array"),
    (0, b"\x89HDF\r\n\x1a\n", "HDF5 file"),
    (0, b"ARROW1", "Arrow file"),
];

/// By extension, for formats without a fixed magic number (or when the head is not read).
fn by_ext(path: &str) -> Option<&'static str> {
    let ext = path.rsplit_once('.')?.1.to_ascii_lowercase();
    Some(match ext.as_str() {
        "sqlite" | "sqlite3" | "db" => "SQLite database",
        "epub" => "EPUB book",
        "zip" | "jar" => "ZIP archive",
        "gz" | "tgz" => "gzip archive",
        "tar" => "tar archive",
        "stl" => "STL 3D model",
        "obj" => "OBJ 3D model",
        "glb" => "glTF binary model",
        "fbx" => "FBX 3D model",
        "3mf" => "3MF 3D model",
        "blend" => "Blender file",
        "ttf" => "TrueType font",
        "otf" => "OpenType font",
        "woff" => "WOFF font",
        "woff2" => "WOFF2 font",
        "wasm" => "WebAssembly module",
        "exe" | "dll" => "Windows executable",
        "docx" => "Word document",
        "xlsx" => "Excel workbook",
        "pptx" => "PowerPoint presentation",
        "psd" => "Photoshop image",
        "parquet" => "Parquet file",
        "npy" => "NumPy array",
        _ => return None,
    })
}

/// The format of a binary blob: its magic number first, since an extension can lie, then the
/// extension. The Office XML formats are ZIP files, so a ZIP head defers to their extension.
pub(crate) fn describe(path: &str, head: &[u8]) -> &'static str {
    let magic = MAGIC
        .iter()
        .find(|(at, m, _)| head.get(*at..at + m.len()) == Some(*m))
        .map(|(_, _, name)| *name);
    match (magic, by_ext(path)) {
        (Some("ZIP archive") | None, Some(e)) => e,
        (Some(m), _) => m,
        (None, None) => "Binary file",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(offset: usize, magic: &[u8]) -> Vec<u8> {
        let mut v = vec![0x01; offset];
        v.extend_from_slice(magic);
        v
    }

    #[test]
    fn magic_names_the_format_whatever_the_extension() {
        assert_eq!(
            describe("data.bin", b"SQLite format 3\0rest"),
            "SQLite database"
        );
        assert_eq!(describe("a.txt", b"\x7fELF\x02"), "ELF executable");
        assert_eq!(describe("backup", &at(257, b"ustar")), "tar archive");
    }

    #[test]
    fn epub_wins_over_the_zip_it_is_built_on() {
        let mut head = b"PK\x03\x04".to_vec();
        head.resize(30, 0);
        head.extend_from_slice(b"mimetypeapplication/epub+zip");
        assert_eq!(describe("book", &head), "EPUB book");
    }

    #[test]
    fn a_zip_head_defers_to_an_office_extension() {
        assert_eq!(describe("report.docx", b"PK\x03\x04"), "Word document");
        assert_eq!(describe("x.zip", b"PK\x03\x04"), "ZIP archive");
    }

    #[test]
    fn the_extension_names_formats_without_magic() {
        assert_eq!(describe("part.stl", b"\x01\x02"), "STL 3D model");
        assert_eq!(describe("blob", b"\x01\x02"), "Binary file");
    }

    #[test]
    fn a_short_head_matches_nothing_and_never_panics() {
        assert_eq!(describe("x", b""), "Binary file");
        assert_eq!(describe("x", b"SQL"), "Binary file");
    }
}
