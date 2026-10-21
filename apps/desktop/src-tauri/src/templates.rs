//! User templates (§4 item 41): "Save as Template" writes an OpenDocument
//! spreadsheet template (`.ots`, mimetype
//! `application/vnd.oasis.opendocument.spreadsheet-template`) into
//! `<app data>/templates/`, and "New from Template" lists that folder next to
//! the engine's built-in templates and opens a chosen file as an untitled
//! workbook (the shell forgets the path, so Save asks where).
//!
//! The engine codec writes `.ods`; the shell re-packs the zip with the
//! template mimetype (stored, first entry, as ODF requires). The engine's
//! ODS reader accepts both mimetypes, so the template opens like any file.

use std::fs;
use std::io::{Cursor, Read, Write};
use std::path::{Path, PathBuf};

use tally_api::{ApiError, DocId, FileFormat, WorkbookInfo};
use tauri::{AppHandle, Manager, Runtime};
use zip::write::SimpleFileOptions;

use crate::commands::{blocking, with_service, CmdResult};
use crate::files::TemplateInfo;
use crate::state::{lock, AppState};

const DIR: &str = "templates";
const PREFIX: &str = "user:";
/// The ODF template media type.
pub const TEMPLATE_MIMETYPE: &str = "application/vnd.oasis.opendocument.spreadsheet-template";
/// Longest template name (characters).
pub const MAX_NAME_CHARS: usize = 80;

fn io_err(what: &str, e: &std::io::Error) -> ApiError {
    ApiError::new(ApiError::IO, format!("{what}: {e}"))
}

fn zip_err(what: &str, e: impl std::fmt::Display) -> ApiError {
    ApiError::new(ApiError::IO, format!("{what}: {e}"))
}

fn dir<R: Runtime>(app: &AppHandle<R>) -> Result<PathBuf, ApiError> {
    let base = app
        .path()
        .app_data_dir()
        .map_err(|e| ApiError::new(ApiError::IO, format!("no app data dir: {e}")))?;
    Ok(base.join(DIR))
}

/// A template name becomes the file stem: no path separators or control characters, ≤ 80 chars.
pub fn check_name(name: &str) -> Result<String, ApiError> {
    let n = name.trim();
    if n.is_empty() || n.chars().count() > MAX_NAME_CHARS {
        return Err(ApiError::invalid_argument(format!(
            "a template name has 1 to {MAX_NAME_CHARS} characters"
        )));
    }
    if n.starts_with('.')
        || n.chars().any(|c| {
            c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
        })
    {
        return Err(ApiError::invalid_argument(
            "a template name cannot contain / \\ : * ? \" < > | or start with a dot",
        ));
    }
    Ok(n.to_owned())
}

/// Re-packs an ODS zip as an ODS template (mimetype entry replaced).
pub fn ods_to_ots(ods: &[u8]) -> Result<Vec<u8>, ApiError> {
    let mut archive =
        zip::ZipArchive::new(Cursor::new(ods)).map_err(|e| zip_err("reading the ODS", e))?;
    let mut out = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let stored = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    out.start_file("mimetype", stored)
        .map_err(|e| zip_err("writing the template", e))?;
    out.write_all(TEMPLATE_MIMETYPE.as_bytes())
        .map_err(|e| io_err("writing the template", &e))?;
    let deflated =
        SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| zip_err("reading the ODS", e))?;
        let name = entry
            .name()
            .map_err(|e| zip_err("reading the ODS", e))?
            .into_owned();
        if name == "mimetype" {
            continue;
        }
        if entry.is_dir() {
            out.add_directory(name, deflated)
                .map_err(|e| zip_err("writing the template", e))?;
            continue;
        }
        let mut bytes = Vec::with_capacity(entry.size() as usize);
        entry
            .read_to_end(&mut bytes)
            .map_err(|e| io_err("reading the ODS", &e))?;
        // The manifest declares the package media type too.
        if name == "META-INF/manifest.xml" {
            let text = String::from_utf8_lossy(&bytes).replace(
                "application/vnd.oasis.opendocument.spreadsheet\"",
                &format!("{TEMPLATE_MIMETYPE}\""),
            );
            bytes = text.into_bytes();
        }
        out.start_file(name, deflated)
            .map_err(|e| zip_err("writing the template", e))?;
        out.write_all(&bytes)
            .map_err(|e| io_err("writing the template", &e))?;
    }
    let cursor = out
        .finish()
        .map_err(|e| zip_err("writing the template", e))?;
    Ok(cursor.into_inner())
}

/// The mimetype entry of a package (the first stored entry).
#[cfg(test)]
pub fn mimetype_of(bytes: &[u8]) -> Option<String> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).ok()?;
    let mut entry = archive.by_name("mimetype").ok()?;
    let mut s = String::new();
    entry.read_to_string(&mut s).ok()?;
    Some(s)
}

/// Saves `doc` as `<name>.ots` in the templates folder (through a temporary ODS).
pub fn save_template<R: Runtime>(
    app: &AppHandle<R>,
    st: &AppState,
    doc: DocId,
    name: &str,
) -> CmdResult<PathBuf> {
    let name = check_name(name)?;
    let folder = dir(app)?;
    fs::create_dir_all(&folder).map_err(|e| io_err("creating the templates dir", &e))?;
    let tmp = folder.join(format!(".{name}.tmp.ods"));
    with_service(st, |s| s.save_copy(doc, &tmp, FileFormat::Ods))?;
    let ods = fs::read(&tmp).map_err(|e| io_err("reading the saved copy", &e));
    let _ = fs::remove_file(&tmp);
    let ots = ods_to_ots(&ods?)?;
    let target = folder.join(format!("{name}.ots"));
    fs::write(&target, ots).map_err(|e| io_err("writing the template", &e))?;
    Ok(target)
}

/// The user's templates (`user:<stem>` ids).
pub fn list<R: Runtime>(app: &AppHandle<R>) -> Vec<TemplateInfo> {
    let Ok(folder) = dir(app) else {
        return Vec::new();
    };
    let Ok(entries) = fs::read_dir(&folder) else {
        return Vec::new();
    };
    let mut out: Vec<TemplateInfo> = entries
        .flatten()
        .filter(|e| {
            e.path()
                .extension()
                .is_some_and(|x| x.eq_ignore_ascii_case("ots"))
        })
        .filter_map(|e| {
            let stem = e.path().file_stem()?.to_str()?.to_owned();
            Some(TemplateInfo {
                id: format!("{PREFIX}{stem}"),
                title: stem,
                description: e.path().display().to_string(),
            })
        })
        .collect();
    out.sort_by(|a, b| a.title.to_lowercase().cmp(&b.title.to_lowercase()));
    out
}

/// The file of a `user:<stem>` id, if it exists inside the templates folder.
pub fn path_of<R: Runtime>(app: &AppHandle<R>, id: &str) -> CmdResult<PathBuf> {
    let stem = id
        .strip_prefix(PREFIX)
        .ok_or_else(|| ApiError::invalid_argument("not a user template id"))?;
    let stem = check_name(stem)?;
    let folder = dir(app)?;
    let path = folder.join(format!("{stem}.ots"));
    let canon = fs::canonicalize(&path).map_err(|e| io_err("opening the template", &e))?;
    let folder_canon =
        fs::canonicalize(&folder).map_err(|e| io_err("opening the templates dir", &e))?;
    if !canon.starts_with(&folder_canon) {
        return Err(ApiError::invalid_argument("not a user template"));
    }
    Ok(canon)
}

/// Opens a template file as an untitled workbook (no path, so Save asks).
pub fn open_as_untitled(st: &AppState, path: &Path) -> CmdResult<WorkbookInfo> {
    let info = with_service(st, |s| s.open_untitled(path, false))?;
    lock(&st.paths).remove(&info.doc.0);
    Ok(info)
}

/// `save_as_template(doc, name)`.
#[tauri::command]
pub async fn save_as_template(app: AppHandle, doc: DocId, name: String) -> CmdResult<()> {
    check_name(&name)?;
    blocking(app, move |app, st| {
        save_template(app, st, doc, &name).map(|_| ())
    })
    .await
}

/// `list_user_templates()`.
#[tauri::command]
pub async fn list_user_templates(app: AppHandle) -> CmdResult<Vec<TemplateInfo>> {
    Ok(list(&app))
}

/// `new_from_user_template(id)`.
#[tauri::command]
pub async fn new_from_user_template(app: AppHandle, id: String) -> CmdResult<WorkbookInfo> {
    crate::limits::text_max("the template id", &id, crate::limits::MAX_PATH)?;
    blocking(app, move |app, st| {
        let path = path_of(app, &id)?;
        open_as_untitled(st, &path)
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_checked() {
        assert_eq!(check_name("  Budget 2027 ").unwrap(), "Budget 2027");
        assert!(check_name("").is_err());
        assert!(check_name("../x").is_err());
        assert!(check_name("a/b").is_err());
        assert!(check_name(".hidden").is_err());
        assert!(check_name(&"x".repeat(81)).is_err());
    }

    #[test]
    fn repacks_with_the_template_mimetype() {
        let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let stored =
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        w.start_file("mimetype", stored).unwrap();
        w.write_all(b"application/vnd.oasis.opendocument.spreadsheet")
            .unwrap();
        let d = SimpleFileOptions::default();
        w.start_file("META-INF/manifest.xml", d).unwrap();
        w.write_all(br#"<manifest:file-entry manifest:media-type="application/vnd.oasis.opendocument.spreadsheet" manifest:full-path="/"/>"#).unwrap();
        w.start_file("content.xml", d).unwrap();
        w.write_all(b"<office:document-content/>").unwrap();
        let ods = w.finish().unwrap().into_inner();
        let ots = ods_to_ots(&ods).unwrap();
        assert_eq!(mimetype_of(&ots).as_deref(), Some(TEMPLATE_MIMETYPE));
        let mut a = zip::ZipArchive::new(Cursor::new(&ots)).unwrap();
        assert_eq!(a.by_index(0).unwrap().name().unwrap(), "mimetype");
        let mut m = String::new();
        a.by_name("META-INF/manifest.xml")
            .unwrap()
            .read_to_string(&mut m)
            .unwrap();
        assert!(m.contains(TEMPLATE_MIMETYPE));
        let mut c = String::new();
        a.by_name("content.xml")
            .unwrap()
            .read_to_string(&mut c)
            .unwrap();
        assert_eq!(c, "<office:document-content/>");
    }
}
