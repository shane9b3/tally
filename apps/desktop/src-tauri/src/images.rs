//! Insert ▸ Image (W5-5, §4 item 42): `pick_image` shows a native Open
//! dialog for PNG/JPEG files, reads the chosen file (≤ 16 MiB), checks the
//! magic bytes (the extension is not trusted), reads the pixel size from
//! the header and returns the bytes as base64 for the `addImage` op. No
//! path ever reaches JS.

use serde::Serialize;
use tally_api::ApiError;
use tauri::AppHandle;
use tauri_plugin_dialog::DialogExt;

use crate::commands::CmdResult;

/// Largest picture accepted (decoded bytes; the engine caps the same).
pub const MAX_IMAGE_BYTES: u64 = 16 * 1024 * 1024;

/// What `pick_image` returns (`PickedImage` in packages/app/src/objects/shell.ts).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PickedImage {
    /// File name without directories.
    pub name: String,
    /// `image/png` or `image/jpeg`.
    pub mime: String,
    /// Base64 bytes.
    pub data: String,
    /// Pixel width from the header, when known.
    pub width: Option<u32>,
    /// Pixel height.
    pub height: Option<u32>,
}

/// The MIME type from the magic bytes (PNG, JPEG), else `None`.
pub fn sniff_mime(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]) {
        Some("image/png")
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        Some("image/jpeg")
    } else {
        None
    }
}

/// Pixel size from a PNG IHDR or the first JPEG SOF marker.
pub fn pixel_size(bytes: &[u8], mime: &str) -> Option<(u32, u32)> {
    match mime {
        "image/png" => {
            if bytes.len() < 24 || &bytes[12..16] != b"IHDR" {
                return None;
            }
            let w = u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
            let h = u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
            (w > 0 && h > 0).then_some((w, h))
        }
        "image/jpeg" => {
            let mut i = 2usize;
            while i + 9 < bytes.len() {
                if bytes[i] != 0xff {
                    return None;
                }
                let marker = bytes[i + 1];
                if marker == 0xff {
                    i += 1;
                    continue;
                }
                let len = u16::from_be_bytes([bytes[i + 2], bytes[i + 3]]) as usize;
                if matches!(marker, 0xc0..=0xc3 | 0xc5..=0xc7 | 0xc9..=0xcb | 0xcd..=0xcf) {
                    let h = u16::from_be_bytes([bytes[i + 5], bytes[i + 6]]) as u32;
                    let w = u16::from_be_bytes([bytes[i + 7], bytes[i + 8]]) as u32;
                    return (w > 0 && h > 0).then_some((w, h));
                }
                if len < 2 {
                    return None;
                }
                i += 2 + len;
            }
            None
        }
        _ => None,
    }
}

/// Standard base64 (with padding), without a dependency.
pub fn base64_encode(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0];
        let b1 = chunk.get(1).copied().unwrap_or(0);
        let b2 = chunk.get(2).copied().unwrap_or(0);
        out.push(TABLE[(b0 >> 2) as usize] as char);
        out.push(TABLE[(((b0 & 0x03) << 4) | (b1 >> 4)) as usize] as char);
        if chunk.len() > 1 {
            out.push(TABLE[(((b1 & 0x0f) << 2) | (b2 >> 6)) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(TABLE[(b2 & 0x3f) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

/// Validates a picture's bytes and builds the result.
pub fn picked_image(name: &str, bytes: &[u8]) -> CmdResult<PickedImage> {
    if bytes.len() as u64 > MAX_IMAGE_BYTES {
        return Err(ApiError::invalid_argument(
            "the picture is larger than 16 MB",
        ));
    }
    let mime = sniff_mime(bytes)
        .ok_or_else(|| ApiError::invalid_argument("only PNG and JPEG pictures can be inserted"))?;
    let size = pixel_size(bytes, mime);
    Ok(PickedImage {
        name: name.to_owned(),
        mime: mime.to_owned(),
        data: base64_encode(bytes),
        width: size.map(|s| s.0),
        height: size.map(|s| s.1),
    })
}

/// Reads and validates a picture file (size checked before reading).
pub fn read_picture(path: &std::path::Path) -> CmdResult<PickedImage> {
    let meta = std::fs::metadata(path)
        .map_err(|e| ApiError::new(ApiError::IO, format!("reading the picture: {e}")))?;
    if meta.len() > MAX_IMAGE_BYTES {
        return Err(ApiError::invalid_argument(
            "the picture is larger than 16 MB",
        ));
    }
    let bytes = std::fs::read(path)
        .map_err(|e| ApiError::new(ApiError::IO, format!("reading the picture: {e}")))?;
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("Picture")
        .to_owned();
    picked_image(&name, &bytes)
}

/// `pick_image`: Open dialog (PNG, JPEG), then the validated bytes as
/// base64; `null` when the user cancels.
#[tauri::command]
pub async fn pick_image(app: AppHandle) -> CmdResult<Option<PickedImage>> {
    let handle = app.clone();
    let picked = tauri::async_runtime::spawn_blocking(move || {
        handle
            .dialog()
            .file()
            .set_title("Insert Image")
            .add_filter("Pictures", &["png", "jpg", "jpeg"])
            .add_filter("PNG", &["png"])
            .add_filter("JPEG", &["jpg", "jpeg"])
            .blocking_pick_file()
    })
    .await
    .map_err(|e| ApiError::new(ApiError::INTERNAL, format!("dialog thread failed: {e}")))?;
    let Some(file) = picked else {
        return Ok(None);
    };
    let path = file
        .into_path()
        .map_err(|e| ApiError::invalid_argument(format!("unsupported location: {e}")))?;
    tauri::async_runtime::spawn_blocking(move || read_picture(&path).map(Some))
        .await
        .map_err(|e| ApiError::new(ApiError::INTERNAL, format!("worker failed: {e}")))?
}

#[cfg(test)]
mod tests {
    use super::*;

    const PNG_1X1: &[u8] = &[
        0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 13, b'I', b'H', b'D', b'R', 0, 0,
        0, 2, 0, 0, 0, 3, 8, 6, 0, 0, 0,
    ];

    #[test]
    fn sniffs_png_and_jpeg_only() {
        assert_eq!(sniff_mime(PNG_1X1), Some("image/png"));
        assert_eq!(
            sniff_mime(&[0xff, 0xd8, 0xff, 0xe0, 0, 16]),
            Some("image/jpeg")
        );
        assert_eq!(sniff_mime(b"GIF89a"), None);
        assert_eq!(sniff_mime(b"<svg/>"), None);
        assert_eq!(sniff_mime(b""), None);
    }

    #[test]
    fn reads_png_ihdr_size() {
        assert_eq!(pixel_size(PNG_1X1, "image/png"), Some((2, 3)));
        assert_eq!(pixel_size(&PNG_1X1[..10], "image/png"), None);
    }

    #[test]
    fn reads_jpeg_sof_size() {
        // SOI, APP0 (length 16, 14 bytes payload), SOF0 with 7x5.
        let mut jpg = vec![0xff, 0xd8, 0xff, 0xe0, 0, 16];
        jpg.extend_from_slice(&[0; 14]);
        jpg.extend_from_slice(&[0xff, 0xc0, 0, 17, 8, 0, 5, 0, 7, 3]);
        jpg.extend_from_slice(&[0; 20]);
        assert_eq!(pixel_size(&jpg, "image/jpeg"), Some((7, 5)));
    }

    #[test]
    fn base64_matches_known_vectors() {
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn picked_image_validates_magic_and_size() {
        let p = picked_image("a.png", PNG_1X1).unwrap();
        assert_eq!(p.mime, "image/png");
        assert_eq!((p.width, p.height), (Some(2), Some(3)));
        assert_eq!(p.data, base64_encode(PNG_1X1));
        let e = picked_image("a.png", b"not an image").unwrap_err();
        assert_eq!(e.code, ApiError::INVALID_ARGUMENT);
        let big = vec![0u8; MAX_IMAGE_BYTES as usize + 1];
        assert_eq!(
            picked_image("big.png", &big).unwrap_err().code,
            ApiError::INVALID_ARGUMENT
        );
    }
}
