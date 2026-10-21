//! Shell side of printing and PDF export (W5-5, over the W5-3 engine
//! pipeline): `wb_export_pdf` asks the engine for the job's PDF, shows a
//! native Save dialog and writes the bytes atomically; `wb_print` shows the
//! PDF in a small `tally-print://` webview window and calls
//! `Webview::print()` (macOS; the only platform where wry implements it),
//! or opens the temporary PDF with the system viewer elsewhere. Until W5-3
//! lands, `print_job_pdf` answers `unsupported`, which the UI shows as a
//! toast.
//!
//! No file system or shell API is exposed to JS: the dialog, the write and
//! the viewer launch all happen here, and the print window only ever
//! receives PDFs the engine produced for this process.

use std::borrow::Cow;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use tally_api::{ApiError, DocId};
use tauri::http::{Request, Response, StatusCode};
use tauri::{AppHandle, Manager, Runtime, State, UriSchemeContext};
use tauri_plugin_dialog::DialogExt;

use crate::commands::{blocking, with_service, CmdResult};
use crate::state::{lock, AppState};

/// The custom URI scheme the print window loads its PDF from.
pub const PRINT_SCHEME: &str = "tally-print";
/// Largest PDF the shell handles (matches the engine's own caps generously).
pub const MAX_PDF_BYTES: usize = 512 * 1024 * 1024;

/// PDFs of the jobs sent to print windows (job id → bytes), managed state.
#[derive(Default)]
pub struct PrintJobs(pub Mutex<HashMap<u32, Vec<u8>>>);

/// Registers the managed print-job store (called from `lib.rs` setup).
pub fn setup<R: Runtime>(app: &AppHandle<R>) {
    app.manage(PrintJobs::default());
}

fn cancelled() -> ApiError {
    ApiError::new(ApiError::CANCELLED, "cancelled")
}

fn join_err(e: impl std::fmt::Display) -> ApiError {
    ApiError::new(ApiError::INTERNAL, format!("dialog thread failed: {e}"))
}

fn io_err(what: &str, e: &std::io::Error) -> ApiError {
    ApiError::new(ApiError::IO, format!("{what}: {e}"))
}

/// The PDF bytes of a prepared job from the engine (`print_job_pdf`).
pub fn pdf_bytes(state: &AppState, job: u32) -> CmdResult<Vec<u8>> {
    let bytes = with_service(state, |s| s.print_job_pdf(job))?;
    if bytes.len() > MAX_PDF_BYTES {
        return Err(ApiError::invalid_argument(format!(
            "the PDF is too large ({} bytes)",
            bytes.len()
        )));
    }
    if !bytes.starts_with(b"%PDF") {
        return Err(ApiError::new(
            ApiError::INTERNAL,
            "the engine did not return a PDF",
        ));
    }
    Ok(bytes)
}

/// The suggested PDF file name for a document: its file stem (else
/// "Untitled") plus `.pdf`.
pub fn pdf_file_name(current: Option<&Path>) -> String {
    let stem = current
        .and_then(|p| p.file_stem())
        .and_then(|s| s.to_str())
        .unwrap_or("Untitled");
    format!("{stem}.pdf")
}

/// Writes `bytes` to `target` atomically: a temporary file next to it,
/// then a rename, so a crash never leaves a half-written PDF.
pub fn write_pdf_atomically(target: &Path, bytes: &[u8]) -> CmdResult<()> {
    let dir = target
        .parent()
        .filter(|d| !d.as_os_str().is_empty())
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let tmp = dir.join(format!(".tally-pdf-{}-{nanos}.tmp", std::process::id()));
    std::fs::write(&tmp, bytes).map_err(|e| io_err("writing the PDF", &e))?;
    if let Err(e) = std::fs::rename(&tmp, target) {
        let _ = std::fs::remove_file(&tmp);
        return Err(io_err("saving the PDF", &e));
    }
    Ok(())
}

/// The path of the temporary PDF opened in the system viewer (non-macOS).
pub fn temp_pdf_path(job: u32) -> PathBuf {
    std::env::temp_dir().join(format!("tally-print-{}-{job}.pdf", std::process::id()))
}

/// Parses the path of a `tally-print://localhost/<job>.pdf` request.
pub fn job_of_path(path: &str) -> Option<u32> {
    let name = path.trim_start_matches('/');
    let stem = name.strip_suffix(".pdf")?;
    if stem.is_empty() || stem.len() > 10 || !stem.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    stem.parse().ok()
}

/// The URL the print window loads for a job.
pub fn print_url(job: u32) -> String {
    format!("{PRINT_SCHEME}://localhost/{job}.pdf")
}

/// Serves the stored PDF of a job (or 404) for the `tally-print` scheme.
pub fn protocol<R: Runtime>(
    ctx: UriSchemeContext<'_, R>,
    req: Request<Vec<u8>>,
) -> Response<Cow<'static, [u8]>> {
    let not_found = || {
        Response::builder()
            .status(StatusCode::NOT_FOUND)
            .header("Content-Type", "text/plain")
            .body(Cow::Borrowed(b"not found".as_slice()))
            .unwrap_or_default()
    };
    let Some(job) = job_of_path(req.uri().path()) else {
        return not_found();
    };
    let jobs = ctx.app_handle().state::<PrintJobs>();
    let bytes = lock(&jobs.0).get(&job).cloned();
    match bytes {
        Some(b) => Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "application/pdf")
            .header("Content-Length", b.len().to_string())
            .header("Cache-Control", "no-store")
            .body(Cow::Owned(b))
            .unwrap_or_else(|_| not_found()),
        None => not_found(),
    }
}

/// `wb_export_pdf(doc, job)`: the PDF first (so `unsupported` is reported
/// before any dialog), then a Save dialog, then an atomic write.
pub async fn export_pdf(
    app: AppHandle,
    state: State<'_, AppState>,
    doc: DocId,
    job: u32,
) -> CmdResult<()> {
    let bytes = blocking(app.clone(), move |_, st| pdf_bytes(st, job)).await?;
    let current: Option<PathBuf> = lock(&state.paths).get(&doc.0).cloned();
    let file_name = pdf_file_name(current.as_deref());
    let handle = app.clone();
    let picked = tauri::async_runtime::spawn_blocking(move || {
        handle
            .dialog()
            .file()
            .set_title("Export as PDF")
            .set_file_name(file_name)
            .add_filter("PDF", &["pdf"])
            .blocking_save_file()
    })
    .await
    .map_err(join_err)?;
    let file = picked.ok_or_else(cancelled)?;
    let mut path = file
        .into_path()
        .map_err(|e| ApiError::invalid_argument(format!("unsupported location: {e}")))?;
    if path
        .extension()
        .and_then(|e| e.to_str())
        .is_none_or(|e| !e.eq_ignore_ascii_case("pdf"))
    {
        path.set_extension("pdf");
    }
    tauri::async_runtime::spawn_blocking(move || write_pdf_atomically(&path, &bytes))
        .await
        .map_err(join_err)?
}

/// `wb_print(doc, job)`: macOS shows the PDF in a print window and opens the
/// print dialog; other platforms hand a temporary PDF to the system viewer.
pub async fn print(app: AppHandle, doc: DocId, job: u32) -> CmdResult<()> {
    let _ = doc;
    let bytes = blocking(app.clone(), move |_, st| pdf_bytes(st, job)).await?;
    if cfg!(target_os = "macos") {
        print_in_window(&app, job, bytes)
    } else {
        open_in_viewer(job, &bytes)
    }
}

/// Stores the PDF for the `tally-print` scheme and opens the print window.
fn print_in_window(app: &AppHandle, job: u32, bytes: Vec<u8>) -> CmdResult<()> {
    {
        let jobs = app.state::<PrintJobs>();
        let mut map = lock(&jobs.0);
        if map.len() >= 4 {
            let oldest = map.keys().min().copied();
            if let Some(k) = oldest {
                map.remove(&k);
            }
        }
        map.insert(job, bytes);
    }
    let label = format!("tally-print-{job}");
    if let Some(existing) = app.get_webview_window(&label) {
        let _ = existing.close();
    }
    let url = tauri::Url::parse(&print_url(job))
        .map_err(|e| ApiError::new(ApiError::INTERNAL, format!("print url: {e}")))?;
    let window = tauri::webview::WebviewWindowBuilder::new(
        app,
        &label,
        tauri::WebviewUrl::CustomProtocol(url),
    )
    .title("Print")
    .inner_size(640.0, 820.0)
    .center()
    .on_page_load(|window, payload| {
        if payload.event() == tauri::webview::PageLoadEvent::Finished {
            if let Err(e) = window.print() {
                eprintln!("tally: print dialog failed: {e}");
            }
        }
    })
    .build()
    .map_err(|e| ApiError::new(ApiError::INTERNAL, format!("print window: {e}")))?;
    let _ = window.set_focus();
    Ok(())
}

/// Writes the PDF to the temp dir and opens it with the OS default viewer.
fn open_in_viewer(job: u32, bytes: &[u8]) -> CmdResult<()> {
    let path = temp_pdf_path(job);
    write_pdf_atomically(&path, bytes)?;
    let (program, args) =
        crate::links::launcher_argv(&path.to_string_lossy(), std::env::consts::OS);
    let status = std::process::Command::new(program)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map_err(|e| ApiError::new(ApiError::IO, format!("cannot open the PDF viewer: {e}")))?;
    if status.success() {
        Ok(())
    } else {
        Err(ApiError::new(
            ApiError::IO,
            "the PDF viewer could not be started",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assist::build_service;

    #[test]
    fn pdf_file_name_follows_the_document() {
        assert_eq!(pdf_file_name(Some(Path::new("/tmp/Book.ods"))), "Book.pdf");
        assert_eq!(pdf_file_name(Some(Path::new("Report.XLSX"))), "Report.pdf");
        assert_eq!(pdf_file_name(None), "Untitled.pdf");
    }

    #[test]
    fn job_paths_parse_strictly() {
        assert_eq!(job_of_path("/7.pdf"), Some(7));
        assert_eq!(job_of_path("12.pdf"), Some(12));
        assert_eq!(job_of_path("/x.pdf"), None);
        assert_eq!(job_of_path("/7.PDF"), None);
        assert_eq!(job_of_path("/../7.pdf"), None);
        assert_eq!(job_of_path("/.pdf"), None);
        assert_eq!(job_of_path("/99999999999.pdf"), None);
        assert_eq!(print_url(3), "tally-print://localhost/3.pdf");
        assert_eq!(
            job_of_path("/3.pdf"),
            job_of_path(&print_url(3)[print_url(3).find("/3").unwrap()..])
        );
    }

    #[test]
    fn atomic_write_leaves_no_temp_file() {
        let dir = std::env::temp_dir().join(format!("tally-print-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let target = dir.join("out.pdf");
        write_pdf_atomically(&target, b"%PDF-1.7\n%%EOF\n").unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"%PDF-1.7\n%%EOF\n");
        let leftovers: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `export_pdf_job_bytes`: the shell asks the engine for the job's PDF;
    /// until W5-3 lands this is `unsupported` (or `not_found` for an unknown
    /// job once it does), and either way the result is a clean `ApiError`,
    /// never a panic.
    #[test]
    fn export_pdf_job_bytes() {
        let st = AppState::new(Box::new(build_service()));
        let err = pdf_bytes(&st, 4_000_000).unwrap_err();
        assert!(
            err.code == ApiError::UNSUPPORTED || err.code == ApiError::NOT_FOUND,
            "{}: {}",
            err.code,
            err.message
        );
    }
}
