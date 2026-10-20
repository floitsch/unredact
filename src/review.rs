// Copyright (C) 2026 Toit contributors.
//! Local, read-only target review. The browser downloads validated target specs;
//! only the existing `target` command writes them to a project.
use crate::{
    context_layout, contextual,
    project::{Project, Target, hash},
    raster::{GrayImage, Rect},
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    fs,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    time::Duration,
};

/// The same checks used by CLI target registration, including pixel evidence
/// and calibration exclusions that are additional to structural validation.
pub fn validate_target_registration(project: &Project, dir: &Path, target: &Target) -> Result<()> {
    project.validate_target(target)?;
    crate::scan_span::validate_pixels(project.page(&target.page)?, target, dir)?;
    let profile = project.profile(&target.profile)?;
    let page = project.page(&target.page)?;
    ensure!(
        !page.words.iter().any(|w| {
            w.bounds.intersects(target.bounds)
                && (profile.training_texts.contains(&w.text)
                    || profile.validation_texts.contains(&w.text)
                    || profile.audit_texts.contains(&w.text))
        }) && profile.spacing.as_ref().is_none_or(|s| s
            .samples
            .iter()
            .all(|sample| !sample.bounds.intersects(target.bounds))),
        "Target overlaps calibration/validation text. Use a held-out region or remove those words and recalibrate."
    );
    Ok(())
}

#[derive(Deserialize)]
struct Request {
    target: Target,
    image_sha256: String,
    #[serde(default)]
    paragraph_context: bool,
}

#[derive(Serialize)]
struct Exclusion {
    bounds: Rect,
    kind: &'static str,
    label: String,
}

struct Session {
    dir: PathBuf,
    project: Project,
    page_id: String,
    bmp: Vec<u8>,
}

fn verified_image(project: &Project, dir: &Path, page_id: &str) -> Result<GrayImage> {
    let page = project.page(page_id)?;
    let image = GrayImage::read(&dir.join(&page.image))?;
    ensure!(
        image.width == page.width
            && image.height == page.height
            && hash(&image.pixels) == page.image_sha256,
        "Page image dimensions or pixel fingerprint differ from imported evidence"
    );
    Ok(image)
}

// A lossless, uncompressed indexed BMP preserves the exact imported gray
// values. No ImageMagick/browser image format support is needed for PGM input.
fn bitmap(image: &GrayImage) -> Vec<u8> {
    let stride = (image.width + 3) & !3;
    let offset = 14 + 40 + 256 * 4;
    let length = offset + stride * image.height;
    let mut out = Vec::with_capacity(length);
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&(length as u32).to_le_bytes());
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&(offset as u32).to_le_bytes());
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&(image.width as i32).to_le_bytes());
    out.extend_from_slice(&(image.height as i32).to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&8u16.to_le_bytes());
    out.extend_from_slice(&[0; 24]);
    for gray in 0..=255 {
        out.extend_from_slice(&[gray, gray, gray, 0]);
    }
    for row in image.pixels.chunks_exact(image.width).rev() {
        out.extend_from_slice(row);
        out.resize(out.len() + stride - image.width, 0);
    }
    out
}

impl Session {
    fn open(dir: &Path, page_id: Option<&str>) -> Result<Self> {
        let project = Project::load(dir)?;
        let page_id = page_id
            .or_else(|| project.pages.first().map(|p| p.id.as_str()))
            .context("Project has no pages")?
            .to_owned();
        let image = verified_image(&project, dir, &page_id)?;
        Ok(Self {
            dir: dir.to_owned(),
            project,
            page_id,
            bmp: bitmap(&image),
        })
    }

    fn data(&self) -> Result<Value> {
        let page = self.project.page(&self.page_id)?;
        let profiles: Vec<_> = self
            .project
            .profiles
            .iter()
            .filter(|p| p.page == page.id)
            .map(|profile| {
                let mut exclusions = vec![];
                for word in &page.words {
                    let kind = if profile.audit_texts.contains(&word.text) {
                        "audit"
                    } else if profile.validation_texts.contains(&word.text) {
                        "validation"
                    } else if profile.training_texts.contains(&word.text) {
                        "training"
                    } else {
                        continue;
                    };
                    exclusions.push(Exclusion {
                        bounds: word.bounds,
                        kind,
                        label: word.text.clone(),
                    });
                }
                if let Some(spacing) = &profile.spacing {
                    exclusions.extend(spacing.samples.iter().map(|sample| Exclusion {
                        bounds: sample.bounds,
                        kind: "spacing",
                        label: "Spacing calibration".into(),
                    }));
                }
                if let Some(neural) = &profile.neural_search {
                    for input in &neural.inputs {
                        exclusions.extend(input.samples.iter().map(|sample| Exclusion {
                            bounds: sample.bounds,
                            kind: "neural",
                            label: "Font selection".into(),
                        }));
                    }
                }
                json!({"id": profile.id, "exclusions": exclusions})
            })
            .collect();
        let targets: Vec<_> = self
            .project
            .targets
            .iter()
            .filter(|t| t.page == page.id)
            .collect();
        Ok(json!({
            "page": {"id": page.id, "width": page.width, "height": page.height,
                "image_sha256": page.image_sha256, "words": page.words,
                "proposed_redactions": page.proposed_redactions},
            "profiles": profiles, "targets": targets,
        }))
    }

    fn preview(&self, request: &Request, export: bool) -> Result<Value> {
        // Reload at export, so external edits cannot silently invalidate a
        // download. Pixel verification also catches changes after server start.
        let fresh;
        let project = if export {
            fresh = Project::load(&self.dir)?;
            verified_image(&fresh, &self.dir, &self.page_id)?;
            &fresh
        } else {
            &self.project
        };
        let page = project.page(&self.page_id)?;
        ensure!(
            request.target.page == self.page_id,
            "Target belongs to another page"
        );
        ensure!(
            request.image_sha256 == page.image_sha256,
            "Page fingerprint changed; restart review"
        );
        let validation = validate_target_registration(project, &self.dir, &request.target);
        if export {
            validation?;
            return Ok(serde_json::to_value(&request.target)?);
        }
        let mut copy = project.clone();
        copy.targets.retain(|t| t.id != request.target.id);
        copy.targets.push(request.target.clone());
        let context = if request.paragraph_context {
            context_layout::paragraph_context(&copy, &request.target)
        } else {
            contextual::visible_context(&copy, &request.target)
        };
        Ok(json!({
            "valid": validation.is_ok(),
            "validation_error": validation.err().map(|e| format!("{e:#}")),
            "context": context.as_ref().ok(),
            "context_error": context.err().map(|e| format!("{e:#}")),
        }))
    }
}

struct HttpRequest {
    method: String,
    path: String,
    host: String,
    origin: Option<String>,
    content_type: String,
    body: Vec<u8>,
}

fn read_request(stream: &mut impl Read) -> Result<HttpRequest> {
    const LIMIT: usize = 1024 * 1024;
    let mut bytes = Vec::new();
    let mut chunk = [0; 4096];
    let header_end = loop {
        let n = stream.read(&mut chunk)?;
        ensure!(n > 0, "Incomplete request");
        bytes.extend_from_slice(&chunk[..n]);
        if let Some(i) = bytes.windows(4).position(|s| s == b"\r\n\r\n") {
            ensure!(i < 16_384, "Request headers too large");
            break i + 4;
        }
        ensure!(bytes.len() < 16_384, "Request headers too large");
    };
    let header = std::str::from_utf8(&bytes[..header_end])?;
    let mut lines = header.split("\r\n");
    let first: Vec<_> = lines
        .next()
        .unwrap_or_default()
        .split_whitespace()
        .collect();
    ensure!(first.len() == 3, "Invalid HTTP request");
    let mut request = HttpRequest {
        method: first[0].to_owned(),
        path: first[1].to_owned(),
        host: String::new(),
        origin: None,
        content_type: String::new(),
        body: vec![],
    };
    let mut length = None;
    for line in lines.filter(|s| !s.is_empty()) {
        let (name, value) = line.split_once(':').context("Malformed HTTP header")?;
        let value = value.trim();
        match name.to_ascii_lowercase().as_str() {
            "host" => {
                ensure!(request.host.is_empty(), "Duplicate Host");
                request.host = value.into();
            }
            "origin" => {
                ensure!(request.origin.is_none(), "Duplicate Origin");
                request.origin = Some(value.into());
            }
            "content-type" => request.content_type = value.into(),
            "content-length" => {
                ensure!(length.is_none(), "Duplicate content length");
                length = Some(value.parse::<usize>()?);
            }
            "transfer-encoding" => anyhow::bail!("Transfer encoding is not supported"),
            _ => {}
        }
    }
    let length = length.unwrap_or(0);
    ensure!(length <= LIMIT, "Request body too large");
    while bytes.len() - header_end < length {
        let remaining = (length - (bytes.len() - header_end)).min(chunk.len());
        let n = stream.read(&mut chunk[..remaining])?;
        ensure!(n > 0, "Incomplete body");
        bytes.extend_from_slice(&chunk[..n]);
    }
    request.body = bytes[header_end..header_end + length].to_vec();
    Ok(request)
}

struct Response {
    status: &'static str,
    content_type: &'static str,
    body: Vec<u8>,
}
impl Response {
    fn json(value: Value) -> Result<Self> {
        Ok(Self {
            status: "200 OK",
            content_type: "application/json",
            body: serde_json::to_vec_pretty(&value)?,
        })
    }
    fn error(status: &'static str, error: impl std::fmt::Display) -> Self {
        Self {
            status,
            content_type: "application/json",
            body: serde_json::to_vec(&json!({"error": error.to_string()})).unwrap(),
        }
    }
    fn write(&self, stream: &mut TcpStream) -> Result<()> {
        write!(
            stream,
            "HTTP/1.1 {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nReferrer-Policy: no-referrer\r\nContent-Security-Policy: default-src 'self'; script-src 'self' 'unsafe-inline'; style-src 'self' 'unsafe-inline'; img-src 'self'; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'none'\r\n\r\n",
            self.status,
            self.content_type,
            self.body.len()
        )?;
        stream.write_all(&self.body)?;
        Ok(())
    }
}

fn response(session: &Session, request: &HttpRequest, host: &str, token: &str) -> Result<Response> {
    if request.host != host
        || request
            .origin
            .as_ref()
            .is_some_and(|o| o != &format!("http://{host}"))
    {
        return Ok(Response::error(
            "403 Forbidden",
            "Local review origin required",
        ));
    }
    let prefix = format!("/{token}/");
    let Some(path) = request.path.strip_prefix(&prefix) else {
        return Ok(Response::error("404 Not Found", "Unknown review session"));
    };
    match (request.method.as_str(), path) {
        ("GET", "") => Ok(Response {
            status: "200 OK",
            content_type: "text/html; charset=utf-8",
            body: include_bytes!("review.html").to_vec(),
        }),
        ("GET", "data") => Response::json(session.data()?),
        ("GET", "page.bmp") => Ok(Response {
            status: "200 OK",
            content_type: "image/bmp",
            body: session.bmp.clone(),
        }),
        ("POST", "preview" | "export") => {
            ensure!(
                request.content_type.split(';').next() == Some("application/json"),
                "Expected application/json"
            );
            let parsed: Request = serde_json::from_slice(&request.body)?;
            Response::json(session.preview(&parsed, path == "export")?)
        }
        _ => Ok(Response::error("404 Not Found", "Unknown review resource")),
    }
}

/// Serve one page at an unguessable URL on IPv4 loopback. No files are written.
pub fn serve(dir: &Path, page_id: Option<&str>, port: u16) -> Result<()> {
    let session = Session::open(dir, page_id)?;
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port))?;
    let host = listener.local_addr()?.to_string();
    let mut entropy = [0u8; 24];
    fs::File::open("/dev/urandom")?.read_exact(&mut entropy)?;
    let token = hash(&entropy);
    println!("Local review: http://{host}/{token}/");
    println!(
        "Page {}. Downloads are target specifications; register them with `unredact target`. Ctrl-C stops review.",
        session.page_id
    );
    std::io::stdout().flush()?;
    for stream in listener.incoming() {
        let mut stream = stream?;
        stream.set_read_timeout(Some(Duration::from_secs(3)))?;
        stream.set_write_timeout(Some(Duration::from_secs(3)))?;
        let result = read_request(&mut stream).and_then(|r| response(&session, &r, &host, &token));
        let response =
            result.unwrap_or_else(|e| Response::error("400 Bad Request", format!("{e:#}")));
        // A browser may cancel an obsolete context request while dragging.
        let _ = response.write(&mut stream);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        calibration::{Fit, Profile, WidthMetric, Word},
        native::MeasurementMode,
        project::{Evidence, Page},
    };

    fn bounds(x: f64, y: f64, w: f64) -> Rect {
        Rect { x, y, w, h: 10.0 }
    }

    fn fixture() -> (tempfile::TempDir, Project, Target) {
        let dir = tempfile::tempdir().unwrap();
        let mut image = GrayImage::white(240, 120).unwrap();
        image.pixels[0] = 0;
        image.pixels[241] = 123;
        image.save(&dir.path().join("page.pgm")).unwrap();
        let words = [
            ("Training", bounds(10., 10., 40.)),
            ("Visible", bounds(50., 50., 40.)),
            ("context", bounds(150., 50., 40.)),
        ]
        .into_iter()
        .map(|(text, bounds)| Word {
            text: text.into(),
            bounds,
            confidence: 99.,
            block: "body".into(),
            line: "line".into(),
        })
        .collect();
        let project = Project {
            pages: vec![Page {
                id: "p<script>".into(),
                source: "source.pdf".into(),
                source_sha256: "source".into(),
                page_number: 1,
                dpi: 200.,
                width: image.width,
                height: image.height,
                image: "page.pgm".into(),
                image_sha256: hash(&image.pixels),
                likely_scan: true,
                image_coverage: 1.,
                native_chars: vec![],
                words,
                proposed_redactions: vec![bounds(100., 50., 40.)],
            }],
            profiles: vec![Profile {
                id: "body".into(),
                page: "p<script>".into(),
                block: None,
                fits: vec![Fit {
                    font: "unused.ttf".into(),
                    font_sha256: None,
                    kern: true,
                    size: 12.,
                    tracking: 0.,
                    ink_bias: 0.,
                    corrections: Default::default(),
                    validation_rmse: 1.,
                    validation_max_error: 2.,
                    training_words: 20,
                    validation_words: 10,
                    width_metric: WidthMetric::Ink,
                    measurement_mode: MeasurementMode::Design,
                    audit: None,
                    spacing: None,
                }],
                training_texts: vec!["Training".into()],
                validation_texts: vec![],
                audit_texts: vec![],
                font_search: None,
                spacing: None,
                typography_group: None,
                neural_search: None,
            }],
            ..Project::default()
        };
        project.save(dir.path()).unwrap();
        let target = Target {
            id: "draft".into(),
            page: "p<script>".into(),
            bounds: bounds(100., 50., 40.),
            profile: "body".into(),
            group: "group".into(),
            surface_kind: "canonical".into(),
            evidence: Evidence::Opaque {
                padding_min: 0.,
                padding_max: 2.,
                measurement_sigma: 1.,
            },
            left_context: String::new(),
            right_context: String::new(),
            dependence_key: None,
        };
        (dir, project, target)
    }

    fn request(project: &Project, target: Target) -> Request {
        Request {
            target,
            image_sha256: project.pages[0].image_sha256.clone(),
            paragraph_context: false,
        }
    }

    #[test]
    fn previews_actual_rust_context_and_exports_only_registration_valid_targets() {
        let (dir, project, target) = fixture();
        let before = fs::read(dir.path().join("project.json")).unwrap();
        let session = Session::open(dir.path(), None).unwrap();
        let mut req = request(&project, target);
        let preview = session.preview(&req, false).unwrap();
        assert_eq!(preview["valid"], true);
        assert_eq!(preview["context"]["left"], "Visible");
        assert_eq!(preview["context"]["right"], "context");
        let exported: Target =
            serde_json::from_value(session.preview(&req, true).unwrap()).unwrap();
        validate_target_registration(&project, dir.path(), &exported).unwrap();
        req.target.bounds = bounds(10., 10., 40.);
        assert_eq!(session.preview(&req, false).unwrap()["valid"], false);
        assert!(
            session
                .preview(&req, true)
                .unwrap_err()
                .to_string()
                .contains("calibration/validation")
        );
        req.target.bounds = bounds(230., 50., 40.);
        assert!(
            session
                .preview(&req, true)
                .unwrap_err()
                .to_string()
                .contains("outside page")
        );
        assert_eq!(fs::read(dir.path().join("project.json")).unwrap(), before);
        let data = session.data().unwrap();
        assert_eq!(data["profiles"][0]["exclusions"][0]["kind"], "training");
    }

    #[test]
    fn export_rechecks_current_project_and_page_evidence() {
        let (dir, mut project, target) = fixture();
        let session = Session::open(dir.path(), None).unwrap();
        let mut req = request(&project, target);
        req.image_sha256 = "wrong".into();
        assert!(
            session
                .preview(&req, true)
                .unwrap_err()
                .to_string()
                .contains("fingerprint")
        );
        req.image_sha256 = project.pages[0].image_sha256.clone();
        project.profiles[0].training_texts.push("Visible".into());
        project.pages[0].words[1].bounds = req.target.bounds;
        project.save(dir.path()).unwrap();
        assert!(
            session
                .preview(&req, true)
                .unwrap_err()
                .to_string()
                .contains("calibration/validation")
        );
        let image = GrayImage::white(240, 120).unwrap();
        image.save(&dir.path().join("page.pgm")).unwrap();
        assert!(
            session
                .preview(&req, true)
                .unwrap_err()
                .to_string()
                .contains("fingerprint")
        );
        assert!(Session::open(dir.path(), None).is_err());
        GrayImage::white(120, 240)
            .unwrap()
            .save(&dir.path().join("page.pgm"))
            .unwrap();
        assert!(
            Session::open(dir.path(), None)
                .err()
                .unwrap()
                .to_string()
                .contains("dimensions")
        );
    }

    #[test]
    fn bitmap_preserves_gray_pixels_with_row_padding() {
        let mut image = GrayImage::white(3, 2).unwrap();
        image.pixels = vec![0, 1, 2, 200, 254, 255];
        let bmp = bitmap(&image);
        assert_eq!(&bmp[..2], b"BM");
        let offset = u32::from_le_bytes(bmp[10..14].try_into().unwrap()) as usize;
        assert_eq!(&bmp[offset..], &[200, 254, 255, 0, 0, 1, 2, 0]);
        assert_eq!(&bmp[54 + 123 * 4..54 + 124 * 4], &[123, 123, 123, 0]);
    }

    #[test]
    fn local_session_requires_host_token_and_origin_and_keeps_document_data_out_of_html() {
        let (dir, _, _) = fixture();
        let session = Session::open(dir.path(), None).unwrap();
        let host = "127.0.0.1:1234";
        let mut req = HttpRequest {
            method: "GET".into(),
            path: "/token/".into(),
            host: host.into(),
            origin: None,
            content_type: String::new(),
            body: vec![],
        };
        let page = response(&session, &req, host, "token").unwrap();
        assert_eq!(page.status, "200 OK");
        assert!(!String::from_utf8(page.body).unwrap().contains("p<script>"));
        req.origin = Some("https://other.example".into());
        assert_eq!(
            response(&session, &req, host, "token").unwrap().status,
            "403 Forbidden"
        );
        req.origin = None;
        req.host = "other.example:1234".into();
        assert_eq!(
            response(&session, &req, host, "token").unwrap().status,
            "403 Forbidden"
        );
        req.host = host.into();
        req.path = "/wrong/data".into();
        assert_eq!(
            response(&session, &req, host, "token").unwrap().status,
            "404 Not Found"
        );
        req.path = "/token/export".into();
        req.method = "POST".into();
        req.content_type = "text/plain".into();
        assert!(response(&session, &req, host, "token").is_err());
    }

    #[test]
    fn parser_rejects_unbounded_or_ambiguous_requests() {
        let mut valid = std::io::Cursor::new(b"POST /token/preview HTTP/1.1\r\nHost: 127.0.0.1:1234\r\nContent-Type: application/json\r\nContent-Length: 2\r\n\r\n{}");
        assert_eq!(read_request(&mut valid).unwrap().body, b"{}");
        for header in [
            "Content-Length: 1048577\r\n",
            "Content-Length: 2\r\nContent-Length: 2\r\n",
            "Transfer-Encoding: chunked\r\n",
            "Host: extra\r\n",
        ] {
            let mut input = std::io::Cursor::new(format!(
                "POST / HTTP/1.1\r\nHost: localhost\r\n{header}\r\n{{}}"
            ));
            assert!(read_request(&mut input).is_err());
        }
    }
}
