//! `list_fonts`: the installed font families and their faces for the font
//! pickers (the toolbar's font box, Format Cells ▸ Font). Metadata only:
//! the list comes from the font-directory scan `tally-io-pdf` already does
//! for printing (fontdb; names, weights, styles, never font data), is cached
//! per process and rescanned on `refresh`.

use tally_io_pdf::{FontFaceInfo, FontFamilyInfo};

use crate::commands::CmdResult;
use tally_api::ApiError;

/// One face of a family (`FontFaceInfo`).
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FontFaceDto {
    /// 100–900 (400 regular, 700 bold).
    pub weight: u16,
    /// Italic or oblique.
    pub italic: bool,
    /// Width class 1–9 (5 normal).
    pub stretch: u16,
}

/// An installed family (`FontFamilyInfo`).
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FontFamilyDto {
    /// Family name.
    pub family: String,
    /// Its faces.
    pub faces: Vec<FontFaceDto>,
    /// Every face is monospaced.
    pub monospaced: bool,
}

fn face_dto(f: &FontFaceInfo) -> FontFaceDto {
    FontFaceDto {
        weight: f.weight,
        italic: f.italic,
        stretch: f.stretch,
    }
}

/// The DTOs of a family list.
pub fn to_dtos(list: &[FontFamilyInfo]) -> Vec<FontFamilyDto> {
    list.iter()
        .map(|f| FontFamilyDto {
            family: f.family.clone(),
            faces: f.faces.iter().map(face_dto).collect(),
            monospaced: f.monospaced,
        })
        .collect()
}

/// The installed font families, sorted by name; `refresh` rescans the font
/// directories (fonts installed while the app runs).
#[tauri::command]
pub async fn list_fonts(refresh: Option<bool>) -> CmdResult<Vec<FontFamilyDto>> {
    let refresh = refresh.unwrap_or(false);
    tauri::async_runtime::spawn_blocking(move || {
        to_dtos(&tally_io_pdf::installed_font_families(refresh))
    })
    .await
    .map_err(|e| ApiError::new(ApiError::INTERNAL, format!("worker failed: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn list_shape_is_camel_case_families_with_faces() {
        // An injected font directory (the bundled Inter files), never the machine's fonts.
        let dir =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../crates/tally-io-pdf/fonts");
        let opts = tally_io_pdf::PdfOptions {
            system_fonts: false,
            font_dirs: vec![dir],
            ..tally_io_pdf::PdfOptions::default()
        };
        let list = to_dtos(&tally_io_pdf::font_families(&opts));
        assert_eq!(list.len(), 1);
        let json = serde_json::to_value(&list).unwrap_or_default();
        assert_eq!(json[0]["family"], "Inter");
        assert_eq!(json[0]["monospaced"], false);
        let faces: Vec<(u64, bool)> = json[0]["faces"]
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|f| (f["weight"].as_u64().unwrap_or(0), f["italic"] == true))
                    .collect()
            })
            .unwrap_or_default();
        assert_eq!(
            faces,
            [(400, false), (400, true), (700, false), (700, true)]
        );
        assert_eq!(json[0]["faces"][0]["stretch"], 5);
        // Metadata only: no bytes / paths in the payload.
        let text = json.to_string();
        assert!(!text.contains("ttf") && !text.contains('/'));
    }
}
