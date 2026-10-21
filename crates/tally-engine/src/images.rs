//! Pictures (wave 5, W5-1): validation of the bytes (PNG/JPEG magic,
//! size cap) and the DTO conversions. Placement reuses the chart anchor.

use tally_api::{ApiError, ImageDto};
use tally_model::ImageData;
use tally_types::SheetId;

use crate::charts::{anchor_from_api, anchor_to_api, Image, MAX_IMAGE_BYTES};
use crate::workbook::Workbook;

/// Most picture bytes per document (all sheets).
pub const MAX_TOTAL_IMAGE_BYTES: usize = 256 * 1024 * 1024;

/// The media type the bytes announce (`image/png`, `image/jpeg`), or
/// `None` for anything else.
pub fn sniff_mime(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(b"\xff\xd8\xff") {
        Some("image/jpeg")
    } else {
        None
    }
}

/// Decodes and checks `image.data` (base64, ≤ 16 MiB decoded, PNG or
/// JPEG magic matching `mime`).
pub fn bytes_from_api(image: &ImageDto) -> Result<(Vec<u8>, String), ApiError> {
    if image.data.len() > MAX_IMAGE_BYTES / 3 * 4 + 8 {
        return Err(ApiError::invalid_argument(
            "the picture is larger than 16 MiB",
        ));
    }
    let bytes = crate::protection::base64_decode(image.data.trim())
        .ok_or_else(|| ApiError::invalid_argument("the picture data is not base64"))?;
    if bytes.len() > MAX_IMAGE_BYTES {
        return Err(ApiError::invalid_argument(
            "the picture is larger than 16 MiB",
        ));
    }
    let mime = sniff_mime(&bytes)
        .ok_or_else(|| ApiError::invalid_argument("only PNG and JPEG pictures are supported"))?;
    if !image.mime.is_empty() && !image.mime.eq_ignore_ascii_case(mime) {
        return Err(ApiError::invalid_argument(format!(
            "the picture is {mime}, not {}",
            image.mime
        )));
    }
    Ok((bytes, mime.to_string()))
}

/// A picture from the API (bytes decoded and checked, anchor validated).
pub fn from_api(wb: &Workbook, sheet: SheetId, image: &ImageDto) -> Result<Image, ApiError> {
    let (bytes, mime) = bytes_from_api(image)?;
    let total: usize = wb
        .sheets
        .iter()
        .flat_map(|s| s.meta.images.iter())
        .map(|i| i.data.bytes.len())
        .sum();
    if total.saturating_add(bytes.len()) > MAX_TOTAL_IMAGE_BYTES {
        return Err(ApiError::invalid_argument(
            "the document already holds 256 MiB of pictures",
        ));
    }
    let _ = sheet;
    Ok(Image {
        id: 0,
        data: ImageData {
            name: image.name.chars().take(64).collect(),
            anchor: anchor_from_api(&image.anchor)?,
            mime,
            bytes,
        },
    })
}

/// The API view of a picture, with or without its bytes.
pub fn to_api(image: &Image, with_data: bool) -> ImageDto {
    ImageDto {
        id: image.id,
        name: image.data.name.clone(),
        anchor: anchor_to_api(&image.data.anchor),
        mime: image.data.mime.clone(),
        data: if with_data {
            crate::protection::base64(&image.data.bytes)
        } else {
            String::new()
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sniffs_magic() {
        assert_eq!(sniff_mime(b"\x89PNG\r\n\x1a\nxxxx"), Some("image/png"));
        assert_eq!(sniff_mime(b"\xff\xd8\xff\xe0"), Some("image/jpeg"));
        assert_eq!(sniff_mime(b"GIF89a"), None);
        assert_eq!(sniff_mime(b""), None);
    }
}
