use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tauri::Manager;
use std::{
    collections::HashSet,
    fs,
    io::{BufRead, BufReader, Read},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex},
};
use tauri::{Emitter, State, WebviewWindow};
use tempfile::TempDir;

const VIDEO_EXTENSIONS: &[&str] = &[
    "mp4", "mkv", "mov", "avi", "webm", "m4v", "ts", "mts", "m2ts",
];
const SUBTITLE_EXTENSIONS: &[&str] = &["ass", "ssa", "srt", "vtt", "sub", "smi"];
const MAX_TOOL_OUTPUT_BYTES: usize = 8 * 1024 * 1024;
const MAX_PREVIEW_FRAME_BYTES: usize = 32 * 1024 * 1024;
const MAX_SUBTITLE_BYTES: u64 = 16 * 1024 * 1024;
const MAX_FONT_FILES: usize = 32;
const MAX_FONT_FILE_BYTES: u64 = 16 * 1024 * 1024;
const MAX_FONT_TOTAL_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Default)]
pub struct AppState {
    export: Arc<Mutex<ExportJob>>,
    media_tools: Mutex<MediaTools>,
    extracted_subtitles: Mutex<Vec<(String, u64, tempfile::NamedTempFile)>>,
}

const EXPORT_CANCELLED: &str = "Export cancelled.";

#[derive(Default)]
struct ExportJob {
    running: bool,
    cancelled: bool,
    child: Option<Child>,
}

impl ExportJob {
    fn check_cancelled(&self) -> Result<(), String> {
        if self.cancelled {
            return Err(EXPORT_CANCELLED.into());
        }
        Ok(())
    }
}

struct JobGuard(Arc<Mutex<ExportJob>>);

impl Drop for JobGuard {
    fn drop(&mut self) {
        let mut job = self.0.lock().unwrap();
        if let Some(mut child) = job.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        job.running = false;
    }
}

fn reserve_job(state: &AppState) -> Result<JobGuard, String> {
    let mut job = state.export.lock().unwrap();
    if job.running {
        return Err("An export is already running.".into());
    }
    job.running = true;
    job.cancelled = false;
    Ok(JobGuard(state.export.clone()))
}

struct StagedOutput {
    _dir: TempDir,
    path: PathBuf,
}

impl StagedOutput {
    fn new(destination: &str, source: &str) -> Result<Self, String> {
        let destination = Path::new(destination);
        if let Ok(existing) = destination.canonicalize() {
            if Path::new(source).canonicalize().ok().as_ref() == Some(&existing) {
                return Err("Choose an output path different from the source video.".into());
            }
        }
        let name = destination.file_name().ok_or("Invalid output filename.")?;
        let parent = destination
            .parent()
            .filter(|path| !path.as_os_str().is_empty());
        let dir = tempfile::Builder::new()
            .prefix(".subaroll-")
            .tempdir_in(parent.unwrap_or_else(|| Path::new(".")))
            .map_err(|error| format!("Could not prepare the output folder: {error}"))?;
        let path = dir.path().join(name);
        Ok(Self { _dir: dir, path })
    }

    fn commit(&self, destination: &str, active: &Mutex<ExportJob>) -> Result<(), String> {
        // Serialize publishing with cancellation, so an existing export is safe until commit.
        let job = active.lock().unwrap();
        job.check_cancelled()?;
        fs::rename(&self.path, destination)
            .map_err(|error| format!("Could not save the finished export: {error}"))
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolStatus {
    ok: bool,
    ffmpeg: Option<String>,
    video_encoder: Option<String>,
    auto_video_encoder_10bit: Option<String>,
    apple_burn_available: bool,
    platform: String,
    available_video_encoders: Vec<String>,
    available_av1_encoders: Vec<String>,
    error: Option<String>,
}

#[derive(Default)]
struct MediaTools {
    custom_ffmpeg: Option<String>,
    cached: Option<Tools>,
}

#[derive(Clone)]
struct Tools {
    ffmpeg: String,
    ffprobe: String,
    video_encoder: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioStream {
    codec: String,
    language: String,
    channels: u64,
    bit_rate: u64,
    profile: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaMetadata {
    width: u64,
    height: u64,
    duration: f64,
    codec: String,
    bit_rate: u64,
    fps: String,
    #[serde(default)]
    time_base: String,
    frame_count: Option<u64>,
    sample_aspect_ratio: String,
    display_aspect_ratio: String,
    display_aspect: f64,
    rotation: i64,
    pixel_format: String,
    bit_depth: u64,
    color_range: String,
    color_space: String,
    color_transfer: String,
    color_primaries: String,
    is_hdr: bool,
    dolby_vision: bool,
    audio_streams: Vec<AudioStream>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubtitleSettings {
    delay_seconds: f64,
    position_percent: f64,
    scale_percent: f64,
    quality: f64,
    #[serde(default = "default_windows_encoder")]
    windows_encoder: String,
    #[serde(default = "default_windows_preset")]
    windows_preset: String,
}

fn default_windows_encoder() -> String {
    "auto".into()
}
fn default_windows_preset() -> String {
    "quality".into()
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewInput {
    video_path: String,
    subtitle_path: Option<String>,
    metadata: Option<MediaMetadata>,
    settings: SubtitleSettings,
    time: f64,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportInput {
    video_path: String,
    subtitle_path: String,
    output_path: String,
    metadata: Option<MediaMetadata>,
    settings: SubtitleSettings,
    clip_start: f64,
    clip_duration: f64,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompressionInput {
    video_path: String,
    output_path: String,
    metadata: Option<MediaMetadata>,
    target_size_mb: f64,
    #[serde(default = "default_compression_codec")]
    codec_preference: String,
    /// "auto", "nvenc", "qsv", "amf" or "software" (the CPU, x264).
    #[serde(default = "default_windows_encoder")]
    encoder_preference: String,
    #[serde(default)]
    clip_start: f64,
    #[serde(default)]
    clip_duration: f64,
}

fn default_compression_codec() -> String {
    "auto".into()
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewResult {
    frame: String,
    font_count: usize,
    subtitle_error: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportResult {
    output: String,
    font_count: usize,
    encoder: String,
    quality: i64,
    quality_setting: String,
    hdr: bool,
    ten_bit: bool,
    dolby_vision_converted: bool,
    clip_start: f64,
    clip_duration: f64,
    audio_copied: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompressionResult {
    output: String,
    bytes: u64,
    plan: CompressionPlan,
    metadata: MediaMetadata,
    encoder: String,
    hardware: bool,
    nvenc_qp: Option<u64>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompressionPlan {
    target_bytes: u64,
    total_bitrate: u64,
    video_bitrate: u64,
    audio_bitrate: u64,
    output_height: u64,
}

fn background_command(command: &str) -> Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;

        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let mut child = Command::new(command);
        child.creation_flags(CREATE_NO_WINDOW);
        child
    }
    #[cfg(not(windows))]
    Command::new(command)
}

fn command_ok(command: &str, args: &[&str]) -> bool {
    background_command(command)
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn command_text(command: &str, args: &[&str]) -> String {
    background_command(command)
        .args(args)
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default()
}

fn supports_libass(ffmpeg: &str) -> bool {
    command_text(ffmpeg, &["-hide_banner", "-filters"]).contains("subtitles         V->V")
}
fn supports_encoder(ffmpeg: &str, encoder: &str) -> bool {
    command_text(ffmpeg, &["-hide_banner", "-encoders"]).contains(encoder)
}

fn encoder_usable(ffmpeg: &str, encoder: &str, ten_bit: bool) -> bool {
    if !supports_encoder(ffmpeg, encoder) {
        return false;
    }
    background_command(ffmpeg)
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            // NVENC can reject a 64x64 input even when the encoder is usable.
            "color=c=black:s=256x256:r=30",
            "-frames:v",
            "1",
            "-c:v",
            encoder,
            "-pix_fmt",
            if ten_bit { "p010le" } else { "yuv420p" },
            "-f",
            "null",
            "-",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn windows_hardware_encoder(ffmpeg: &str, ten_bit: bool, requested: &str) -> Option<&'static str> {
    let candidates = if ten_bit {
        ["hevc_nvenc", "hevc_amf", "hevc_qsv"]
    } else {
        ["h264_nvenc", "h264_amf", "h264_qsv"]
    };
    candidates
        .into_iter()
        .filter(|encoder| requested == "auto" || encoder.ends_with(&format!("_{requested}")))
        .find(|encoder| encoder_usable(ffmpeg, encoder, ten_bit))
}

fn windows_burn_encoder(ffmpeg: &str, ten_bit: bool, requested: &str) -> &'static str {
    windows_hardware_encoder(ffmpeg, ten_bit, requested).unwrap_or_else(|| {
        if ten_bit && supports_encoder(ffmpeg, "libx265") {
            "libx265"
        } else {
            "libx264"
        }
    })
}

fn available_windows_encoders(ffmpeg: &str) -> Vec<String> {
    [
        ("nvenc", "h264_nvenc"),
        ("amf", "h264_amf"),
        ("qsv", "h264_qsv"),
    ]
    .into_iter()
    .filter(|(_, encoder)| encoder_usable(ffmpeg, encoder, false))
    .map(|(family, _)| family.to_owned())
    .collect()
}

fn available_windows_av1_encoders(ffmpeg: &str) -> Vec<String> {
    [
        ("nvenc", "av1_nvenc"),
        ("qsv", "av1_qsv"),
        ("amf", "av1_amf"),
    ]
    .into_iter()
    .filter(|(_, encoder)| {
        supports_encoder(ffmpeg, encoder)
            && compression_recipe_usable(
                ffmpeg,
                &compression_hardware_recipe(encoder, 4_000_000),
                1920,
                1080,
            )
    })
    .map(|(family, _)| family.to_owned())
    .collect()
}

fn find_tools(custom: Option<String>) -> Option<Tools> {
    let mut candidates = Vec::new();
    if let Some(value) = custom {
        if !value.is_empty() {
            candidates.push(value);
        }
    }
    if let Ok(value) =
        std::env::var("SUBAROLL_FFMPEG").or_else(|_| std::env::var("SUBTITLE_BURNER_FFMPEG"))
    {
        candidates.push(value);
    }
    #[cfg(target_os = "macos")]
    if let Ok(executable) = std::env::current_exe() {
        if let Some(directory) = executable.parent() {
            candidates.push(directory.join("ffmpeg").to_string_lossy().into_owned());
        }
    }
    #[cfg(target_os = "macos")]
    candidates.extend(
        [
            "/opt/homebrew/opt/ffmpeg-full/bin/ffmpeg",
            "/usr/local/opt/ffmpeg-full/bin/ffmpeg",
            "/opt/homebrew/bin/ffmpeg",
            "/usr/local/bin/ffmpeg",
        ]
        .map(str::to_owned),
    );
    #[cfg(target_os = "windows")]
    {
        if let Ok(executable) = std::env::current_exe() {
            if let Some(directory) = executable.parent() {
                candidates.push(directory.join("ffmpeg.exe").to_string_lossy().into_owned());
            }
        }
        candidates.push("ffmpeg.exe".into());
    }
    #[cfg(not(target_os = "windows"))]
    candidates.push("ffmpeg".into());
    let mut seen = HashSet::new();
    for ffmpeg in candidates.into_iter().filter(|p| seen.insert(p.clone())) {
        if !command_ok(&ffmpeg, &["-version"]) || !supports_libass(&ffmpeg) {
            continue;
        }
        let sibling = Path::new(&ffmpeg).with_file_name(if cfg!(windows) {
            "ffprobe.exe"
        } else {
            "ffprobe"
        });
        let ffprobe = if command_ok(&sibling.to_string_lossy(), &["-version"]) {
            sibling.to_string_lossy().into_owned()
        } else if command_ok("ffprobe", &["-version"]) {
            "ffprobe".into()
        } else {
            continue;
        };
        let video_encoder =
            if cfg!(target_os = "macos") && supports_encoder(&ffmpeg, "hevc_videotoolbox") {
                "hevc_videotoolbox"
            } else if cfg!(target_os = "windows") {
                windows_burn_encoder(&ffmpeg, false, "auto")
            } else {
                "libx264"
            }
            .into();
        return Some(Tools {
            ffmpeg,
            ffprobe,
            video_encoder,
        });
    }
    None
}

fn tools_for(state: &AppState) -> Result<Tools, String> {
    let mut media_tools = state
        .media_tools
        .lock()
        .map_err(|_| "Tool state is unavailable")?;
    if let Some(tools) = &media_tools.cached {
        return Ok(tools.clone());
    }
    let tools = find_tools(media_tools.custom_ffmpeg.clone())
        .ok_or_else(|| "FFmpeg with libass is required.".to_owned())?;
    media_tools.cached = Some(tools.clone());
    Ok(tools)
}

#[tauri::command]
pub fn tool_status(state: State<AppState>) -> ToolStatus {
    match tools_for(&state) {
        Ok(t) => {
            let available_video_encoders = if cfg!(target_os = "windows") {
                available_windows_encoders(&t.ffmpeg)
            } else if cfg!(target_os = "macos") {
                if encoder_usable(&t.ffmpeg, "h264_videotoolbox", false) {
                    vec!["videotoolbox".to_owned()]
                } else {
                    Vec::new()
                }
            } else {
                vec![t.video_encoder.clone()]
            };
            let available_av1_encoders = if cfg!(target_os = "windows") {
                available_windows_av1_encoders(&t.ffmpeg)
            } else {
                Vec::new()
            };
            ToolStatus {
                ok: true,
                apple_burn_available: cfg!(target_os = "macos")
                    && encoder_usable(&t.ffmpeg, "hevc_videotoolbox", false),
                auto_video_encoder_10bit: cfg!(windows)
                    .then(|| windows_burn_encoder(&t.ffmpeg, true, "auto").to_owned()),
                ffmpeg: Some(t.ffmpeg),
                video_encoder: Some(t.video_encoder),
                platform: std::env::consts::OS.into(),
                available_video_encoders,
                available_av1_encoders,
                error: None,
            }
        }
        Err(e) => ToolStatus {
            ok: false,
            ffmpeg: None,
            video_encoder: None,
            auto_video_encoder_10bit: None,
            apple_burn_available: false,
            platform: std::env::consts::OS.into(),
            available_video_encoders: Vec::new(),
            available_av1_encoders: Vec::new(),
            error: Some(e),
        },
    }
}

#[tauri::command]
pub fn choose_ffmpeg(state: State<AppState>) -> ToolStatus {
    let selected = rfd::FileDialog::new()
        .set_title("Choose FFmpeg with libass support")
        .pick_file();
    if let Some(path) = selected {
        *state.media_tools.lock().unwrap() = MediaTools {
            custom_ffmpeg: Some(path.to_string_lossy().into_owned()),
            cached: None,
        };
    }
    tool_status(state)
}

#[tauri::command]
pub fn choose_video() -> String {
    rfd::FileDialog::new()
        .set_title("Choose a video")
        .add_filter("Videos", VIDEO_EXTENSIONS)
        .pick_file()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default()
}

#[tauri::command]
pub fn choose_subtitle() -> String {
    rfd::FileDialog::new()
        .set_title("Choose subtitles")
        .add_filter("Subtitles", SUBTITLE_EXTENSIONS)
        .pick_file()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// (title, season, episode) from a file stem; season is None for "- 07" / "EP07" forms.
fn episode_key(stem: &str) -> Option<(String, Option<u32>, u32)> {
    let se = Regex::new(r"(?i)(?:^|[^a-z0-9])s(\d{1,2})[\s._-]*e(\d{1,3})(?:[^0-9]|$)").unwrap();
    let nxn = Regex::new(r"(?i)(?:^|[^a-z0-9])(\d{1,2})x(\d{2,3})(?:[^0-9]|$)").unwrap();
    let bare = Regex::new(r"(?i)(?:\s-\s*|[\s._]ep?(?:isode)?[\s._-]*)(\d{1,3})(?:v\d)?(?:[^0-9]|$)").unwrap();
    let (m, season, ep) = if let Some(c) = se.captures(stem).or_else(|| nxn.captures(stem)) {
        (c.get(0)?, c[1].parse().ok(), c[2].parse().ok()?)
    } else {
        let c = bare.captures(stem)?;
        (c.get(0)?, None, c[1].parse().ok()?)
    };
    let groups = Regex::new(r"\[[^\]]*\]|\([^)]*\)").unwrap();
    let title: String = groups
        .replace_all(&stem[..m.start()], "")
        .chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect();
    Some((title, season, ep))
}

/// Same-name subtitle beside the video (or in Subs/Subtitles), else one with the same
/// series + season + episode. Offline, read-only; a lone "episode" never matches an S/E video.
fn find_sidecars(video: &Path) -> Vec<PathBuf> {
    let Some(dir) = video.parent() else { return Vec::new() };
    let Some(video_stem) = video.file_stem() else { return Vec::new() };
    let stem = video_stem.to_string_lossy().to_lowercase();
    let want = episode_key(&stem);
    let mut matches: Vec<(u8, usize, String, PathBuf)> = Vec::new();
    for folder in [dir.to_path_buf(), dir.join("Subs"), dir.join("Subtitles")] {
        let Ok(entries) = fs::read_dir(&folder) else { continue };
        for entry in entries.flatten().take(5000) {
            if !entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
                continue;
            }
            let path = entry.path();
            let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
            let Some(ext_rank) = SUBTITLE_EXTENSIONS.iter().position(|e| *e == ext) else { continue };
            let Some(name) = path.file_stem() else { continue };
            let name = name.to_string_lossy().to_lowercase();
            // "ep.en" (language suffix) still counts as the same name
            let exact = name == stem || name.strip_prefix(&format!("{stem}.")).is_some();
            let matched = exact
                || match (&want, episode_key(&name)) {
                    (Some((vt, vs, ve)), Some((st, ss, se))) => {
                        ve == &se
                            && vs == &ss
                            && (vt.is_empty() || st.is_empty() || vt.contains(&st) || st.contains(vt))
                    }
                    _ => false,
                };
            if !matched {
                continue;
            }
            let key = (u8::from(!exact), ext_rank, name, path);
            matches.push(key);
        }
    }
    matches.sort();
    matches.into_iter().map(|entry| entry.3).collect()
}

fn find_sidecar(video: &Path) -> Option<PathBuf> {
    find_sidecars(video).into_iter().next()
}

#[tauri::command]
pub fn find_subtitle(video_path: String) -> String {
    find_sidecar(Path::new(&video_path))
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default()
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubtitleTrack {
    path: Option<String>,
    stream_index: Option<u64>,
    language: String,
    title: String,
    supported: bool,
    forced: bool,
    codec: String,
}

fn discover_subtitle_tracks(ffprobe: &str, video_path: &str) -> Result<Vec<SubtitleTrack>, String> {
    let bytes = run_output(ffprobe, &["-v", "error", "-print_format", "json", "-show_streams", video_path].map(str::to_owned))?;
    let data: Value = serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
    let mut tracks = Vec::new();
    for path in find_sidecars(Path::new(video_path)) {
        let title = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
        let forced = title.to_ascii_lowercase().split(|c: char| !c.is_ascii_alphanumeric()).any(|part| part == "forced");
        tracks.push(SubtitleTrack { path: Some(path.to_string_lossy().into_owned()), stream_index: None, language: String::new(), title, supported: true, forced, codec: String::new() });
    }
    for stream in data["streams"].as_array().into_iter().flatten().filter(|stream| stream["codec_type"] == "subtitle") {
        let Some(index) = stream["index"].as_u64() else { continue };
        let codec = stream["codec_name"].as_str().unwrap_or("");
        let supported = matches!(codec, "subrip" | "ass" | "ssa" | "mov_text" | "webvtt" | "text");
        tracks.push(SubtitleTrack {
            path: None, stream_index: Some(index),
            language: stream["tags"]["language"].as_str().unwrap_or("").to_owned(),
            title: stream["tags"]["title"].as_str().unwrap_or("").to_owned(),
            supported, forced: stream["disposition"]["forced"].as_i64() == Some(1), codec: codec.to_owned(),
        });
    }
    Ok(tracks)
}

#[tauri::command]
pub async fn discover_subtitles(state: State<'_, AppState>, video_path: String) -> Result<Vec<SubtitleTrack>, String> {
    let tools = tools_for(&state)?;
    tauri::async_runtime::spawn_blocking(move || discover_subtitle_tracks(&tools.ffprobe, &video_path))
        .await.map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn extract_subtitle(state: State<'_, AppState>, video_path: String, stream_index: u64) -> Result<String, String> {
    {
        let cache = state.extracted_subtitles.lock().map_err(|_| "Subtitle cache unavailable")?;
        if let Some((_, _, file)) = cache.iter().find(|(video, index, _)| video == &video_path && *index == stream_index) {
            return Ok(file.path().to_string_lossy().into_owned());
        }
        if cache.len() >= 64 { return Err("Too many extracted subtitle tracks in this session; restart the app to clear them.".into()); }
    }
    let tools = tools_for(&state)?;
    let cache_key = video_path.clone();
    let (path, file) = tauri::async_runtime::spawn_blocking(move || extract_subtitle_file(&tools, &video_path, stream_index))
        .await.map_err(|error| error.to_string())??;
    state.extracted_subtitles.lock().map_err(|_| "Subtitle cache unavailable")?.push((cache_key, stream_index, file));
    Ok(path)
}

fn extract_subtitle_file(tools: &Tools, video_path: &str, stream_index: u64) -> Result<(String, tempfile::NamedTempFile), String> {
        let track = discover_subtitle_tracks(&tools.ffprobe, &video_path)?.into_iter()
            .find(|track| track.stream_index == Some(stream_index) && track.supported)
            .ok_or("Subtitle track is unavailable or image-based.")?;
        let styled = matches!(track.codec.as_str(), "ass" | "ssa");
        let file = tempfile::Builder::new().prefix("subaroll-subtitle-").suffix(if styled { ".ass" } else { ".srt" }).tempfile().map_err(|error| error.to_string())?;
        let path = file.path().to_string_lossy().into_owned();
        run_output(&tools.ffmpeg, &["-v".to_owned(), "error".to_owned(), "-y".to_owned(), "-i".to_owned(), video_path.to_owned(),
            "-map".to_owned(), format!("0:{stream_index}"), "-c:s".to_owned(), if styled { "ass" } else { "srt" }.to_owned(),
            "-fs".to_owned(), (MAX_SUBTITLE_BYTES + 1).to_string(), path.clone()])?;
        if fs::metadata(&path).map_err(|error| error.to_string())?.len() > MAX_SUBTITLE_BYTES {
            return Err("Extracted subtitle exceeds the 16 MiB safety limit.".to_owned());
        }
        Ok((path, file))
}

#[tauri::command]
pub fn choose_output(suggested: String) -> String {
    let path = Path::new(&suggested);
    let mut dialog = rfd::FileDialog::new()
        .set_title("Save output video")
        .add_filter("Video", &["mp4", "mkv"]);
    if let Some(parent) = path.parent() {
        dialog = dialog.set_directory(parent);
    }
    if let Some(name) = path.file_name() {
        dialog = dialog.set_file_name(name.to_string_lossy());
    }
    dialog
        .save_file()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn run_output(command: &str, args: &[String]) -> Result<Vec<u8>, String> {
    run_output_with_limit(command, args, MAX_TOOL_OUTPUT_BYTES)
}

fn run_output_with_limit(
    command: &str,
    args: &[String],
    output_limit: usize,
) -> Result<Vec<u8>, String> {
    let mut child = background_command(command)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let out_thread = std::thread::spawn(move || read_capped(stdout, output_limit));
    let err_thread = std::thread::spawn(move || read_capped(stderr, MAX_TOOL_OUTPUT_BYTES));
    let status = child.wait();
    let (stdout, stdout_truncated) = out_thread.join().unwrap_or_default();
    let (stderr, stderr_truncated) = err_thread.join().unwrap_or_default();
    let status = status.map_err(|e| e.to_string())?;
    if stdout_truncated {
        return Err(format!(
            "Media tool output exceeded the {} MiB safety limit.",
            output_limit / (1024 * 1024)
        ));
    }
    if status.success() {
        Ok(stdout)
    } else {
        let mut message = last_lines(&String::from_utf8_lossy(&stderr));
        if stderr_truncated {
            message.push_str("\n(Media tool error output was truncated.)");
        }
        Err(message)
    }
}

fn read_text_capped(path: &Path, limit: u64, description: &str) -> Result<String, String> {
    let mut bytes = Vec::new();
    fs::File::open(path)
        .and_then(|file| file.take(limit + 1).read_to_end(&mut bytes))
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > limit {
        return Err(format!(
            "{description} exceeds the {} MiB safety limit.",
            limit / (1024 * 1024)
        ));
    }
    String::from_utf8(bytes).map_err(|e| e.to_string())
}

fn read_capped<R: Read>(mut reader: R, limit: usize) -> (Vec<u8>, bool) {
    let mut kept = Vec::with_capacity(limit.min(64 * 1024));
    let mut buffer = [0u8; 16 * 1024];
    let mut truncated = false;
    loop {
        let Ok(count) = reader.read(&mut buffer) else {
            break;
        };
        if count == 0 {
            break;
        }
        let remaining = limit.saturating_sub(kept.len());
        let keep = count.min(remaining);
        kept.extend_from_slice(&buffer[..keep]);
        truncated |= keep < count;
    }
    (kept, truncated)
}

fn last_lines(text: &str) -> String {
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .rev()
        .take(6)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join("\n")
}

fn ratio(value: &str, fallback: f64) -> f64 {
    let parts: Vec<_> = value.split([':', '/']).collect();
    if parts.len() == 2 {
        if let (Ok(a), Ok(b)) = (parts[0].parse::<f64>(), parts[1].parse::<f64>()) {
            if b != 0.0 {
                return a / b;
            }
        }
    }
    fallback
}

fn probe_media(ffprobe: &str, path: &str) -> Result<MediaMetadata, String> {
    let args = vec![
        "-v",
        "error",
        "-print_format",
        "json",
        "-show_format",
        "-show_streams",
        path,
    ]
    .into_iter()
    .map(str::to_owned)
    .collect::<Vec<_>>();
    let bytes = run_output(ffprobe, &args)?;
    let data: Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    let streams = data["streams"].as_array().cloned().unwrap_or_default();
    let video = streams
        .iter()
        .find(|s| s["codec_type"] == "video" && s["disposition"]["attached_pic"] != 1)
        .ok_or("The selected file does not contain a video stream.")?;
    let n = |v: &Value| {
        v.as_f64()
            .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
            .unwrap_or(0.0)
    };
    let s = |v: &Value| v.as_str().unwrap_or("").to_owned();
    let width = n(&video["width"]) as u64;
    let height = n(&video["height"]) as u64;
    let duration = {
        let d = n(&video["duration"]);
        if d > 0.0 {
            d
        } else {
            n(&data["format"]["duration"])
        }
    };
    let pixel_format = s(&video["pix_fmt"]);
    let mut bit_depth = n(&video["bits_per_raw_sample"]) as u64;
    if bit_depth == 0 {
        bit_depth = if pixel_format.contains("10") || pixel_format.contains("p010") {
            10
        } else if pixel_format.contains("12") {
            12
        } else {
            8
        };
    }
    let color_transfer = s(&video["color_transfer"]);
    let color_primaries = s(&video["color_primaries"]);
    let is_hdr = color_transfer.contains("smpte2084")
        || color_transfer.contains("arib-std-b67")
        || (bit_depth > 8 && color_primaries == "bt2020");
    let side = video["side_data_list"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let rotation = side
        .iter()
        .find_map(|x| x["rotation"].as_i64())
        .or_else(|| {
            video["tags"]["rotate"]
                .as_str()
                .and_then(|x| x.parse().ok())
        })
        .unwrap_or(0);
    let dolby_vision = side.iter().any(|x| {
        s(&x["side_data_type"]).to_lowercase().contains("dovi")
            || s(&x["side_data_type"])
                .to_lowercase()
                .contains("dolby vision")
    });
    let audio_streams = streams
        .iter()
        .filter(|x| x["codec_type"] == "audio")
        .map(|x| AudioStream {
            codec: s(&x["codec_name"]),
            language: s(&x["tags"]["language"]),
            channels: n(&x["channels"]) as u64,
            bit_rate: n(&x["bit_rate"]) as u64,
            profile: s(&x["profile"]),
        })
        .collect();
    let sample_aspect_ratio = {
        let v = s(&video["sample_aspect_ratio"]);
        if v.is_empty() {
            "1:1".into()
        } else {
            v
        }
    };
    let display_aspect_ratio = s(&video["display_aspect_ratio"]);
    let display_aspect = ratio(
        &display_aspect_ratio,
        width as f64 / height.max(1) as f64 * ratio(&sample_aspect_ratio, 1.0),
    );
    Ok(MediaMetadata {
        width,
        height,
        duration,
        codec: {
            let v = s(&video["codec_name"]);
            if v.is_empty() {
                "unknown".into()
            } else {
                v
            }
        },
        bit_rate: n(&video["bit_rate"]) as u64,
        fps: {
            let v = s(&video["avg_frame_rate"]);
            if ratio(&v, 0.0) > 0.0 {
                v
            } else {
                s(&video["r_frame_rate"])
            }
        },
        time_base: s(&video["time_base"]),
        frame_count: (n(&video["nb_frames"]) > 0.0).then(|| n(&video["nb_frames"]) as u64),
        sample_aspect_ratio,
        display_aspect_ratio,
        display_aspect,
        rotation,
        pixel_format,
        bit_depth,
        color_range: s(&video["color_range"]),
        color_space: s(&video["color_space"]),
        color_transfer,
        color_primaries,
        is_hdr,
        dolby_vision,
        audio_streams,
    })
}

#[tauri::command]
pub async fn probe(
    state: State<'_, AppState>,
    video_path: String,
) -> Result<MediaMetadata, String> {
    let tools = tools_for(&state)?;
    tauri::async_runtime::spawn_blocking(move || probe_media(&tools.ffprobe, &video_path))
        .await
        .map_err(|e| e.to_string())?
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubtitleTiming {
    cue_count: usize,
    last_cue_seconds: f64,
}

#[tauri::command]
pub fn inspect_subtitle(subtitle_path: String) -> Result<SubtitleTiming, String> {
    let text = read_text_capped(
        Path::new(&subtitle_path),
        MAX_SUBTITLE_BYTES,
        "Subtitle file",
    )?;
    let re = Regex::new(r"(\d{1,2}):(\d{2}):(\d{2})[,.](\d{2,3})").unwrap();
    let mut count = 0;
    let mut last: f64 = 0.0;
    for c in re.captures_iter(&text) {
        count += 1;
        let f = c[4].parse::<f64>().unwrap_or(0.0) / if c[4].len() == 2 { 100.0 } else { 1000.0 };
        last = last.max(
            c[1].parse::<f64>().unwrap_or(0.0) * 3600.0
                + c[2].parse::<f64>().unwrap_or(0.0) * 60.0
                + c[3].parse::<f64>().unwrap_or(0.0)
                + f,
        );
    }
    Ok(SubtitleTiming {
        cue_count: count / 2,
        last_cue_seconds: last,
    })
}

#[derive(Clone, Copy)]
struct Canvas {
    width: u64,
    height: u64,
    padded: bool,
}
fn canvas(m: &MediaMetadata) -> Canvas {
    if m.width == 0 || m.height == 0 || m.width as f64 / m.height as f64 <= 16.0 / 9.0 + 0.01 {
        Canvas {
            width: m.width,
            height: m.height,
            padded: false,
        }
    } else {
        let h = (((m.width as f64 * 9.0 / 16.0) / 2.0).ceil() * 2.0) as u64;
        Canvas {
            width: m.width,
            height: h,
            padded: true,
        }
    }
}
fn fmt(v: f64) -> String {
    let s = format!("{:.2}", v);
    s.trim_end_matches('0').trim_end_matches('.').to_owned()
}
fn escape_filter_path(path: &Path) -> String {
    path.to_string_lossy()
        .replace('\\', "/")
        .replace('\'', "\\'")
        .replace(':', "\\:")
        .replace(',', "\\,")
        .replace('[', "\\[")
        .replace(']', "\\]")
}

struct Prepared {
    _dir: TempDir,
    ass: PathBuf,
    fonts: PathBuf,
    font_count: usize,
}

fn split_row(row: &str, count: usize) -> Vec<String> {
    let mut parts = Vec::new();
    let mut rest = row;
    for _ in 0..count.saturating_sub(1) {
        if let Some(i) = rest.find(',') {
            parts.push(rest[..i].into());
            rest = &rest[i + 1..]
        } else {
            break;
        }
    }
    parts.push(rest.into());
    parts
}

fn explicit_position(text: &str) -> bool {
    text.split('}')
        .filter_map(|part| part.split_once('{'))
        .any(|(_, tags)| {
            let tags = tags.to_ascii_lowercase();
            tags.contains("\\pos(") || tags.contains("\\move(")
        })
}

fn transform_ass(source: &str, settings: &SubtitleSettings, plain: bool, aspect: f64) -> String {
    let scale = (settings.scale_percent / 100.0).clamp(0.25, 3.0);
    let position = settings.position_percent.clamp(0.0, 100.0);
    let mut text = source.trim_start_matches('\u{feff}').replace("\r\n", "\n");
    let play_y_re = Regex::new(r"(?im)^PlayResY:\s*([\d.]+)").unwrap();
    let play_x_re = Regex::new(r"(?im)^PlayResX:\s*([\d.]+)").unwrap();
    let mut play_y = play_y_re
        .captures(&text)
        .and_then(|c| c[1].parse().ok())
        .unwrap_or(288.0);
    let mut play_x = play_x_re
        .captures(&text)
        .and_then(|c| c[1].parse().ok())
        .unwrap_or(play_y * aspect);
    if plain {
        play_y = 360.0;
        play_x = (360.0 * aspect).round();
        if play_x_re.is_match(&text) {
            text = play_x_re
                .replace(&text, format!("PlayResX: {}", play_x as i64))
                .into_owned()
        } else {
            text = text.replacen(
                "[Script Info]",
                &format!("[Script Info]\nPlayResX: {}", play_x as i64),
                1,
            )
        }
        if play_y_re.is_match(&text) {
            text = play_y_re.replace(&text, "PlayResY: 360").into_owned()
        } else {
            text = text.replacen("[Script Info]", "[Script Info]\nPlayResY: 360", 1)
        }
    }
    let mut section = String::new();
    let mut event_fmt = Vec::<String>::new();
    let mut dialogue_styles = HashSet::new();
    for line in text.lines() {
        if line.starts_with('[') && line.ends_with(']') {
            section = line[1..line.len() - 1].to_lowercase()
        }
        if section == "events" && line.to_lowercase().starts_with("format:") {
            event_fmt = line[7..]
                .split(',')
                .map(|s| s.trim().to_lowercase())
                .collect()
        } else if section == "events"
            && line.to_lowercase().starts_with("dialogue:")
            && !event_fmt.is_empty()
        {
            let f = split_row(&line[9..], event_fmt.len());
            let si = event_fmt.iter().position(|x| x == "style").unwrap_or(0);
            let ti = event_fmt
                .iter()
                .position(|x| x == "text")
                .unwrap_or(f.len() - 1);
            if f.len() == event_fmt.len() && !explicit_position(&f[ti]) {
                dialogue_styles.insert(f[si].trim().to_owned());
            }
        }
    }
    let fs_re = Regex::new(r"(?i)\\fs([\d.]+)").unwrap();
    let an_re = Regex::new(r"(?i)\\an[1-9]").unwrap();
    section.clear();
    let mut style_fmt = Vec::<String>::new();
    let mut out = Vec::new();
    for line in text.lines() {
        let mut current = line.to_owned();
        if line.starts_with('[') && line.ends_with(']') {
            section = line[1..line.len() - 1].to_lowercase()
        }
        let styles = section == "v4+ styles" || section == "v4 styles";
        if styles && line.to_lowercase().starts_with("format:") {
            style_fmt = line[7..]
                .split(',')
                .map(|s| s.trim().to_lowercase())
                .collect()
        } else if styles && line.to_lowercase().starts_with("style:") && !style_fmt.is_empty() {
            let mut f = split_row(&line[6..], style_fmt.len());
            let ix = |name: &str| style_fmt.iter().position(|x| x == name);
            let name = ix("name")
                .and_then(|i| f.get(i))
                .map(|s| s.trim().to_owned())
                .unwrap_or_default();
            let adjust = plain || dialogue_styles.is_empty() || dialogue_styles.contains(&name);
            let set = |f: &mut Vec<String>, name: &str, value: String| {
                if let Some(i) = ix(name) {
                    f[i] = value
                }
            };
            if plain {
                set(&mut f, "fontname", "Trebuchet MS".into());
                set(&mut f, "fontsize", fmt(24.0 * scale));
                set(&mut f, "primarycolour", "&H00FFFFFF".into());
                set(&mut f, "outlinecolour", "&H00000000".into());
                set(&mut f, "backcolour", "&H00000000".into());
                set(&mut f, "bold", "0".into());
                set(&mut f, "borderstyle", "1".into());
                set(&mut f, "outline", "2".into());
                set(&mut f, "shadow", "1".into());
                set(&mut f, "alignment", "2".into());
                set(&mut f, "marginl", "10".into());
                set(&mut f, "marginr", "10".into());
                set(&mut f, "marginv", "18".into())
            } else if adjust {
                if let Some(i) = ix("fontsize") {
                    f[i] = fmt(f[i].parse::<f64>().unwrap_or(55.0) * scale)
                }
            }
            current = format!("Style:{}", f.join(","));
        } else if section == "events"
            && line.to_lowercase().starts_with("dialogue:")
            && !event_fmt.is_empty()
        {
            let mut f = split_row(&line[9..], event_fmt.len());
            let si = event_fmt.iter().position(|x| x == "style").unwrap_or(0);
            let ti = event_fmt
                .iter()
                .position(|x| x == "text")
                .unwrap_or(f.len() - 1);
            if f.len() == event_fmt.len() && !explicit_position(&f[ti])
                && (plain || dialogue_styles.contains(f[si].trim()))
            {
                let scaled = fs_re
                    .replace_all(&f[ti], |c: &regex::Captures| {
                        format!("\\fs{}", fmt(c[1].parse::<f64>().unwrap_or(0.0) * scale))
                    })
                    .into_owned();
                let lines = scaled.to_lowercase().matches("\\n").count() + 1;
                let top = (play_y * 0.3).min(4.0 + 30.0 * scale * lines as f64);
                let y = (top + (play_y - 4.0 - top) * position / 100.0).round();
                f[ti] = format!(
                    "{{\\an2\\pos({},{})}}{}",
                    (play_x / 2.0).round(),
                    y,
                    an_re.replace_all(&scaled, "")
                );
            }
            current = format!("Dialogue:{}", f.join(","));
        }
        out.push(current);
    }
    out.join("\n")
}

fn prepare_subtitle(
    ffmpeg: &str,
    subtitle: &str,
    metadata: &MediaMetadata,
    settings: &SubtitleSettings,
) -> Result<Prepared, String> {
    let dir = tempfile::Builder::new()
        .prefix("Subaroll-")
        .tempdir()
        .map_err(|e| e.to_string())?;
    let converted = dir.path().join("source.ass");
    let ass = dir.path().join("prepared.ass");
    let fonts = dir.path().join("fonts");
    fs::create_dir_all(&fonts).map_err(|e| e.to_string())?;
    let ext = Path::new(subtitle)
        .extension()
        .and_then(|x| x.to_str())
        .unwrap_or("")
        .to_lowercase();
    let styled = ext == "ass" || ext == "ssa";
    if fs::metadata(subtitle).map_err(|e| e.to_string())?.len() > MAX_SUBTITLE_BYTES {
        return Err("Subtitle file exceeds the 16 MiB safety limit.".into());
    }
    if ext == "ass" {
        fs::copy(subtitle, &converted).map_err(|e| e.to_string())?;
    } else {
        run_output(
            ffmpeg,
            &[
                "-hide_banner",
                "-loglevel",
                "error",
                "-y",
                "-i",
                subtitle,
                converted.to_string_lossy().as_ref(),
            ]
            .into_iter()
            .map(|x| x.to_owned())
            .collect::<Vec<_>>(),
        )?;
    }
    if fs::metadata(&converted).map_err(|e| e.to_string())?.len() > MAX_SUBTITLE_BYTES {
        return Err("Converted subtitle exceeds the 16 MiB safety limit.".into());
    }
    let source = read_text_capped(&converted, MAX_SUBTITLE_BYTES, "Converted subtitle")?;
    let c = canvas(metadata);
    let result = transform_ass(
        &source,
        settings,
        !styled,
        c.width as f64 / c.height.max(1) as f64,
    );
    fs::write(&ass, &result).map_err(|e| e.to_string())?;
    let mut count = 0;
    let mut total_font_bytes = 0u64;
    if let Some(parent) = Path::new(subtitle).parent() {
        if let Ok(entries) = fs::read_dir(parent) {
            for e in entries.flatten() {
                let p = e.path();
                if p.extension()
                    .and_then(|x| x.to_str())
                    .map(|x| matches!(x.to_lowercase().as_str(), "ttf" | "otf" | "ttc" | "otc"))
                    .unwrap_or(false)
                {
                    let Ok(file_type) = fs::symlink_metadata(&p).map(|m| m.file_type()) else {
                        continue;
                    };
                    if !file_type.is_file() {
                        continue;
                    }
                    if count >= MAX_FONT_FILES {
                        return Err("Subtitle folder contains more than 32 font files.".into());
                    }
                    let remaining = MAX_FONT_TOTAL_BYTES.saturating_sub(total_font_bytes);
                    if remaining == 0 {
                        return Err("Subtitle fonts exceed the 64 MiB total safety limit.".into());
                    }
                    let read_limit = MAX_FONT_FILE_BYTES.min(remaining);
                    let mut font = Vec::new();
                    match fs::File::open(&p)
                        .and_then(|file| file.take(read_limit + 1).read_to_end(&mut font))
                    {
                        Ok(_) if font.len() as u64 > read_limit => {
                            return Err("Subtitle fonts exceed the 64 MiB total or 16 MiB per-file safety limit.".into());
                        }
                        Ok(_) => {}
                        Err(_) => continue,
                    }
                    fs::write(fonts.join(p.file_name().unwrap()), &font)
                        .map_err(|e| e.to_string())?;
                    count += 1;
                    total_font_bytes += font.len() as u64;
                }
            }
        }
    }
    Ok(Prepared {
        _dir: dir,
        ass,
        fonts,
        font_count: count,
    })
}

fn subtitle_filter(p: &Prepared, delay: f64, timeline: f64) -> String {
    let subs = format!(
        "subtitles=filename='{}':fontsdir='{}'",
        escape_filter_path(&p.ass),
        escape_filter_path(&p.fonts),
    );
    let offset = timeline.max(0.0) - delay;
    if offset != 0.0 {
        format!(
            "settb=AVTB,setpts=PTS{sign}{n}/TB,{subs},setpts=PTS{back}{n}/TB",
            sign = if offset < 0.0 { "-" } else { "+" },
            back = if offset < 0.0 { "+" } else { "-" },
            n = format!("{:.6}", offset.abs())
        )
    } else {
        subs
    }
}

fn burn_filters(p: &Prepared, m: &MediaMetadata, delay: f64, timeline: f64) -> Vec<String> {
    let c = canvas(m);
    let mut filters = Vec::new();
    if c.padded {
        filters.push(format!("pad={}:{}:0:0:black", c.width, c.height));
    }
    let format = if m.is_hdr || m.bit_depth > 8 {
        "format=p010le"
    } else {
        "format=yuv420p"
    };
    filters.push(format.into());
    filters.push(subtitle_filter(p, delay, timeline));
    filters.push(format.into());
    filters
}

fn fps(v: &str) -> f64 {
    ratio(v, v.parse().unwrap_or(25.0)).max(0.001)
}
fn preview_time(m: &MediaMetadata, t: f64) -> f64 {
    t.max(0.0)
        .min((m.duration - (0.1f64).max(1.0 / fps(&m.fps))).max(0.0))
}
#[tauri::command]
pub async fn render_preview(
    state: State<'_, AppState>,
    input: PreviewInput,
) -> Result<PreviewResult, String> {
    let tools = tools_for(&state)?;
    tauri::async_runtime::spawn_blocking(move || preview_video(&tools, input))
        .await
        .map_err(|e| e.to_string())?
}

fn preview_video(tools: &Tools, input: PreviewInput) -> Result<PreviewResult, String> {
    let m = match input.metadata {
        Some(m) => m,
        None => probe_media(&tools.ffprobe, &input.video_path)?,
    };
    let time = preview_time(&m, input.time);
    let mut subtitle_error = None;
    if let Some(path) = input
        .subtitle_path
        .as_deref()
        .filter(|path| !path.is_empty())
    {
        let render = prepare_subtitle(&tools.ffmpeg, path, &m, &input.settings).and_then(|p| {
            let ff = burn_filters(&p, &m, input.settings.delay_seconds, time);
            decode_preview_frame(tools, &input.video_path, &m, time, ff)
                .map(|frame| (frame, p.font_count))
        });
        match render {
            Ok((frame, font_count)) => {
                return Ok(PreviewResult {
                    frame,
                    font_count,
                    subtitle_error: None,
                })
            }
            Err(error) => subtitle_error = Some(error),
        }
    }
    let frame = decode_preview_frame(tools, &input.video_path, &m, time, Vec::new())?;
    Ok(PreviewResult {
        frame,
        font_count: 0,
        subtitle_error,
    })
}

fn decode_preview_frame(
    tools: &Tools,
    video: &str,
    m: &MediaMetadata,
    time: f64,
    mut ff: Vec<String>,
) -> Result<String, String> {
    if m.is_hdr {
        ff.extend(
            [
                "zscale=t=linear:npl=100",
                "format=gbrpf32le",
                "zscale=p=bt709",
                "tonemap=hable:desat=0",
                "zscale=t=bt709:m=bt709:r=tv",
            ]
            .map(str::to_owned),
        )
    }
    // Composite at the source size. Only the display image is reduced.
    ff.push("scale=w='min(1280,iw)':h='min(720,ih)':force_original_aspect_ratio=decrease:flags=bilinear".into());
    ff.push("format=rgb24".into());
    let frame = run_output_with_limit(
        &tools.ffmpeg,
        &[
            "-hide_banner",
            "-loglevel",
            "error",
            "-ss",
            &fmt(time),
            "-noautorotate",
            "-i",
            video,
            "-map",
            "0:V:0",
            "-vf",
            &ff.join(","),
            "-frames:v",
            "1",
            "-c:v",
            "png",
            "-compression_level",
            "1",
            "-pix_fmt",
            "rgb24",
            "-f",
            "image2pipe",
            "pipe:1",
        ]
        .into_iter()
        .map(|x| x.to_owned())
        .collect::<Vec<_>>(),
        MAX_PREVIEW_FRAME_BYTES,
    )?;
    if frame.is_empty() {
        return Err("No video frame was decoded at this time.".into());
    }
    Ok(format!("data:image/png;base64,{}", BASE64.encode(frame)))
}

fn color_args(m: &MediaMetadata) -> Vec<String> {
    [
        ("-color_range", &m.color_range),
        ("-colorspace", &m.color_space),
        ("-color_trc", &m.color_transfer),
        ("-color_primaries", &m.color_primaries),
    ]
    .into_iter()
    .filter(|(_, v)| !v.is_empty() && v.as_str() != "unknown")
    .flat_map(|(k, v)| [k.to_owned(), v.clone()])
    .collect()
}

fn source_aware_ceiling_args(m: &MediaMetadata, quality: i64) -> Vec<String> {
    if m.bit_rate == 0 {
        return Vec::new();
    }
    let factor = 0.55 + ((quality.clamp(60, 100) - 60) as f64 / 40.0) * 0.60;
    let target = (m.bit_rate as f64 * factor).round() as u64;
    vec![
        "-maxrate".into(),
        (target as f64 * 1.35).round().to_string(),
        "-bufsize".into(),
        (target * 2).to_string(),
    ]
}

fn extend_quality_to_ten(quality: i64, legacy: i64, recommended: i64) -> i64 {
    if quality <= 90 { legacy } else {
        (recommended as f64 + (10 - recommended) as f64 * (quality - 90) as f64 / 10.0).round() as i64
    }
}

fn software_crf(base: i64, quality: i64) -> i64 {
    let legacy = (base as f64 - quality as f64 * 0.3).round() as i64;
    extend_quality_to_ten(quality, legacy, base - 27)
}

fn windows_encoder_args(
    hardware: &str,
    ten_bit: bool,
    quality: i64,
    preset: &str,
    m: &MediaMetadata,
) -> Vec<String> {
    // The legacy slider's 90 maps to high-quality values: NVENC CQP 18,
    // AMF HEVC 18/20/24, and QSV HEVC global_quality 18.
    let cq = (40.0 - quality as f64 * 0.244).round().clamp(14.0, 26.0) as i64;
    let amf_p = if ten_bit {
        extend_quality_to_ten(quality, cq + 2, 20)
    } else {
        extend_quality_to_ten(quality, cq, 18)
    };
    let pixel_format = if ten_bit { "p010le" } else { "yuv420p" };
    let nvenc_preset = match preset {
        "fast" => "p4",
        "balanced" => "p6",
        _ => "p7",
    };
    let amf_preset = match preset {
        "fast" => "speed",
        "balanced" => "balanced",
        _ => "quality",
    };
    let qsv_preset = match preset {
        "fast" => "medium",
        "balanced" => "slower",
        _ => "veryslow",
    };
    let mut args = if hardware.ends_with("_nvenc") {
        vec![
            "-c:v".into(),
            hardware.into(),
            "-preset".into(),
            nvenc_preset.into(),
            "-tune".into(),
            "hq".into(),
            "-rc".into(),
            "constqp".into(),
            "-qp".into(),
            extend_quality_to_ten(quality, cq, 18).to_string(),
            "-pix_fmt".into(),
            pixel_format.into(),
        ]
    } else if hardware.ends_with("_amf") {
        vec![
            "-c:v".into(),
            hardware.into(),
            "-quality".into(),
            amf_preset.into(),
            "-rc".into(),
            "cqp".into(),
            "-qp_i".into(),
            (amf_p - 2).max(10).to_string(),
            "-qp_p".into(),
            amf_p.to_string(),
            "-qp_b".into(),
            (amf_p + 4).to_string(),
            "-pix_fmt".into(),
            pixel_format.into(),
        ]
    } else {
        vec![
            "-c:v".into(),
            hardware.into(),
            "-preset".into(),
            qsv_preset.into(),
            "-global_quality".into(),
            if ten_bit {
                extend_quality_to_ten(quality, cq, 18).to_string()
            } else {
                extend_quality_to_ten(quality, (cq - 3).max(1), 15).to_string()
            },
            "-look_ahead".into(),
            "1".into(),
            "-pix_fmt".into(),
            pixel_format.into(),
        ]
    };
    if hardware.starts_with("hevc_") {
        args.extend([
            "-profile:v".into(),
            if ten_bit {
                "main10".into()
            } else {
                "main".into()
            },
            "-tag:v".into(),
            "hvc1".into(),
        ]);
    }
    if !hardware.ends_with("_nvenc") {
        args.extend(source_aware_ceiling_args(m, quality));
    }
    args.extend(color_args(m));
    args
}
fn encoder_args(
    ffmpeg: &str,
    m: &MediaMetadata,
    settings: &SubtitleSettings,
) -> (String, Vec<String>, bool) {
    let q = settings.quality;
    let quality = q.round().clamp(60.0, 100.0) as i64;
    let ten = m.is_hdr || m.bit_depth > 8;
    let mut a: Vec<String>;
    let windows_encoder = cfg!(target_os = "windows")
        .then(|| windows_burn_encoder(ffmpeg, ten, &settings.windows_encoder));
    if let Some(encoder) = windows_encoder.filter(|encoder| !encoder.starts_with("libx")) {
        a = windows_encoder_args(encoder, ten, quality, &settings.windows_preset, m);
        return (encoder.into(), a, ten);
    }
    if cfg!(target_os = "macos")
        && matches!(settings.windows_encoder.as_str(), "auto" | "videotoolbox")
        && encoder_usable(ffmpeg, "hevc_videotoolbox", ten)
    {
        // VideoToolbox q100 switches into a pathological near-lossless mode and
        // can exceed the source bitrate by more than 10x. q95 is the practical
        // maximum-quality setting; the source-aware VBV ceiling remains active.
        let hardware_quality = quality.min(95);
        a = vec![
            "-c:v",
            "hevc_videotoolbox",
            "-profile:v",
            if ten { "main10" } else { "main" },
            "-q:v",
            &hardware_quality.to_string(),
            "-allow_sw",
            "0",
            "-realtime",
            "0",
            "-prio_speed",
            "0",
            "-spatial_aq",
            "1",
            "-pix_fmt",
            if ten { "p010le" } else { "yuv420p" },
            "-tag:v",
            "hvc1",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect();
        a.extend(source_aware_ceiling_args(m, quality));
        a.extend(color_args(m));
        ("hevc_videotoolbox".into(), a, ten)
    } else if (m.width > 4096 || m.height > 4096) && supports_encoder(ffmpeg, "libx265") {
        let crf = software_crf(41, quality);
        let mut args = [
            "-c:v",
            "libx265",
            "-tag:v",
            "hvc1",
            "-preset",
            "fast",
            "-crf",
            &crf.to_string(),
            "-pix_fmt",
            if ten { "yuv420p10le" } else { "yuv420p" },
            "-x265-params",
            "pools=4:frame-threads=1:log-level=warning",
        ]
        .map(str::to_owned)
        .to_vec();
        args.extend(color_args(m));
        ("libx265".into(), args, ten)
    } else if let Some(encoder) = windows_encoder {
        if encoder == "libx265" {
            let crf = software_crf(47, quality);
            let cpu_preset = match settings.windows_preset.as_str() {
                "fast" => "fast",
                "balanced" => "medium",
                _ => "medium",
            };
            a = vec![
                "-c:v".into(),
                "libx265".into(),
                "-preset".into(),
                cpu_preset.into(),
                "-crf".into(),
                crf.to_string(),
                "-pix_fmt".into(),
                "yuv420p10le".into(),
            ];
            a.extend(color_args(m));
            return ("libx265".into(), a, true);
        }
        let crf = software_crf(43, quality);
        let cpu_preset = match settings.windows_preset.as_str() {
            "fast" => "fast",
            "balanced" => "medium",
            _ => "slow",
        };
        return (
            "libx264".into(),
            vec![
                "-c:v".into(),
                "libx264".into(),
                "-preset".into(),
                cpu_preset.into(),
                "-crf".into(),
                crf.to_string(),
                "-pix_fmt".into(),
                "yuv420p".into(),
            ],
            false,
        );
    } else if ten && supports_encoder(ffmpeg, "libx265") {
        let crf = software_crf(41, quality);
        a = vec![
            "-c:v",
            "libx265",
            "-preset",
            "medium",
            "-crf",
            &crf.to_string(),
            "-pix_fmt",
            "yuv420p10le",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect();
        a.extend(color_args(m));
        ("libx265".into(), a, true)
    } else {
        let crf = software_crf(41, quality);
        (
            "libx264".into(),
            vec![
                "-c:v",
                "libx264",
                "-preset",
                "medium",
                "-crf",
                &crf.to_string(),
                "-pix_fmt",
                "yuv420p",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect(),
            false,
        )
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QualityProfile {
    encoder: String,
    values: Vec<String>,
    hint: String,
}

fn arg_value<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    args.windows(2).find(|pair| pair[0] == flag).map(|pair| pair[1].as_str())
}

fn quality_setting_from_args(encoder: &str, args: &[String]) -> Option<String> {
    if encoder.ends_with("_videotoolbox") {
        Some(format!("Quality {}", arg_value(args, "-q:v")?))
    } else if encoder.starts_with("libx") {
        Some(format!("CRF {}", arg_value(args, "-crf")?))
    } else if encoder.ends_with("_nvenc") {
        Some(format!("CQP {}", arg_value(args, "-qp")?))
    } else if encoder.ends_with("_qsv") {
        Some(format!("ICQ {}", arg_value(args, "-global_quality")?))
    } else if encoder.ends_with("_amf") {
        Some(format!("CQP {}", arg_value(args, "-qp_p")?))
    } else { None }
}

fn quality_profile_for_args(encoder: &str, args: &[String], ten: bool, metadata: &MediaMetadata) -> Result<QualityProfile, String> {
    let cpu_base = if encoder.starts_with("libx") {
        Some(arg_value(args, "-crf").ok_or("Missing CRF")?.parse::<i64>().map_err(|_| "Invalid CRF")? + 27)
    } else { None };
    let values = (60..=100).map(|quality| {
        if encoder.ends_with("_videotoolbox") {
            format!("Quality {}", quality.min(95))
        } else if let Some(base) = cpu_base {
            format!("CRF {}", software_crf(base, quality))
        } else {
            let params = windows_encoder_args(encoder, ten, quality, "quality", metadata);
            quality_setting_from_args(encoder, &params).unwrap_or_default()
        }
    }).collect();
    let hint = if encoder.ends_with("_amf") {
        "CQP shows the P-frame QP; I- and B-frame QPs use offsets. Lower values mean higher quality. A source-aware bitrate ceiling may also apply."
    } else if encoder.ends_with("_nvenc") {
        "NVIDIA constant QP: lower values mean higher quality and generally larger files."
    } else if encoder.ends_with("_videotoolbox") {
        "VideoToolbox quality: higher values mean higher quality; the practical maximum is 95. A source-aware bitrate ceiling may also apply."
    } else if encoder.starts_with("libx") {
        "CRF: lower values mean higher quality."
    } else {
        "Lower values mean higher quality. Hardware encoding may also apply a source-aware bitrate ceiling."
    };
    Ok(QualityProfile { encoder: encoder.into(), values, hint: hint.into() })
}

#[tauri::command]
pub async fn burn_quality_profile(state: State<'_, AppState>, metadata: Option<MediaMetadata>, settings: SubtitleSettings) -> Result<QualityProfile, String> {
    let tools = tools_for(&state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let metadata = metadata.unwrap_or_default();
        let mut sample = settings;
        sample.quality = 90.0;
        let (encoder, args, ten) = encoder_args(&tools.ffmpeg, &metadata, &sample);
        quality_profile_for_args(&encoder, &args, ten, &metadata)
    }).await.map_err(|error| error.to_string())?
}

fn run_progress(
    command: &str,
    args: &[String],
    duration: f64,
    base: f64,
    span: f64,
    emit: &dyn Fn(f64),
    active: &Mutex<ExportJob>,
) -> Result<(), String> {
    let (child_id, stdout, stderr) = {
        let mut job = active.lock().unwrap();
        job.check_cancelled()?;
        let mut child = background_command(command)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| e.to_string())?;
        let pipes = (
            child.id(),
            child.stdout.take().unwrap(),
            child.stderr.take().unwrap(),
        );
        job.child = Some(child);
        pipes
    };
    #[cfg(not(target_os = "macos"))]
    let _ = child_id;
    #[cfg(target_os = "macos")]
    let awake = Command::new("/usr/bin/caffeinate")
        .args(["-i", "-w", &child_id.to_string()])
        .spawn()
        .ok();
    let err_thread = std::thread::spawn(move || read_capped(stderr, MAX_TOOL_OUTPUT_BYTES));
    for line in BufReader::new(stdout).lines().map_while(Result::ok) {
        if let Some(v) = line
            .strip_prefix("out_time_us=")
            .or_else(|| line.strip_prefix("out_time_ms="))
        {
            if let Ok(us) = v.parse::<f64>() {
                emit(base + (us / 1_000_000.0 / duration.max(0.1)).clamp(0.0, 1.0) * span);
            }
        }
    }

    // Release the lock between polls so Cancel still works if the tool closes stdout early.
    let status = loop {
        {
            let mut job = active.lock().unwrap();
            match job.child.as_mut().unwrap().try_wait() {
                Ok(Some(status)) => {
                    job.child.take();
                    break Ok(status);
                }
                Ok(None) => {}
                Err(error) => {
                    let mut child = job.child.take().unwrap();
                    let _ = child.kill();
                    let _ = child.wait();
                    break Err(error);
                }
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(15));
    };
    #[cfg(target_os = "macos")]
    if let Some(mut awake) = awake {
        let _ = awake.kill();
        let _ = awake.wait();
    }
    let (stderr, truncated) = err_thread.join().unwrap_or_default();
    active.lock().unwrap().check_cancelled()?;
    let status = status.map_err(|e| e.to_string())?;
    if status.success() {
        Ok(())
    } else {
        let mut message = last_lines(&String::from_utf8_lossy(&stderr));
        if truncated {
            message.push_str("\n(Media tool error output was truncated.)");
        }
        Err(message)
    }
}

#[tauri::command]
pub async fn start_export(
    window: WebviewWindow,
    state: State<'_, AppState>,
    input: ExportInput,
) -> Result<ExportResult, String> {
    let job_guard = reserve_job(&state)?;
    let tools = tools_for(&state)?;
    let active = state.export.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _job_guard = job_guard;
        let emit = |v: f64| {
            let _ = window.emit("export-progress", v);
        };
        burn_video(&tools, input, &active, &emit)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// A sample covering the source uses the same untrimmed path as a normal export.
fn export_clip(source_duration: f64, start: f64, length: f64) -> Option<(f64, f64)> {
    if !(length.is_finite() && length > 0.0 && length < source_duration) {
        return None;
    }
    Some((start.max(0.0).min(source_duration - length), length))
}

fn preserve_video_timing(args: &mut Vec<String>, source: &MediaMetadata) {
    args.extend(["-fps_mode".into(), "passthrough".into()]);
    if ratio(&source.time_base, 0.0) > 0.0 {
        args.extend(["-enc_time_base:v".into(), source.time_base.clone()]);
    }
}

fn validate_frame_count(source: &MediaMetadata, output: &MediaMetadata) -> Result<(), String> {
    if let (Some(expected), Some(actual)) = (source.frame_count, output.frame_count) {
        if expected != actual {
            return Err(format!(
                "Export frame count changed from {expected} to {actual}."
            ));
        }
    }
    Ok(())
}

fn burn_video(
    tools: &Tools,
    input: ExportInput,
    active: &Mutex<ExportJob>,
    emit: &dyn Fn(f64),
) -> Result<ExportResult, String> {
    let m = match input.metadata {
        Some(m) => m,
        None => probe_media(&tools.ffprobe, &input.video_path)?,
    };
    let staged = StagedOutput::new(&input.output_path, &input.video_path)?;
    let staged_path = staged.path.to_string_lossy();
    let c = canvas(&m);
    let p = prepare_subtitle(&tools.ffmpeg, &input.subtitle_path, &m, &input.settings)?;
    let clip = export_clip(m.duration, input.clip_start, input.clip_duration);
    let (start, duration) = clip.unwrap_or((0.0, 0.0));
    let filters = burn_filters(&p, &m, input.settings.delay_seconds, start);
    let (mut encoder, enc, mut ten_bit) = encoder_args(&tools.ffmpeg, &m, &input.settings);
    let mut used_args = enc.clone();
    let ext = Path::new(&input.output_path)
        .extension()
        .and_then(|x| x.to_str())
        .unwrap_or("")
        .to_lowercase();
    let copy = ext == "mkv"
        || m.audio_streams
            .iter()
            .all(|a| matches!(a.codec.as_str(), "aac" | "ac3" | "eac3" | "mp3" | "alac"));
    let mut args = vec![
        "-hide_banner".into(),
        "-loglevel".into(),
        "warning".into(),
        "-y".into(),
    ];
    if m.width > 4096 || m.height > 4096 {
        args.extend(["-filter_threads".into(), "2".into()]);
    }
    if start > 0.0 {
        args.extend(["-ss".into(), format!("{start:.6}")])
    }
    args.extend([
        "-noautorotate".into(),
        "-i".into(),
        input.video_path.clone(),
        "-map".into(),
        "0:V:0".into(),
        "-map".into(),
        "0:a?".into(),
        "-vf".into(),
        filters.join(","),
    ]);
    if copy {
        args.extend(["-c:a".into(), "copy".into()])
    } else {
        args.extend(["-c:a".into(), "aac".into(), "-b:a".into(), "256k".into()])
    }
    args.extend([
        "-map_metadata".into(),
        "0".into(),
        "-map_chapters".into(),
        "0".into(),
    ]);
    if duration > 0.0 {
        args.extend(["-t".into(), format!("{duration:.6}")])
    }
    preserve_video_timing(&mut args, &m);
    args.extend(["-progress".into(), "pipe:1".into(), "-nostats".into()]);
    let run_burn = |encoder_args: Vec<String>| {
        let mut command = args.clone();
        command.extend(encoder_args);
        command.push(staged_path.to_string());
        run_progress(
            &tools.ffmpeg,
            &command,
            clip.map_or(m.duration, |(_, length)| length),
            0.0,
            1.0,
            emit,
            active,
        )
    };
    if let Err(hardware_error) = run_burn(enc) {
        if encoder.starts_with("libx") || hardware_error == EXPORT_CANCELLED {
            return Err(hardware_error);
        }
        let software = SubtitleSettings {
            windows_encoder: "software".into(),
            ..input.settings.clone()
        };
        let (fallback, args, fallback_ten_bit) = encoder_args(&tools.ffmpeg, &m, &software);
        used_args = args.clone();
        run_burn(args).map_err(|error| {
            if error == EXPORT_CANCELLED {
                error
            } else {
                format!("Hardware encoding failed: {hardware_error}\nCPU encoding failed: {error}")
            }
        })?;
        encoder = fallback;
        ten_bit = fallback_ten_bit;
    }
    let out = probe_media(&tools.ffprobe, &staged_path)?;
    if out.width != c.width || out.height != c.height {
        return Err(format!(
            "Export safety check failed: expected {}×{}, but output is {}×{}.",
            c.width, c.height, out.width, out.height
        ));
    }
    if m.is_hdr && !out.is_hdr {
        return Err("Export safety check failed: HDR color signaling was not preserved.".into());
    }
    if m.bit_depth > 8 && out.bit_depth < 10 {
        return Err("Export safety check failed: the 10-bit source was reduced to 8-bit.".into());
    }
    if clip.is_none() {
        validate_frame_count(&m, &out)?;
    }
    let expected_aspect =
        c.width as f64 / c.height.max(1) as f64 * ratio(&m.sample_aspect_ratio, 1.0);
    if (out.display_aspect - expected_aspect).abs() > 0.001 {
        return Err("Export safety check failed: the output display aspect ratio changed.".into());
    }
    if out.rotation != m.rotation {
        return Err("Export safety check failed: the output orientation changed.".into());
    }
    staged.commit(&input.output_path, active)?;
    emit(1.0);
    let quality_setting = quality_setting_from_args(&encoder, &used_args).unwrap_or_default();
    Ok(ExportResult {
        output: input.output_path,
        font_count: p.font_count,
        encoder,
        quality: input.settings.quality.round() as i64,
        quality_setting,
        hdr: m.is_hdr,
        ten_bit,
        dolby_vision_converted: m.dolby_vision,
        clip_start: start,
        clip_duration: duration,
        audio_copied: copy,
    })
}

const MAX_COMPRESSION_MB: f64 = 750.0;
// Below this even 480p is unwatchable, so smaller targets are refused instead of encoded badly.
const MIN_VIDEO_BITRATE: u64 = 150_000;
const SIZE_BUDGET: f64 = 0.96;

fn audio_bitrate_for(m: &MediaMetadata, total: u64) -> u64 {
    if m.audio_streams.is_empty() {
        0
    } else if total >= 1_500_000 {
        128_000
    } else if total >= 700_000 {
        96_000
    } else {
        64_000
    }
}

/// Targets below this keep the plain size (small files are not worth squeezing for a margin).
const SAFE_MARGIN_FROM_MB: f64 = 5.0;
const GUARDED_DECIMAL_MB: f64 = 990_000.0;

/// The byte ceiling for a picked size. From 5 MB up, "MB" means 1,000,000 bytes (what upload limits
/// use) minus 1% for container overhead, so the file also fits services that count that way.
/// Below 5 MB it stays 1 MB = 1,048,576 bytes.
fn ceiling_bytes(mb: f64) -> u64 {
    if mb >= SAFE_MARGIN_FROM_MB {
        (mb * GUARDED_DECIMAL_MB) as u64
    } else {
        (mb * 1_048_576.0) as u64
    }
}

fn budget_bitrate(bytes: u64, duration: f64) -> u64 {
    (bytes as f64 * 8.0 / duration * SIZE_BUDGET) as u64
}

fn planned_video_bitrate(m: &MediaMetadata, mb: f64) -> u64 {
    let total = budget_bitrate(ceiling_bytes(mb), m.duration);
    total.saturating_sub(audio_bitrate_for(m, total))
}

/// Smallest whole-MB target that meets the same video bitrate floor as export.
fn min_target_mb(m: &MediaMetadata) -> f64 {
    let rate = MIN_VIDEO_BITRATE + audio_bitrate_for(m, MIN_VIDEO_BITRATE);
    let needed = rate as f64 * m.duration / 8.0 / SIZE_BUDGET;
    let small_target = (needed / 1_048_576.0).ceil().max(1.0);
    let target = if small_target < SAFE_MARGIN_FROM_MB {
        small_target
    } else {
        (needed / GUARDED_DECIMAL_MB)
            .ceil()
            .max(SAFE_MARGIN_FROM_MB)
    };
    // Byte rounding must never advertise a target that export will reject.
    if planned_video_bitrate(m, target) < MIN_VIDEO_BITRATE {
        target + 1.0
    } else {
        target
    }
}

fn check_compression_target(m: &MediaMetadata, mb: f64) -> Result<(), String> {
    if !mb.is_finite() || !(1.0..=MAX_COMPRESSION_MB).contains(&mb) {
        return Err(format!(
            "Choose a target size between 1 and {MAX_COMPRESSION_MB} MB."
        ));
    }
    if !m.duration.is_finite() || m.duration <= 0.0 {
        return Err("Couldn't read this video's length, so a target size can't be planned.".into());
    }
    if planned_video_bitrate(m, mb) < MIN_VIDEO_BITRATE {
        let minimum = min_target_mb(m);
        if minimum > MAX_COMPRESSION_MB {
            return Err(format!(
                "This video needs at least {minimum} MB at 150 kbps. The limit is {MAX_COMPRESSION_MB} MB."
            ));
        }
        return Err(format!(
            "Target too small. This video needs at least {minimum} MB in order to keep 150 kbps of video."
        ));
    }
    Ok(())
}

fn compression_plan(m: &MediaMetadata, mb: f64) -> CompressionPlan {
    compression_plan_bytes(m, ceiling_bytes(mb.clamp(1.0, MAX_COMPRESSION_MB)))
}

fn compression_plan_bytes(m: &MediaMetadata, target: u64) -> CompressionPlan {
    let total = budget_bitrate(target, m.duration);
    let audio = audio_bitrate_for(m, total);
    let mut video = total.saturating_sub(audio).max(MIN_VIDEO_BITRATE);
    let mut target = target;
    // Spending far more than the source already has only bloats the file; keep headroom for codec differences.
    if m.bit_rate > 0 && video as f64 > m.bit_rate as f64 * 1.5 {
        video = m.bit_rate.saturating_add(m.bit_rate / 2);
        // The size goal shrinks with it, otherwise constant-QP encoders would chase the unreachable target.
        target =
            target.min(((video + audio) as f64 * m.duration / 8.0 / SIZE_BUDGET) as u64);
    }
    let output_height = planned_compression_height(m, video, false);
    CompressionPlan {
        target_bytes: target,
        total_bitrate: total,
        video_bitrate: video,
        audio_bitrate: audio,
        output_height,
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompressionEstimate {
    video_bitrate: u64,
    output_height: u64,
    bitrate_capped: bool,
    min_mb: f64,
    uses_cpu: bool,
}

/// Live estimate for the UI. Errors carry the same message the export would fail with.
#[tauri::command]
pub fn plan_compression(
    metadata: MediaMetadata,
    target_size_mb: f64,
    av1: bool,
    encoder_preference: String,
) -> Result<CompressionEstimate, String> {
    check_compression_target(&metadata, target_size_mb)?;
    let plan = compression_plan(&metadata, target_size_mb);
    let uses_cpu = use_cpu_encoder(plan.video_bitrate, &encoder_preference);
    let height = if av1 && !uses_cpu {
        planned_compression_height(&metadata, plan.video_bitrate, true)
    } else {
        plan.output_height
    };
    Ok(CompressionEstimate {
        video_bitrate: plan.video_bitrate,
        output_height: height,
        bitrate_capped: plan.target_bytes < ceiling_bytes(target_size_mb),
        min_mb: min_target_mb(&metadata),
        uses_cpu,
    })
}

fn compression_height_for_bitrate(video_bitrate: u64, av1: bool) -> u64 {
    if av1 {
        if video_bitrate >= 5_000_000 {
            2160
        } else if video_bitrate >= 2_800_000 {
            1440
        } else if video_bitrate >= 1_200_000 {
            1080
        } else if video_bitrate >= 750_000 {
            900
        } else if video_bitrate >= 400_000 {
            720
        } else if video_bitrate >= 250_000 {
            540
        } else {
            480
        }
    // Two-pass x264 remains useful at 1080p around 2 Mbps. Keep source detail
    // until the actual file-size budget makes a lower resolution necessary.
    } else if video_bitrate >= 8_000_000 {
        2160
    } else if video_bitrate >= 4_500_000 {
        1440
    } else if video_bitrate >= 2_000_000 {
        1080
    } else if video_bitrate >= 1_200_000 {
        900
    } else if video_bitrate >= 650_000 {
        720
    } else if video_bitrate >= 350_000 {
        540
    } else {
        480
    }
}

fn planned_compression_height(m: &MediaMetadata, bitrate: u64, av1: bool) -> u64 {
    let height = m.height.min(compression_height(bitrate, av1, fps(&m.fps)));
    let limit = if m.width > 4096 {
        height.min(m.height.saturating_mul(4096) / m.width)
    } else {
        height
    };
    (limit & !1).max(2)
}

fn compression_height(video_bitrate: u64, av1: bool, source_fps: f64) -> u64 {
    let source_fps = source_fps.max(1.0);
    let rate_at_source_fps = (video_bitrate as f64 * (30.0 / source_fps.max(12.0))).round() as u64;
    let height_at_source_fps = compression_height_for_bitrate(rate_at_source_fps, av1);
    if !av1 && source_fps > 30.0 && height_at_source_fps >= 900 && rate_at_source_fps < 1_600_000 {
        // Fast 60 fps footage on a thin H.264 budget (about 2.4-3.2 Mbps): 720p measured cleaner than 900p
        // at the same file size (SSIM 0.893 vs 0.879 on a Rocket League capture), so spend the bits on fewer pixels.
        720
    } else {
        height_at_source_fps
    }
}

#[derive(Clone, Debug)]
struct CompressionEncoder {
    name: String,
    args: Vec<String>,
    hardware: bool,
    nvenc_qp: Option<u64>,
}

fn compression_hardware_recipe(encoder: &str, video_bitrate: u64) -> CompressionEncoder {
    compression_hardware_recipe_with_qp(encoder, video_bitrate, 20)
}

fn compression_hardware_recipe_with_qp(
    encoder: &str,
    video_bitrate: u64,
    nvenc_qp: u64,
) -> CompressionEncoder {
    let bitrate = video_bitrate.to_string();
    let maxrate = ((video_bitrate as f64 * 1.25) as u64).to_string();
    let bufsize = (video_bitrate * 2).to_string();
    let nvenc_qp_string = nvenc_qp.to_string();
    let mut args = if encoder.ends_with("_nvenc") {
        vec![
            "-c:v",
            encoder,
            "-preset",
            "p5",
            "-tune",
            "hq",
            "-rc",
            "constqp",
            "-qp",
            &nvenc_qp_string,
            "-multipass",
            "qres",
            "-spatial-aq",
            "1",
        ]
    } else if encoder.ends_with("_qsv") {
        vec!["-c:v", encoder, "-preset", "slow"]
    } else if encoder.ends_with("_videotoolbox") {
        // Average-bitrate mode. Explicit -1 keeps FFmpeg's old qmin/qmax defaults from capping the bitrate.
        vec!["-c:v", encoder, "-profile:v", "high", "-realtime", "0", "-qmin", "-1", "-qmax", "-1"]
    } else {
        vec!["-c:v", encoder, "-quality", "quality", "-rc", "vbr_peak"]
    }
    .into_iter()
    .map(str::to_owned)
    .collect::<Vec<_>>();
    if !encoder.ends_with("_nvenc") {
        args.extend([
            "-b:v".into(),
            bitrate,
            "-maxrate".into(),
            maxrate,
            "-bufsize".into(),
            bufsize,
        ]);
    }
    if encoder == "h264_nvenc" {
        args.extend([
            "-bf".into(),
            "3".into(),
            "-b_ref_mode".into(),
            "middle".into(),
        ]);
    }
    args.extend(["-pix_fmt".into(), "yuv420p".into()]);
    CompressionEncoder {
        name: encoder.into(),
        args,
        hardware: true,
        nvenc_qp: encoder.ends_with("_nvenc").then_some(nvenc_qp),
    }
}

/// Below this video bitrate, hardware encoders cannot hold a size ceiling (NVIDIA failed at 0.44 and 0.19 Mbps and
/// Quick Sync at 0.19 Mbps in testing; NVIDIA worked at 1.5 Mbps), so whatever encoder is picked, the job goes
/// straight to x264 two-pass instead of failing first. The bitrate is always planned before an encoder is chosen.
const HARDWARE_MIN_VIDEO_BITRATE: u64 = 1_000_000;

/// macOS is exempt: Apple VideoToolbox has no documented failure at low bitrates and is fast, so it is tried first
/// (the corrective re-encode and the x264 fallback still catch a miss). "CPU" always means x264.
fn use_cpu_encoder(video_bitrate: u64, encoder_preference: &str) -> bool {
    encoder_preference == "software"
        || (video_bitrate < HARDWARE_MIN_VIDEO_BITRATE && !cfg!(target_os = "macos"))
}

/// Bitrate-controlled NVENC, used when no constant-QP setting fits under the size ceiling.
fn nvenc_vbr_args(encoder: &str, video_bitrate: u64) -> Vec<String> {
    let mut args: Vec<String> = [
        "-c:v",
        encoder,
        "-preset",
        "p5",
        "-tune",
        "hq",
        "-rc",
        "vbr",
        "-b:v",
        &video_bitrate.to_string(),
        "-maxrate",
        &((video_bitrate as f64 * 1.25) as u64).to_string(),
        "-bufsize",
        &(video_bitrate * 2).to_string(),
        "-multipass",
        "qres",
        "-spatial-aq",
        "1",
        "-pix_fmt",
        "yuv420p",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    if encoder == "h264_nvenc" {
        args.extend(["-bf".into(), "3".into(), "-b_ref_mode".into(), "middle".into()]);
    }
    args
}

// On long exports, avoid repeating the whole video for a constant-QP search.
const LONG_COMPRESSION_SECONDS: f64 = 30.0 * 60.0;

fn compression_recipe_for_job(encoder: &str, bitrate: u64, long: bool) -> CompressionEncoder {
    if long && encoder.ends_with("_nvenc") {
        return CompressionEncoder {
            name: encoder.into(),
            args: nvenc_vbr_args(encoder, bitrate),
            hardware: true,
            nvenc_qp: None,
        };
    }
    compression_hardware_recipe(encoder, bitrate)
}

fn compression_recipe_usable(
    ffmpeg: &str,
    recipe: &CompressionEncoder,
    width: u64,
    height: u64,
) -> bool {
    let mut args = vec![
        "-hide_banner".into(),
        "-loglevel".into(),
        "error".into(),
        "-f".into(),
        "lavfi".into(),
        "-i".into(),
        format!(
            "color=c=black:s={}x{}:r=30",
            width.max(256),
            height.max(256)
        ),
        "-frames:v".into(),
        "1".into(),
    ];
    args.extend(recipe.args.clone());
    args.extend(["-an".into(), "-f".into(), "null".into(), "-".into()]);
    background_command(ffmpeg)
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn automatic_compression_encoder(
    ffmpeg: &str,
    video_bitrate: u64,
    source: &MediaMetadata,
    codec_preference: &str,
    encoder_preference: &str,
    long: bool,
) -> CompressionEncoder {
    if use_cpu_encoder(video_bitrate, encoder_preference) {
        return CompressionEncoder {
            name: "libx264".into(),
            args: Vec::new(),
            hardware: false,
            nvenc_qp: None,
        };
    }
    let usable = |recipe: &CompressionEncoder| {
        let height = source.height.min(compression_height(
            video_bitrate,
            recipe.name.starts_with("av1_"),
            fps(&source.fps),
        ));
        let width =
            (source.width as f64 * height as f64 / source.height.max(1) as f64).round() as u64;
        compression_recipe_usable(ffmpeg, recipe, width & !1, height & !1)
    };
    let allowed = |encoder: &str| match encoder_preference {
        "nvenc" => encoder.ends_with("_nvenc"),
        "qsv" => encoder.ends_with("_qsv"),
        "amf" => encoder.ends_with("_amf"),
        "videotoolbox" => encoder.ends_with("_videotoolbox"),
        _ => true,
    };
    if cfg!(target_os = "windows") {
        if codec_preference != "h264" {
            for encoder in ["av1_nvenc", "av1_qsv", "av1_amf"] {
                if allowed(encoder) && supports_encoder(ffmpeg, encoder) {
                    let recipe = compression_recipe_for_job(encoder, video_bitrate, long);
                    if usable(&recipe) {
                        return recipe;
                    }
                }
            }
        }
        for encoder in ["h264_nvenc", "h264_qsv", "h264_amf"] {
            if allowed(encoder) && supports_encoder(ffmpeg, encoder) {
                let recipe = compression_recipe_for_job(encoder, video_bitrate, long);
                if usable(&recipe) {
                    return recipe;
                }
            }
        }
    }
    if cfg!(target_os = "macos")
        && allowed("h264_videotoolbox")
        && supports_encoder(ffmpeg, "h264_videotoolbox")
    {
        let recipe = compression_recipe_for_job("h264_videotoolbox", video_bitrate, long);
        if usable(&recipe) {
            return recipe;
        }
    }
    CompressionEncoder {
        name: "libx264".into(),
        args: Vec::new(),
        hardware: false,
        nvenc_qp: None,
    }
}

fn cpu_compression_tuning(logical_cores: usize) -> (&'static str, usize) {
    let cores = logical_cores.max(1);
    let preset = if cores <= 4 {
        "fast"
    } else if cores <= 8 {
        "medium"
    } else {
        "slow"
    };
    let threads = if cores <= 2 { cores } else { cores - 2 };
    (preset, threads)
}

const NVENC_QP_MIN: u64 = 16;
const NVENC_QP_MAX: u64 = 42;
const NVENC_MAX_ATTEMPTS: usize = 4;

/// Next constant-QP to try so the file lands just under the target. Uses the measured (qp, bytes)
/// points on a log scale (about half the size every 6 QP until two points give a real slope).
/// Returns None when the last result is already within -7%..0% of the target (never over) or nothing new is worth trying.
fn next_nvenc_qp(tried: &[(u64, u64)], target_bytes: u64) -> Option<u64> {
    let &(qp, bytes) = tried.last()?;
    let target = target_bytes as f64;
    if (target * 0.93..=target).contains(&(bytes as f64)) {
        return None;
    }
    let default_slope = std::f64::consts::LN_2 / 6.0;
    let slope = match tried.len().checked_sub(2).map(|i| tried[i]) {
        Some((q0, b0)) if q0 != qp && b0 > 0 && bytes > 0 => {
            ((b0 as f64 / bytes as f64).ln() / (qp as f64 - q0 as f64)).clamp(0.06, 0.2)
        }
        _ => default_slope,
    };
    let step = ((bytes as f64 / (target * 0.97)).ln() / slope).round();
    let mut next = (qp as f64 + step).clamp(NVENC_QP_MIN as f64, NVENC_QP_MAX as f64) as u64;
    if tried.iter().any(|&(q, _)| q == next) {
        // The model rounded onto a QP we already have: step one notch in the needed direction.
        next = if bytes as f64 > target { qp + 1 } else { qp.saturating_sub(1) };
        if !(NVENC_QP_MIN..=NVENC_QP_MAX).contains(&next) || tried.iter().any(|&(q, _)| q == next) {
            return None;
        }
    }
    Some(next)
}

fn compression_filters(m: &MediaMetadata, height: u64, long: bool) -> String {
    let mut filters = Vec::new();
    if m.is_hdr {
        filters.extend(
            [
                "zscale=t=linear:npl=100",
                "format=gbrpf32le",
                "zscale=p=bt709",
                "tonemap=hable:desat=0",
                "zscale=t=bt709:m=bt709:r=tv",
            ]
            .map(str::to_owned),
        );
    }
    let even_height = (height & !1).max(2);
    if height != m.height || m.width % 2 != 0 || even_height != height {
        let flags = if long { "bicubic" } else { "lanczos" };
        filters.push(format!("scale=-2:{even_height}:flags={flags}"));
    }
    filters.push("format=yuv420p".into());
    filters.join(",")
}

fn correction_bitrate(video: u64, audio: u64, bytes: u64, ceiling: u64) -> u64 {
    let total = (video + audio) as f64 * ceiling as f64 * 0.97 / bytes.max(1) as f64;
    (total as u64)
        .saturating_sub(audio)
        .clamp(1_000, video.max(1_000))
}

/// The whole compression job, independent of the Tauri window so it can be tested from the terminal.
fn compress_video(
    tools: &Tools,
    input: CompressionInput,
    active: &Mutex<ExportJob>,
    emit: &dyn Fn(f64),
) -> Result<CompressionResult, String> {
    let mut m = match input.metadata {
        Some(m) => m,
        None => probe_media(&tools.ffprobe, &input.video_path)?,
    };
    check_compression_target(&m, input.target_size_mb)?;
    let staged = StagedOutput::new(&input.output_path, &input.video_path)?;
    let staged_path = staged.path.to_string_lossy();
    let target_size_mb = input.target_size_mb;
    let original_duration = m.duration;
    let mut plan = compression_plan(&m, target_size_mb);
    let mut target_bytes = ceiling_bytes(target_size_mb);
    let clip = export_clip(m.duration, input.clip_start, input.clip_duration);
    let mut clip_args = Vec::new();
    if let Some((start, length)) = clip {
        let fraction = length / m.duration;
        target_bytes = (target_bytes as f64 * fraction) as u64;
        plan.target_bytes = (plan.target_bytes as f64 * fraction) as u64;
        m.duration = length;
        clip_args = vec![
            "-ss".into(),
            format!("{start:.6}"),
            "-t".into(),
            format!("{length:.6}"),
        ];
    }
    let long = original_duration >= LONG_COMPRESSION_SECONDS;
    let h264_height = plan.output_height;
    let choice = automatic_compression_encoder(
        &tools.ffmpeg,
        plan.video_bitrate,
        &m,
        &input.codec_preference,
        &input.encoder_preference,
        long,
    );
    if choice.name.starts_with("av1_") {
        plan.output_height = planned_compression_height(&m, plan.video_bitrate, true);
    }
    let attempts = tempfile::Builder::new()
        .prefix("SubarollCompress-")
        .tempdir()
        .map_err(|e| e.to_string())?;
    let pass_log = attempts.path().join("pass");
    let mut input_args = vec!["-hide_banner".into(), "-y".into(), "-noautorotate".into()];
    if long || m.width > 4096 || m.height > 4096 {
        input_args.extend(["-filter_threads".into(), "2".into()]);
    }
    input_args.extend(clip_args);
    input_args.extend([
        "-i".into(),
        input.video_path.clone(),
        "-map".into(),
        "0:V:0".into(),
    ]);

    let last_progress = std::cell::Cell::new(0.0);
    let report = |progress: f64| {
        if progress > last_progress.get() {
            last_progress.set(progress);
            emit(progress);
        }
    };
    let video_args = |height| {
        let mut args = input_args.clone();
        args.extend(["-vf".into(), compression_filters(&m, height, long)]);
        preserve_video_timing(&mut args, &m);
        args
    };
    let append_output = |args: &mut Vec<String>, out: &str| {
        if plan.audio_bitrate > 0 {
            args.extend([
                "-map".into(),
                "0:a:0".into(),
                "-c:a".into(),
                "aac".into(),
                "-b:a".into(),
                plan.audio_bitrate.to_string(),
            ]);
        }
        args.extend([
            "-map_metadata".into(),
            "0".into(),
            "-map_chapters".into(),
            "0".into(),
            "-movflags".into(),
            "+faststart".into(),
            "-progress".into(),
            "pipe:1".into(),
            "-nostats".into(),
            out.to_owned(),
        ]);
    };
    let encode_hardware = |recipe: &CompressionEncoder, out: &str, base, span| {
        let mut args = video_args(plan.output_height);
        args.extend(recipe.args.clone());
        append_output(&mut args, out);
        run_progress(
            &tools.ffmpeg,
            &args,
            m.duration,
            base,
            span,
            &report,
            active,
        )?;
        fs::metadata(out)
            .map(|file| file.len())
            .map_err(|error| error.to_string())
    };
    let audio = plan.audio_bitrate;
    let encode_cpu = |bitrate: u64, base, span| {
        let cores = std::thread::available_parallelism()
            .map(|v| v.get())
            .unwrap_or(4);
        let (preset, threads) = cpu_compression_tuning(cores);
        let preset = if long && preset == "slow" {
            "medium"
        } else {
            preset
        };
        let threads = if long { threads.min(8) } else { threads };
        let mut args = video_args(h264_height);
        args.extend([
            "-c:v".into(),
            "libx264".into(),
            "-preset".into(),
            preset.into(),
            "-threads".into(),
            threads.to_string(),
            "-b:v".into(),
            bitrate.to_string(),
            "-maxrate".into(),
            ((bitrate as f64 * 1.25) as u64).to_string(),
            "-bufsize".into(),
            (bitrate * 2).to_string(),
            "-passlogfile".into(),
            pass_log.to_string_lossy().into_owned(),
        ]);
        let mut first = args.clone();
        first.extend([
            "-pass".into(),
            "1".into(),
            "-an".into(),
            "-progress".into(),
            "pipe:1".into(),
            "-nostats".into(),
            "-f".into(),
            "null".into(),
            "-".into(),
        ]);
        run_progress(
            &tools.ffmpeg,
            &first,
            m.duration,
            base,
            span * 0.4,
            &report,
            active,
        )?;
        args.extend(["-pass".into(), "2".into()]);
        append_output(&mut args, &staged_path);
        run_progress(
            &tools.ffmpeg,
            &args,
            m.duration,
            base + span * 0.4,
            span * 0.6,
            &report,
            active,
        )?;
        fs::metadata(&staged.path)
            .map(|file| file.len())
            .map_err(|error| error.to_string())
    };
    // The source-based bitrate cap guides the encoder. Only the selected size is a hard limit.
    let hardware_result = || -> Result<(CompressionEncoder, u64, u64), String> {
        let mut recipe = choice.clone();
        let mut bitrate = plan.video_bitrate;
        if let Some(first_qp) = recipe.nvenc_qp {
            let first_out = attempts.path().join("attempt-0.mp4");
            let first = encode_hardware(&recipe, &first_out.to_string_lossy(), 0.0, 0.14)?;
            let mut tried = vec![(first_qp, first, first_out)];
            while tried.len() < NVENC_MAX_ATTEMPTS {
                let sizes: Vec<_> = tried.iter().map(|&(qp, bytes, _)| (qp, bytes)).collect();
                let Some(qp) = next_nvenc_qp(&sizes, plan.target_bytes) else {
                    break;
                };
                let path = attempts.path().join(format!("attempt-{}.mp4", tried.len()));
                let attempt = compression_hardware_recipe_with_qp(&recipe.name, bitrate, qp);
                let bytes = encode_hardware(
                    &attempt,
                    &path.to_string_lossy(),
                    tried.len() as f64 * 0.14,
                    0.14,
                )?;
                tried.push((qp, bytes, path));
            }
            if let Some((qp, bytes, path)) = tried
                .iter()
                .filter(|(_, size, _)| *size <= plan.target_bytes)
                .max_by_key(|(_, size, _)| *size)
                .or_else(|| {
                    tried
                        .iter()
                        .filter(|(_, size, _)| *size <= target_bytes)
                        .min_by_key(|(_, size, _)| *size)
                })
            {
                fs::copy(path, &staged.path).map_err(|error| error.to_string())?;
                recipe = compression_hardware_recipe_with_qp(&recipe.name, bitrate, *qp);
                return Ok((recipe, *bytes, bitrate));
            }
            recipe.args = nvenc_vbr_args(&recipe.name, bitrate);
            recipe.nvenc_qp = None;
            let bytes = encode_hardware(&recipe, &staged_path, 0.56, 0.17)?;
            return Ok((recipe, bytes, bitrate));
        }

        let mut bytes = encode_hardware(&recipe, &staged_path, 0.0, 0.66)?;
        if bytes > target_bytes {
            bitrate = correction_bitrate(bitrate, audio, bytes, target_bytes);
            recipe = compression_recipe_for_job(&recipe.name, bitrate, long);
            bytes = encode_hardware(&recipe, &staged_path, 0.66, 0.14)?;
        }
        Ok((recipe, bytes, bitrate))
    };

    let mut failure = None;
    let (mut encoder, mut bytes, mut bitrate) = if choice.hardware {
        match hardware_result() {
            Ok(result) => result,
            Err(error) => {
                active.lock().unwrap().check_cancelled()?;
                failure = Some(error);
                (choice, u64::MAX, plan.video_bitrate)
            }
        }
    } else {
        (choice, target_bytes.saturating_add(1), plan.video_bitrate)
    };
    if bytes > target_bytes {
        let used_hardware = encoder.hardware;
        if used_hardware && failure.is_none() {
            bitrate = correction_bitrate(bitrate, audio, bytes, target_bytes);
        }
        let base = if used_hardware { 0.80 } else { 0.0 };
        bytes = encode_cpu(bitrate, base, 0.93 - base).map_err(|error| {
            if error == EXPORT_CANCELLED {
                error
            } else if let Some(hardware_error) = failure.as_ref() {
                format!("Hardware encoding failed: {hardware_error}\nCPU encoding failed: {error}")
            } else {
                error
            }
        })?;
        if bytes > target_bytes {
            bitrate = correction_bitrate(bitrate, audio, bytes, target_bytes);
            bytes = encode_cpu(bitrate, 0.93, 0.06)?;
        }
        encoder = CompressionEncoder {
            name: "libx264".into(),
            args: Vec::new(),
            hardware: false,
            nvenc_qp: None,
        };
        plan.output_height = h264_height;
    }
    if bytes > target_bytes {
        return Err(format!(
            "Couldn't fit this video under {target_size_mb} MB. Choose a larger size."
        ));
    }
    let metadata = probe_media(&tools.ffprobe, &staged_path)?;
    if clip.is_none() {
        validate_frame_count(&m, &metadata)?;
    }
    staged.commit(&input.output_path, active)?;
    emit(1.0);
    Ok(CompressionResult {
        output: input.output_path,
        bytes,
        plan,
        metadata,
        encoder: encoder.name,
        hardware: encoder.hardware,
        nvenc_qp: encoder.nvenc_qp,
    })
}

#[tauri::command]
pub async fn start_compression(
    window: WebviewWindow,
    state: State<'_, AppState>,
    input: CompressionInput,
) -> Result<CompressionResult, String> {
    let job_guard = reserve_job(&state)?;
    let tools = tools_for(&state)?;
    let active = state.export.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _job_guard = job_guard;
        let emit = |v: f64| {
            let _ = window.emit("export-progress", v);
        };
        compress_video(&tools, input, &active, &emit)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn cancel_export(state: State<AppState>) -> bool {
    cancel_running_export(&state)
}

pub fn cancel_running_export(state: &AppState) -> bool {
    let mut job = state.export.lock().unwrap();
    if !job.running {
        return false;
    }
    if let Some(child) = job.child.as_mut() {
        if child.try_wait().ok().flatten().is_none() && child.kill().is_err() {
            return false;
        }
    }
    job.cancelled = true;
    true
}

#[tauri::command]
pub fn reveal_output(output_path: String) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        Command::new("open")
            .args(["-R", &output_path])
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(target_os = "windows")]
    {
        Command::new("explorer")
            .arg(format!("/select,{output_path}"))
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(target_os = "linux")]
    {
        let parent = Path::new(&output_path).parent().unwrap_or(Path::new("."));
        Command::new("xdg-open")
            .arg(parent)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

const REDUX_SHA256: &str = "574614b9a4d9f72ab202877a7ad6a1f4bf819dd1d27b9d42dfe8cd429fcebdd5";
const REDUX_SIZE: u64 = 213_319_296;
const REDUX_URL: &str = "https://huggingface.co/mudler/parakeet-cpp-gguf/resolve/68f808a5903a0bf12657bca3a2a669d573abfb58/redux-packed.gguf";

#[tauri::command]
pub fn choose_generated_subtitle(video_path: String) -> String {
    let path = Path::new(&video_path);
    let mut dialog = rfd::FileDialog::new().set_title("Save generated subtitles").add_filter("SubRip subtitles", &["srt"]);
    if let Some(parent) = path.parent() { dialog = dialog.set_directory(parent); }
    let name = format!("{}.srt", path.file_stem().unwrap_or_default().to_string_lossy());
    dialog.set_file_name(name).save_file().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default()
}

// Like the media exporter, keep the child registered so Cancel and app shutdown reap it.
fn generation_command(program: &str, args: &[String], active: &Mutex<ExportJob>, limit: usize) -> Result<Vec<u8>, String> {
    let (stdout, stderr) = {
        let mut job = active.lock().unwrap();
        job.check_cancelled()?;
        let mut command = background_command(program);
        command.args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
        command.env("PARAKEET_DEVICE", "cpu");
        let mut child = command.spawn().map_err(|e| format!("Could not start subtitle tool: {e}"))?;
        let pipes = (child.stdout.take().unwrap(), child.stderr.take().unwrap());
        job.child = Some(child);
        pipes
    };
    let stdout_reader = std::thread::spawn(move || read_capped(stdout, limit));
    let stderr_reader = std::thread::spawn(move || read_capped(stderr, MAX_TOOL_OUTPUT_BYTES));
    let status = loop {
        let result = {
            let mut job = active.lock().unwrap();
            match job.child.as_mut().unwrap().try_wait() {
                Ok(Some(status)) => { job.child.take(); Some(Ok(status)) },
                Ok(None) => None,
                Err(e) => { if let Some(mut child) = job.child.take() { let _ = child.kill(); let _ = child.wait(); } Some(Err(e)) },
            }
        };
        if let Some(result) = result { break result; }
        std::thread::sleep(std::time::Duration::from_millis(50));
    };
    let (output, truncated) = stdout_reader.join().map_err(|_| "Subtitle tool output failed")?;
    let (errors, _) = stderr_reader.join().map_err(|_| "Subtitle tool error output failed")?;
    active.lock().unwrap().check_cancelled()?;
    if !status.map_err(|e| e.to_string())?.success() {
        return Err(format!("Subtitle generation failed: {}", last_lines(&String::from_utf8_lossy(&errors))));
    }
    if truncated { return Err("Subtitle tool output was too large.".into()); }
    Ok(output)
}

fn verify_redux(path: &Path, active: &Mutex<ExportJob>) -> Result<bool, String> {
    if fs::metadata(path).map(|m| m.len()).ok() != Some(REDUX_SIZE) { return Ok(false); }
    let mut file = fs::File::open(path).map_err(|e| e.to_string())?;
    let mut hash = Sha256::new();
    let mut buf = [0u8; 1024 * 1024];
    loop {
        active.lock().unwrap().check_cancelled()?;
        let count = file.read(&mut buf).map_err(|e| e.to_string())?;
        if count == 0 { break; }
        hash.update(&buf[..count]);
    }
    Ok(format!("{:x}", hash.finalize()) == REDUX_SHA256)
}

fn redux_model(cache: &Path, active: &Mutex<ExportJob>) -> Result<PathBuf, String> {
    fs::create_dir_all(cache).map_err(|e| format!("Cannot create subtitle model folder: {e}"))?;
    let target = cache.join("parakeet-redux-574614b9.gguf");
    if verify_redux(&target, active)? { return Ok(target); }
    let partial = tempfile::Builder::new().prefix("redux-download-").tempfile_in(cache).map_err(|e| e.to_string())?;
    let path = partial.path().to_string_lossy().into_owned();
    let curl = if cfg!(windows) { "curl.exe" } else { "/usr/bin/curl" };
    let args = ["--fail", "--location", "--silent", "--show-error", "--proto", "=https", "--proto-redir", "=https", "--max-filesize", "213319296", "--connect-timeout", "30", "--output", &path, REDUX_URL].map(str::to_owned);
    generation_command(curl, &args, active, 1024)?;
    if !verify_redux(partial.path(), active)? { return Err("The downloaded subtitle model failed its integrity check.".into()); }
    if target.exists() { fs::remove_file(&target).map_err(|e| e.to_string())?; }
    partial.persist(&target).map_err(|e| format!("Could not save the subtitle model: {e}"))?;
    Ok(target)
}

fn srt_clock(seconds: f64) -> String {
    let millis = (seconds.max(0.0) * 1000.0).round() as u64;
    format!("{:02}:{:02}:{:02},{:03}", millis / 3_600_000, (millis / 60_000) % 60, (millis / 1000) % 60, millis % 1000)
}

#[derive(Clone)]
struct SubtitleWord { text: String, start: f64, end: f64 }
fn words_to_srt(words: &[SubtitleWord]) -> String {
    let mut out = String::new();
    let mut block: Vec<&SubtitleWord> = Vec::new();
    let mut number = 0;
    let flush = |block: &mut Vec<&SubtitleWord>, out: &mut String, number: &mut usize| {
        if block.is_empty() { return; }
        *number += 1;
        let start = block[0].start;
        let end = block.last().unwrap().end.max(start + 0.2);
        let text = block.iter().map(|w| w.text.as_str()).collect::<Vec<_>>().join(" ");
        out.push_str(&format!("{}\n{} --> {}\n{}\n\n", number, srt_clock(start), srt_clock(end), text));
        block.clear();
    };
    for word in words {
        let length: usize = block.iter().map(|w| w.text.chars().count() + 1).sum();
        let end_sentence = block.last().is_some_and(|w| w.text.ends_with(['.', '?', '!']));
        if !block.is_empty() && (word.start - block[0].start >= 5.0 || word.start - block.last().unwrap().end > 1.0 || length + word.text.chars().count() > 76 || end_sentence) {
            flush(&mut block, &mut out, &mut number);
        }
        block.push(word);
    }
    flush(&mut block, &mut out, &mut number);
    out
}

#[tauri::command]
pub async fn generate_subtitles(window: WebviewWindow, state: State<'_, AppState>, video_path: String, output_path: String, audio_index: usize) -> Result<String, String> {
    let guard = reserve_job(&state)?;
    let tools = tools_for(&state)?;
    let active = state.export.clone();
    let cache = window.app_handle().path().app_cache_dir().map_err(|e| e.to_string())?.join("models");
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let engine = exe.parent().ok_or("Cannot locate subtitle engine")?.join(if cfg!(windows) { "subaroll-asr.exe" } else { "subaroll-asr" });
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = guard;
        if !engine.is_file() { return Err("Subtitle generation engine is missing from this build.".into()); }
        if Path::new(&output_path).extension().and_then(|s| s.to_str()).map(|s| !s.eq_ignore_ascii_case("srt")).unwrap_or(true) { return Err("Save generated subtitles as an .srt file.".into()); }
        let metadata = probe_media(&tools.ffprobe, &video_path)?;
        if audio_index >= metadata.audio_streams.len() { return Err("The selected audio track is unavailable.".into()); }
        if !metadata.duration.is_finite() || metadata.duration <= 0.0 { return Err("Could not determine the video duration.".into()); }
        let _ = window.emit("generation-phase", "Checking subtitle model…");
        let model = redux_model(&cache, &active)?;
        let staging = StagedOutput::new(&output_path, &video_path)?;
        let scratch = tempfile::Builder::new().prefix("subaroll-asr-").tempdir().map_err(|e| e.to_string())?;
        let mut words: Vec<SubtitleWord> = Vec::new();
        let mut offset = 0.0;
        const CHUNK: f64 = 120.0;
        while offset < metadata.duration {
            active.lock().unwrap().check_cancelled()?;
            let length = CHUNK.min(metadata.duration - offset);
            let start = (offset - 3.0).max(0.0);
            let stop = (offset + length + 3.0).min(metadata.duration);
            let audio = scratch.path().join("audio.wav");
            let _ = window.emit("generation-phase", "Preparing audio…");
            let args = vec!["-v".into(), "error".into(), "-y".into(), "-ss".into(), format!("{start:.3}"), "-i".into(), video_path.clone(), "-t".into(), format!("{:.3}", stop - start), "-map".into(), format!("0:a:{audio_index}"), "-vn".into(), "-ac".into(), "1".into(), "-ar".into(), "16000".into(), "-c:a".into(), "pcm_s16le".into(), audio.to_string_lossy().into_owned()];
            generation_command(&tools.ffmpeg, &args, &active, 1024)?;
            let _ = window.emit("generation-phase", "Generating subtitles…");
            let threads = std::thread::available_parallelism().map(|n| n.get().saturating_sub(1).clamp(1, 4)).unwrap_or(2);
            let args = vec!["transcribe".into(), "--model".into(), model.to_string_lossy().into_owned(), "--input".into(), audio.to_string_lossy().into_owned(), "--threads".into(), threads.to_string(), "--timestamps".into(), "--json".into(), "--vad".into()];
            let result = generation_command(&engine.to_string_lossy(), &args, &active, MAX_TOOL_OUTPUT_BYTES)?;
            let data: Value = serde_json::from_slice(&result).map_err(|e| format!("Invalid subtitle engine output: {e}"))?;
            let items = data["words"].as_array().ok_or("The subtitle engine returned no word timings.")?;
            for item in items {
                let Some(text) = item["w"].as_str() else { continue };
                let (Some(begin), Some(end)) = (item["start"].as_f64(), item["end"].as_f64()) else { continue };
                let begin = start + begin; let end = (start + end).min(metadata.duration);
                let midpoint = (begin + end) / 2.0;
                if midpoint < offset || midpoint >= offset + length || !begin.is_finite() || !end.is_finite() || end <= begin || text.trim().is_empty() { continue; }
                words.push(SubtitleWord { text: text.trim().to_owned(), start: begin, end });
            }
            offset += length;
            let _ = window.emit("export-progress", offset / metadata.duration);
        }
        if words.is_empty() { return Err("No speech was detected in the selected audio track.".into()); }
        let content = words_to_srt(&words);
        if content.len() as u64 > MAX_SUBTITLE_BYTES { return Err("Generated subtitles exceed the 16 MiB limit.".into()); }
        fs::write(&staging.path, content).map_err(|e| format!("Could not write generated subtitles: {e}"))?;
        staging.commit(&output_path, &active)?;
        Ok(output_path)
    }).await.map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    #[test]
    fn generated_subtitles_include_ordered_timed_cues() {
        use super::{words_to_srt, SubtitleWord};
        let words = vec![
            SubtitleWord { text: "Hola".into(), start: 1.1, end: 1.4 },
            SubtitleWord { text: "mundo.".into(), start: 1.4, end: 2.0 },
            SubtitleWord { text: "Siguiente".into(), start: 4.0, end: 4.5 },
        ];
        let result = words_to_srt(&words);
        assert_eq!(result, "1\n00:00:01,100 --> 00:00:02,000\nHola mundo.\n\n2\n00:00:04,000 --> 00:00:04,500\nSiguiente\n\n");
    }

    use super::*;

    #[test]
    #[ignore = "requires FFmpeg and FFprobe"]
    fn real_subtitle_tracks_discover_and_extract() {
        let ffmpeg = std::env::var("SUBAROLL_FFMPEG").unwrap_or_else(|_| {
            if cfg!(target_os = "macos") { "/opt/homebrew/bin/ffmpeg".into() } else { "ffmpeg".into() }
        });
        let ffprobe = Path::new(&ffmpeg).with_file_name(if cfg!(windows) { "ffprobe.exe" } else { "ffprobe" }).to_string_lossy().into_owned();
        let tools = Tools { ffmpeg, ffprobe, video_encoder: String::new() };
        let dir = tempfile::tempdir().unwrap();
        let en = dir.path().join("english.srt");
        let es = dir.path().join("spanish.srt");
        fs::write(&en, "1\n00:00:00,000 --> 00:00:01,000\nHello there\n").unwrap();
        fs::write(&es, "1\n00:00:00,000 --> 00:00:01,000\nHola amigo\n").unwrap();
        let video = dir.path().join("video.mkv");
        let args = vec!["-v".into(), "error".into(), "-y".into(), "-f".into(), "lavfi".into(), "-i".into(), "color=s=160x90:r=10:d=1".into(),
            "-i".into(), en.to_string_lossy().into_owned(), "-i".into(), es.to_string_lossy().into_owned(),
            "-map".into(), "0:v".into(), "-map".into(), "1:s".into(), "-map".into(), "2:s".into(),
            "-c:v".into(), "libx264".into(), "-c:s".into(), "srt".into(),
            "-metadata:s:s:0".into(), "language=eng".into(), "-metadata:s:s:1".into(), "language=spa".into(), video.to_string_lossy().into_owned()];
        run_output(&tools.ffmpeg, &args).unwrap();
        fs::write(dir.path().join("video.pol.srt"), "1\n00:00:00,000 --> 00:00:01,000\nCzesc\n").unwrap();
        let tracks = discover_subtitle_tracks(&tools.ffprobe, video.to_str().unwrap()).unwrap();
        assert_eq!(tracks.len(), 3);
        assert!(tracks[0].path.as_deref().unwrap().ends_with("video.pol.srt"));
        let english = tracks.iter().find(|t| t.language == "eng").unwrap();
        let spanish = tracks.iter().find(|t| t.language == "spa").unwrap();
        assert!(english.supported && spanish.supported);
        let (english_path, _english_file) = extract_subtitle_file(&tools, video.to_str().unwrap(), english.stream_index.unwrap()).unwrap();
        let (spanish_path, _spanish_file) = extract_subtitle_file(&tools, video.to_str().unwrap(), spanish.stream_index.unwrap()).unwrap();
        assert!(fs::read_to_string(english_path).unwrap().contains("Hello there"));
        assert!(fs::read_to_string(spanish_path).unwrap().contains("Hola amigo"));
        assert!(extract_subtitle_file(&tools, video.to_str().unwrap(), 999).is_err());
    }

    #[test]
    fn displayed_quality_matches_encoder_arguments() {
        let metadata = MediaMetadata::default();
        for encoder in ["h264_nvenc", "hevc_nvenc", "h264_amf", "hevc_amf", "h264_qsv", "hevc_qsv"] {
            for ten in [false, true] {
                let sample = windows_encoder_args(encoder, ten, 90, "quality", &metadata);
                let profile = quality_profile_for_args(encoder, &sample, ten, &metadata).unwrap();
                for quality in 60..=100 {
                    let actual = windows_encoder_args(encoder, ten, quality, "quality", &metadata);
                    assert_eq!(profile.values[(quality - 60) as usize], quality_setting_from_args(encoder, &actual).unwrap(), "{encoder}, ten-bit={ten}, quality={quality}");
                    if encoder.ends_with("_nvenc") {
                        assert_eq!(arg_value(&actual, "-rc"), Some("constqp"));
                        assert_eq!(profile.values[(quality - 60) as usize], format!("CQP {}", arg_value(&actual, "-qp").unwrap()));
                        assert!(!actual.iter().any(|value| matches!(value.as_str(), "-cq" | "-maxrate" | "-b:v")));
                    }
                }
            }
        }
        for base in [41, 43, 47] {
            let sample = vec!["-crf".into(), (base - 27).to_string()];
            let profile = quality_profile_for_args("libx265", &sample, true, &metadata).unwrap();
            for quality in 60..=100 {
                let crf = software_crf(base, quality);
                assert_eq!(profile.values[(quality - 60) as usize], format!("CRF {crf}"));
            }
        }
        let profile = quality_profile_for_args("hevc_videotoolbox", &[], false, &metadata).unwrap();
        assert_eq!(profile.values[30], "Quality 90");
        assert_eq!(profile.values[40], "Quality 95");
    }

    #[test]
    fn quality_minimums_are_ten_and_recommended_values_stay_the_same() {
        let metadata = MediaMetadata::default();
        for encoder in ["h264_nvenc", "hevc_nvenc", "h264_qsv", "hevc_qsv"] {
            for ten in [false, true] {
                let flag = if encoder.ends_with("_nvenc") { "-qp" } else { "-global_quality" };
                let expected = if encoder.ends_with("_qsv") && !ten { "15" } else { "18" };
                let default = windows_encoder_args(encoder, ten, 90, "quality", &metadata);
                assert_eq!(arg_value(&default, flag), Some(expected));
                for quality in 60..=100 {
                    let args = windows_encoder_args(encoder, ten, quality, "quality", &metadata);
                    let value = arg_value(&args, flag).unwrap().parse::<i64>().unwrap();
                    assert!(value >= 10);
                    if quality == 100 { assert_eq!(value, 10); }
                }
            }
        }
        for base in [41, 43, 47] {
            assert_eq!(software_crf(base, 90), base - 27);
            for quality in 60..=100 { assert!(software_crf(base, quality) >= 10); }
            assert_eq!(software_crf(base, 100), 10);
        }
        let amd = windows_encoder_args("h264_amf", false, 90, "quality", &metadata);
        assert_eq!(quality_setting_from_args("h264_amf", &amd).as_deref(), Some("CQP 18"));
        assert_eq!(arg_value(&amd, "-qp_i"), Some("16"));
        assert_eq!(arg_value(&amd, "-qp_b"), Some("22"));
        for encoder in ["h264_amf", "hevc_amf"] {
            for ten in [false, true] {
                let max = windows_encoder_args(encoder, ten, 100, "quality", &metadata);
                assert_eq!(quality_setting_from_args(encoder, &max).as_deref(), Some("CQP 10"));
                assert_eq!(arg_value(&max, "-qp_i"), Some("10"));
            }
        }
    }

    #[test]
    fn cancelled_export_keeps_the_existing_output_and_removes_staging() {
        let dir = tempfile::tempdir().unwrap();
        let output = dir.path().join("output.mp4");
        let output = output.to_str().unwrap();
        fs::write(output, b"prior export").unwrap();
        let state = AppState::default();
        let guard = reserve_job(&state).unwrap();
        let staged = StagedOutput::new(output, "source.mp4").unwrap();
        let staging_dir = staged.path.parent().unwrap().to_owned();
        fs::write(&staged.path, b"partial export").unwrap();
        assert!(cancel_running_export(&state));
        assert_eq!(
            staged.commit(output, &state.export).unwrap_err(),
            EXPORT_CANCELLED
        );
        drop(staged);
        drop(guard);
        assert!(!staging_dir.exists());
        assert_eq!(fs::read(output).unwrap(), b"prior export");
        assert!(!cancel_running_export(&state));

        let _next = reserve_job(&state).unwrap();
        let staged = StagedOutput::new(output, "source.mp4").unwrap();
        fs::write(&staged.path, b"finished export").unwrap();
        staged.commit(output, &state.export).unwrap();
        assert_eq!(fs::read(output).unwrap(), b"finished export");
        assert!(StagedOutput::new(output, output).is_err());
    }

    #[test]
    #[ignore = "uses FFmpeg to verify actual child process cancellation"]
    fn real_cancel_reaps_ffmpeg_and_prevents_another_pass() {
        let tools = find_tools(None).expect("FFmpeg with libass is required");
        let state = AppState::default();
        let _guard = reserve_job(&state).unwrap();
        let args = [
            "-hide_banner",
            "-re",
            "-f",
            "lavfi",
            "-i",
            "color=s=320x180:r=30",
            "-progress",
            "pipe:1",
            "-nostats",
            "-t",
            "30",
            "-f",
            "null",
            "-",
        ]
        .map(str::to_owned);
        std::thread::scope(|scope| {
            let worker = scope.spawn(|| {
                run_progress(&tools.ffmpeg, &args, 30.0, 0.0, 1.0, &|_| {}, &state.export)
            });
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
            while state.export.lock().unwrap().child.is_none() {
                assert!(std::time::Instant::now() < deadline, "FFmpeg did not start");
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            assert!(cancel_running_export(&state));
            assert_eq!(worker.join().unwrap().unwrap_err(), EXPORT_CANCELLED);
        });
        assert!(state.export.lock().unwrap().child.is_none());
        assert_eq!(
            run_progress(&tools.ffmpeg, &args, 30.0, 0.0, 1.0, &|_| {}, &state.export).unwrap_err(),
            EXPORT_CANCELLED
        );
    }

    #[test]
    #[ignore = "uses FFmpeg to exercise failing production exports"]
    fn real_export_failures_remove_staging_and_keep_existing_output() {
        let tools = find_tools(None).expect("FFmpeg with libass is required");
        let dir = tempfile::tempdir().unwrap();
        let output = dir
            .path()
            .join("existing.mp4")
            .to_string_lossy()
            .into_owned();
        let source = dir
            .path()
            .join("missing.mp4")
            .to_string_lossy()
            .into_owned();
        let subtitle = dir.path().join("source.srt");
        fs::write(&subtitle, "1\n00:00:00,000 --> 00:00:02,000\nTest\n").unwrap();
        fs::write(&output, b"previous export").unwrap();
        let active = Mutex::new(ExportJob::default());
        let metadata = meta(2.0, 1_000_000);
        let burn = burn_video(
            &tools,
            ExportInput {
                video_path: source.clone(),
                subtitle_path: subtitle.to_string_lossy().into_owned(),
                output_path: output.clone(),
                metadata: Some(metadata.clone()),
                settings: SubtitleSettings {
                    windows_encoder: "software".into(),
                    ..Default::default()
                },
                clip_start: 0.0,
                clip_duration: 0.0,
            },
            &active,
            &|_| {},
        );
        assert!(burn.err().unwrap().contains("missing.mp4"));
        let compress = compress_video(
            &tools,
            CompressionInput {
                video_path: source,
                output_path: output.clone(),
                metadata: Some(metadata),
                target_size_mb: 1.0,
                codec_preference: "h264".into(),
                encoder_preference: "software".into(),
                clip_start: 0.0,
                clip_duration: 0.0,
            },
            &active,
            &|_| {},
        );
        assert!(compress.err().unwrap().contains("missing.mp4"));
        assert_eq!(fs::read(output).unwrap(), b"previous export");
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 2);
        assert!(active.lock().unwrap().child.is_none());
    }

    #[test]
    fn sidecar_subtitle_matching() {
        let dir = tempfile::tempdir().unwrap();
        let touch = |n: &str| fs::write(dir.path().join(n), "x").unwrap();
        for n in [
            "Frieren S02E03.mkv", "Frieren S2E3 [JP].srt", "Frieren S02E30.srt",
            "Frieren S01E03.srt", "Other Show S02E03.srt", "Frieren - 03.srt",
        ] {
            touch(n);
        }
        let found = |v: &str| find_sidecar(&dir.path().join(v)).map(|p| p.file_name().unwrap().to_string_lossy().into_owned());
        assert_eq!(found("Frieren S02E03.mkv").as_deref(), Some("Frieren S2E3 [JP].srt"));
        touch("Frieren S02E03.ass");
        assert_eq!(found("Frieren S02E03.mkv").as_deref(), Some("Frieren S02E03.ass"));
        assert_eq!(found("Frieren S02E04.mkv"), None);
        assert_eq!(found("[Grp] Frieren - 03 [1080p].mkv").as_deref(), Some("Frieren - 03.srt"));
    }

    #[test]
    fn media_tool_output_is_capped_and_truncation_is_reported() {
        let (kept, truncated) = read_capped(&b"abcdef"[..], 4);
        assert_eq!(kept, b"abcd");
        assert!(truncated);

        let (kept, truncated) = read_capped(&b"abc"[..], 4);
        assert_eq!(kept, b"abc");
        assert!(!truncated);
    }

    #[test]
    fn export_reservation_allows_only_one_active_job() {
        let state = AppState::default();
        let first = reserve_job(&state).unwrap();
        assert!(reserve_job(&state).is_err());
        drop(first);
        assert!(reserve_job(&state).is_ok());
    }

    #[test]
    fn cinematic_canvas_adds_only_bottom_area() {
        let m = MediaMetadata {
            width: 3840,
            height: 1600,
            ..Default::default()
        };
        let c = canvas(&m);
        assert_eq!((c.width, c.height, c.padded), (3840, 2160, true));
    }
    #[test]
    fn sixteen_nine_canvas_is_unchanged() {
        let m = MediaMetadata {
            width: 1920,
            height: 1080,
            ..Default::default()
        };
        let c = canvas(&m);
        assert_eq!((c.width, c.height, c.padded), (1920, 1080, false));
    }
    fn meta(duration: f64, bit_rate: u64) -> MediaMetadata {
        MediaMetadata {
            width: 1920,
            height: 1080,
            duration,
            bit_rate,
            fps: "30/1".into(),
            audio_streams: vec![AudioStream::default()],
            ..Default::default()
        }
    }

    #[test]
    fn test_clip_boundaries_and_short_bitrate_budget() {
        for duration in [0.025, 2.0, 29.97, 30.0] {
            assert_eq!(export_clip(duration, 2.0, 30.0), None);
        }
        assert_eq!(export_clip(45.0, 42.0, 30.0), Some((15.0, 30.0)));
        assert_eq!(export_clip(45.0, 1.5, 30.0), Some((1.5, 30.0)));
        assert_eq!(budget_bitrate(1_048_576, 0.025), 322_122_547);
    }

    #[test]
    fn compression_resolution_uses_bitrate_per_frame_and_source_dimensions() {
        let mut m = meta(7200.0, 0);
        m.width = 7680;
        m.height = 4320;
        m.audio_streams.clear();
        m.fps = "120/1".into();
        let plan = compression_plan(&m, 750.0);
        assert_eq!(plan.video_bitrate, 792_000);
        assert_eq!(plan.output_height, 480);
        m.fps = "2/1".into();
        assert_eq!(compression_plan(&m, 750.0).output_height, 900);
        m.width = 7680;
        m.height = 1800;
        m.duration = 10.0;
        assert_eq!(compression_plan(&m, 100.0).output_height, 960);
        m.width = 640;
        m.height = 359;
        assert_eq!(compression_plan(&m, 100.0).output_height, 358);
    }

    #[test]
    fn compression_target_limits_are_enforced() {
        let m = meta(600.0, 0);
        for bad in [0.0, 0.5, 750.1, 10000.0, f64::NAN, f64::INFINITY, -5.0] {
            assert!(check_compression_target(&m, bad).is_err(), "{bad} should be refused");
        }
        for good in [1.0 + 24.0, 25.0, 100.0, 250.0, 750.0] {
            assert!(check_compression_target(&m, good).is_ok(), "{good} should be accepted");
        }
        assert!(check_compression_target(&meta(0.0, 0), 25.0).is_err());
        assert!(check_compression_target(&meta(f64::NAN, 0), 25.0).is_err());
    }

    #[test]
    fn too_small_targets_are_refused_with_a_usable_minimum() {
        let long = meta(2.0 * 3600.0, 0);
        let error = check_compression_target(&long, 25.0).unwrap_err();
        assert!(error.contains("at least"), "{error}");
        // The advertised minimum must itself be accepted, and one MB less must not.
        let min = min_target_mb(&long);
        assert!(min <= MAX_COMPRESSION_MB);
        assert!(check_compression_target(&long, min).is_ok());
        assert!(check_compression_target(&long, min - 1.0).is_err());
        // The same 25 MB is fine for a short video.
        assert!(check_compression_target(&meta(300.0, 0), 25.0).is_ok());
    }

    #[test]
    fn every_accepted_target_keeps_a_watchable_bitrate() {
        for duration in [5.0, 60.0, 600.0, 3600.0, 7200.0, 21600.0] {
            let m = meta(duration, 0);
            for mb in [1.0, 5.0, 10.0, 25.0, 100.0, 250.0, 500.0, 750.0] {
                if check_compression_target(&m, mb).is_ok() {
                    let plan = compression_plan(&m, mb);
                    assert!(plan.video_bitrate >= MIN_VIDEO_BITRATE, "{mb} MB / {duration}s");
                    assert!(plan.target_bytes <= ceiling_bytes(MAX_COMPRESSION_MB));
                    assert!(plan.output_height >= 480.min(m.height));
                }
            }
        }
    }

    #[test]
    fn multi_hour_targets_use_the_actual_audio_budget() {
        let mut movie = meta(6.0 * 3600.0, 4_000_000);
        let minimum = min_target_mb(&movie);
        assert!(minimum > 250.0 && minimum <= 750.0);
        assert!(check_compression_target(&movie, minimum).is_ok());
        assert!(check_compression_target(&movie, minimum - 1.0).is_err());
        let plan = compression_plan(&movie, 750.0);
        assert!(plan.output_height > 0 && plan.video_bitrate > MIN_VIDEO_BITRATE);
        movie.audio_streams.clear();
        assert!(min_target_mb(&movie) < minimum);
        assert_eq!(compression_plan(&movie, 750.0).audio_bitrate, 0);

        movie.duration = 12.0 * 3600.0;
        let error = check_compression_target(&movie, 750.0).unwrap_err();
        assert!(error.contains("needs at least") && error.contains("750 MB"));
    }

    #[test]
    fn long_nvenc_uses_bitrate_control_and_high_budgets_keep_more_resolution() {
        let short = compression_recipe_for_job("h264_nvenc", 3_000_000, false);
        let long = compression_recipe_for_job("h264_nvenc", 3_000_000, true);
        assert_eq!(short.nvenc_qp, Some(20));
        assert!(long.nvenc_qp.is_none());
        assert!(long.args.windows(2).any(|pair| pair == ["-rc", "vbr"]));
        let mut source = meta(600.0, 12_000_000);
        source.width = 3840;
        source.height = 2160;
        let smaller = compression_plan(&source, 250.0);
        let larger = compression_plan(&source, 750.0);
        assert!(larger.output_height > smaller.output_height);
        assert!(larger.video_bitrate > smaller.video_bitrate);
    }

    #[test]
    fn huge_targets_do_not_inflate_past_the_source() {
        let plan = compression_plan(&meta(60.0, 1_000_000), 250.0);
        assert!(plan.video_bitrate <= 1_500_000, "{}", plan.video_bitrate);
        // With an unknown source bitrate the budget is used as given.
        assert!(compression_plan(&meta(60.0, 0), 250.0).video_bitrate > 1_500_000);
    }

    #[test]
    fn estimate_matches_the_plan_and_av1_never_picks_less_than_h264() {
        let m = meta(300.0, 0);
        let h264 = plan_compression(m.clone(), 25.0, false, "auto".into()).unwrap();
        let av1 = plan_compression(m.clone(), 25.0, true, "auto".into()).unwrap();
        assert_eq!(h264.video_bitrate, compression_plan(&m, 25.0).video_bitrate);
        assert!(av1.output_height >= h264.output_height);
        assert!(plan_compression(m, 0.0, false, "auto".into()).is_err());
    }

    #[test]
    fn authored_ass_positions_survive_shared_dialogue_styles() {
        let source = r"[Script Info]
PlayResX: 640
PlayResY: 360
[Events]
Format: Layer, Start, End, Style, Text
Dialogue: 0,0:00:00.00,0:00:01.00,Default,Regular dialogue
Dialogue: 0,0:00:00.00,0:00:01.00,Default,{\an7\pos(95,130)}Positioned sign
Dialogue: 0,0:00:00.00,0:00:01.00,Default,{\move(20,30,40,50)}Moving sign
Dialogue: broken";
        let settings = SubtitleSettings {
            position_percent: 20.0,
            scale_percent: 150.0,
            ..Default::default()
        };
        let transformed = transform_ass(source, &settings, false, 16.0 / 9.0);
        assert!(transformed.contains(r"{\an2\pos(320,110)}Regular dialogue"));
        assert!(transformed.contains(r"Default,{\an7\pos(95,130)}Positioned sign"));
        assert!(transformed.contains(r"Default,{\move(20,30,40,50)}Moving sign"));
    }

    /// 1080p30 test clip with audio, built with the same ffmpeg the app uses.
    fn make_clip(tools: &Tools, dir: &Path, seconds: u32) -> PathBuf {
        let path = dir.join("source.mp4");
        let status = Command::new(&tools.ffmpeg)
            .args(["-y", "-hide_banner", "-loglevel", "error", "-f", "lavfi", "-i"])
            .arg(format!("testsrc2=size=1920x1080:rate=30:duration={seconds}"))
            .args(["-f", "lavfi", "-i"])
            .arg(format!("sine=frequency=440:duration={seconds}"))
            .args(["-c:v", "libx264", "-preset", "veryfast", "-b:v", "6M", "-pix_fmt", "yuv420p"])
            .args(["-c:a", "aac", "-b:a", "128k"])
            .arg(&path)
            .status()
            .expect("run ffmpeg");
        assert!(status.success(), "could not build the test clip");
        path
    }

    #[test]
    #[ignore = "runs FFmpeg and libass against a generated video"]
    fn real_preview_renders_with_and_without_subtitles() {
        let tools = find_tools(None).expect("FFmpeg with libass is required");
        let dir = tempfile::tempdir().unwrap();
        let video = make_clip(&tools, dir.path(), 2);
        let subtitle = dir.path().join("source.srt");
        fs::write(&subtitle, "1\n00:00:00,000 --> 00:00:02,000\nPreview test\n").unwrap();
        let mut input = PreviewInput {
            video_path: video.to_string_lossy().into_owned(),
            subtitle_path: None,
            metadata: None,
            settings: SubtitleSettings {
                position_percent: 90.0,
                scale_percent: 100.0,
                ..Default::default()
            },
            time: 0.5,
        };
        let video_only = preview_video(&tools, input.clone()).unwrap();
        assert!(video_only.frame.starts_with("data:image/png;base64,iVBOR"));
        assert!(video_only.subtitle_error.is_none());
        input.time = 1.5;
        let seeked = preview_video(&tools, input.clone()).unwrap();
        assert_ne!(seeked.frame, video_only.frame);

        input.subtitle_path = Some(subtitle.to_string_lossy().into_owned());
        let with_subtitles = preview_video(&tools, input.clone()).unwrap();
        assert_ne!(with_subtitles.frame, seeked.frame);
        assert!(with_subtitles.subtitle_error.is_none());

        fs::remove_file(subtitle).unwrap();
        let missing_subtitles = preview_video(&tools, input).unwrap();
        assert_eq!(missing_subtitles.frame, seeked.frame);
        assert!(missing_subtitles.subtitle_error.is_some());
    }

    #[cfg(target_os = "macos")]
    fn bright_bounds(tools: &Tools, path: &str, time: f64, width: usize) -> Option<[usize; 4]> {
        let pixels = run_output(
            &tools.ffmpeg,
            &[
                "-v",
                "error",
                "-ss",
                &fmt(time),
                "-i",
                path,
                "-frames:v",
                "1",
                "-f",
                "rawvideo",
                "-pix_fmt",
                "gray",
                "pipe:1",
            ]
            .map(str::to_owned),
        )
        .unwrap();
        assert!(
            !pixels.is_empty(),
            "no decoded pixels from {path} at {time}s"
        );
        let mut bounds = None;
        for (i, value) in pixels.into_iter().enumerate() {
            if value < 180 {
                continue;
            }
            let [x, y] = [i % width, i / width];
            let [left, top, right, bottom] = bounds.unwrap_or([x, y, x, y]);
            bounds = Some([left.min(x), top.min(y), right.max(x), bottom.max(y)]);
        }
        bounds
    }

    fn frame_times(tools: &Tools, path: &str) -> Vec<String> {
        let bytes = run_output(
            &tools.ffprobe,
            &[
                "-v",
                "error",
                "-select_streams",
                "v:0",
                "-show_entries",
                "frame=best_effort_timestamp_time",
                "-of",
                "json",
                path,
            ]
            .map(str::to_owned),
        )
        .unwrap();
        serde_json::from_slice::<Value>(&bytes).unwrap()["frames"]
            .as_array()
            .unwrap()
            .iter()
            .map(|frame| {
                frame["best_effort_timestamp_time"]
                    .as_str()
                    .unwrap()
                    .to_owned()
            })
            .collect()
    }

    #[test]
    #[ignore = "requires FFmpeg with libass, x264 and x265"]
    fn real_short_exports_preserve_frames_at_extreme_resolutions_and_rates() {
        let tools = tools_for(&AppState::default()).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let subtitle = dir.path().join("source.srt");
        fs::write(
            &subtitle,
            "1\n00:00:00,000 --> 00:00:29,000\nSubtitle test\n",
        )
        .unwrap();

        for (label, size, rate, frames, is_8k) in [
            ("low", "320x180", "1/2", "3", false),
            ("high", "640x360", "120", "97", false),
            ("variable", "640x360", "60", "45", false),
            ("4k", "3840x2160", "24", "7", false),
            ("8k", "7680x4320", "1", "2", true),
        ] {
            let source = dir.path().join(format!("{label}.mp4"));
            let source = source.to_str().unwrap();
            let mut args = [
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                &format!("testsrc2=size={size}:rate={rate}"),
                "-frames:v",
                frames,
            ]
            .map(str::to_owned)
            .to_vec();
            if label == "variable" {
                args.extend(
                    [
                        "-vf",
                        "select=not(mod(n\\,2))+not(mod(n\\,3))",
                        "-fps_mode",
                        "passthrough",
                    ]
                    .map(str::to_owned),
                );
            }
            args.extend(if is_8k {
                [
                    "-c:v",
                    "libx265",
                    "-preset",
                    "ultrafast",
                    "-x265-params",
                    "pools=4:frame-threads=2",
                    "-tag:v",
                    "hvc1",
                ]
                .map(str::to_owned)
                .to_vec()
            } else {
                ["-c:v", "libx264", "-preset", "ultrafast", "-crf", "18"]
                    .map(str::to_owned)
                    .to_vec()
            });
            args.push(source.into());
            run_output(&tools.ffmpeg, &args).unwrap();
            let meta = probe_media(&tools.ffprobe, source).unwrap();
            let timestamps = frame_times(&tools, source);
            let active = Mutex::new(ExportJob::default());
            let mb = if is_8k || label == "4k" { 25.0 } else { 1.0 };
            let estimate = plan_compression(meta.clone(), mb, false, "software".into()).unwrap();

            for (action, clip_duration) in [("sample", 30.0), ("full", 0.0)] {
                let burn_path = dir.path().join(format!("{label}-burn-{action}.mp4"));
                let burn = burn_video(
                    &tools,
                    ExportInput {
                        video_path: source.into(),
                        subtitle_path: subtitle.to_string_lossy().into_owned(),
                        output_path: burn_path.to_string_lossy().into_owned(),
                        metadata: Some(meta.clone()),
                        settings: SubtitleSettings {
                            quality: 90.0,
                            windows_encoder: "software".into(),
                            ..Default::default()
                        },
                        clip_start: meta.duration / 2.0,
                        clip_duration,
                    },
                    &active,
                    &|_| {},
                )
                .unwrap_or_else(|error| panic!("{label} burn {action}: {error}"));
                assert_eq!(
                    frame_times(&tools, &burn.output),
                    timestamps,
                    "{label} burn {action}"
                );
                let burned = probe_media(&tools.ffprobe, &burn.output).unwrap();
                assert_eq!((burned.width, burned.height), (meta.width, meta.height));

                let compress_path = dir.path().join(format!("{label}-compress-{action}.mp4"));
                let compressed = compress_video(
                    &tools,
                    CompressionInput {
                        video_path: source.into(),
                        output_path: compress_path.to_string_lossy().into_owned(),
                        metadata: Some(meta.clone()),
                        target_size_mb: mb,
                        codec_preference: "h264".into(),
                        encoder_preference: "software".into(),
                        clip_start: meta.duration / 2.0,
                        clip_duration,
                    },
                    &active,
                    &|_| {},
                )
                .unwrap_or_else(|error| panic!("{label} compress {action}: {error}"));
                assert_eq!(
                    frame_times(&tools, &compressed.output),
                    timestamps,
                    "{label} compress {action}"
                );
                assert_eq!(compressed.metadata.height, estimate.output_height);
                assert!(
                    compressed.metadata.width <= meta.width && compressed.metadata.width <= 4096
                );
                assert!(compressed.bytes <= ceiling_bytes(mb));
                println!(
                    "{label} {action}: {} frames, burn {}x{} {}, compress {}x{} {} bytes at {} bps ({} planned)",
                    timestamps.len(), burned.width, burned.height, burn.encoder,
                    compressed.metadata.width, compressed.metadata.height, compressed.bytes,
                    compressed.metadata.bit_rate, estimate.video_bitrate,
                );
            }
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "requires actual Apple VideoToolbox hardware and FFmpeg with libass"]
    fn real_apple_and_cpu_export_matrix() {
        let state = AppState::default();
        let started = std::time::Instant::now();
        let tools = tools_for(&state).unwrap();
        println!(
            "FFmpeg discovery once at startup: {:.0?}",
            started.elapsed()
        );
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("burn-source.mp4");
        let source = source.to_str().unwrap();
        run_output(
            &tools.ffmpeg,
            &[
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "color=c=0x202020:s=640x360:r=30000/1001:d=4",
                "-vf",
                "noise=alls=12:allf=t+u:all_seed=123",
                "-c:v",
                "libx264",
                "-crf",
                "18",
                "-pix_fmt",
                "yuv420p",
                source,
            ]
            .map(str::to_owned),
        )
        .unwrap();
        let input_times = frame_times(&tools, source);
        let input_meta = probe_media(&tools.ffprobe, source).unwrap();
        let subtitle = dir.path().join("source.srt");
        fs::write(
            &subtitle,
            "1\n00:00:00,000 --> 00:00:01,500\nPreview test\nSecond line\n",
        )
        .unwrap();

        let preview_path = dir.path().join("preview.png");
        let mut prior: Option<[usize; 4]> = None;
        for (position, size, delay, quality) in
            [(20.0, 75.0, 0.55, 65.0), (85.0, 150.0, -0.25, 85.0)]
        {
            let mut input = PreviewInput {
                video_path: source.into(),
                subtitle_path: Some(subtitle.to_string_lossy().into_owned()),
                metadata: Some(input_meta.clone()),
                settings: SubtitleSettings {
                    delay_seconds: delay,
                    position_percent: position,
                    scale_percent: size,
                    quality,
                    ..Default::default()
                },
                time: 0.4,
            };
            let times = [0.4, 0.72, 1.35, 1.95, 2.15];
            let mut preview_bounds = Vec::new();
            for t in times {
                input.time = t;
                let started = std::time::Instant::now();
                let result = preview_video(&tools_for(&state).unwrap(), input.clone()).unwrap();
                println!(
                    "Preview {}% position, {}% size, seek to {t}s: {:.0?}",
                    position,
                    size,
                    started.elapsed()
                );
                assert!(result.subtitle_error.is_none());
                fs::write(
                    &preview_path,
                    BASE64
                        .decode(result.frame.split_once(',').unwrap().1)
                        .unwrap(),
                )
                .unwrap();
                preview_bounds.push(bright_bounds(
                    &tools,
                    preview_path.to_str().unwrap(),
                    0.0,
                    640,
                ));
            }
            assert_eq!(
                preview_bounds
                    .iter()
                    .map(Option::is_some)
                    .collect::<Vec<_>>(),
                if delay > 0.0 {
                    vec![false, true, true, true, false]
                } else {
                    vec![true, true, false, false, false]
                },
                "subtitle timing at delay {delay}"
            );
            let bounds = preview_bounds[1].unwrap();
            if let Some([left, top, right, bottom]) = prior {
                assert!(bounds[1] > bottom, "85% position should be below 20%");
                assert!(
                    bounds[2] - bounds[0] > right - left && bounds[3] - bounds[1] > bottom - top,
                    "150% size should be larger than 75%"
                );
            }
            prior = Some(bounds);

            for (choice, expected, quality_flag, quality_value) in [
                ("videotoolbox", "hevc_videotoolbox", "-q:v", quality as i64),
                (
                    "software",
                    "libx264",
                    "-crf",
                    (41.0_f64 - quality * 0.3).round() as i64,
                ),
            ] {
                let settings = SubtitleSettings {
                    windows_encoder: choice.into(),
                    ..input.settings.clone()
                };
                let (_, args, _) = encoder_args(&tools.ffmpeg, &input_meta, &settings);
                assert!(args
                    .windows(2)
                    .any(|pair| pair[0] == quality_flag && pair[1] == quality_value.to_string()));
                let out = dir.path().join(format!("burn-{choice}-{quality}.mp4"));
                let out = out.to_str().unwrap();
                let started = std::time::Instant::now();
                let job = ExportInput {
                    video_path: source.into(),
                    subtitle_path: input.subtitle_path.clone().unwrap(),
                    output_path: out.into(),
                    metadata: Some(input_meta.clone()),
                    settings,
                    clip_start: 0.0,
                    clip_duration: 0.0,
                };
                let result =
                    burn_video(&tools, job.clone(), &Mutex::new(ExportJob::default()), &|_| {}).unwrap();
                assert_eq!(
                    result.encoder, expected,
                    "selected hardware may not silently fall back"
                );
                assert_eq!(result.quality_setting, format!("{} {}", if choice == "software" { "CRF" } else { "Quality" }, quality_value));
                assert_eq!(result.quality, quality as i64);
                assert_eq!(
                    frame_times(&tools, out),
                    input_times,
                    "burn changed frame times"
                );
                for (t, preview) in times.iter().zip(&preview_bounds) {
                    let burn = bright_bounds(&tools, out, *t, 640);
                    match (preview, burn) {
                        (Some(a), Some(b)) => assert!(
                            a.iter().zip(b).all(|(x, y)| x.abs_diff(y) <= 2),
                            "{choice} at {t}s: preview {a:?}, burn {b:?}"
                        ),
                        _ => assert_eq!(*preview, burn, "{choice} at {t}s"),
                    }
                }
                if delay > 0.0 {
                    let clip = dir.path().join(format!("clip-{choice}.mp4"));
                    let clip = clip.to_str().unwrap();
                    burn_video(
                        &tools,
                        ExportInput {
                            output_path: clip.into(),
                            clip_start: 1.0,
                            clip_duration: 2.0,
                            ..job
                        },
                        &Mutex::new(ExportJob::default()),
                        &|_| {},
                    )
                    .unwrap();
                    assert_eq!(bright_bounds(&tools, clip, 0.35, 640), preview_bounds[2]);
                    assert!(bright_bounds(&tools, clip, 1.15, 640).is_none());
                }
                let meta = probe_media(&tools.ffprobe, out).unwrap();
                println!("Burn {expected} CQ {quality}: {:.0?} incl. verification, {} bps, {} bytes, {} frames",
                    started.elapsed(), meta.bit_rate, fs::metadata(out).unwrap().len(), input_times.len());
            }
        }

        let source = make_clip(&tools, dir.path(), 8);
        let source = source.to_str().unwrap();
        let source_meta = probe_media(&tools.ffprobe, source).unwrap();
        let source_times = frame_times(&tools, source);
        println!(
            "Compression source: {} bytes, {} bps, {}x{}, {} fps, {:.2}s",
            fs::metadata(source).unwrap().len(),
            source_meta.bit_rate,
            source_meta.width,
            source_meta.height,
            fps(&source_meta.fps),
            source_meta.duration
        );
        for (choice, expected) in [
            ("videotoolbox", "h264_videotoolbox"),
            ("software", "libx264"),
        ] {
            let out = dir.path().join(format!("compress-{choice}.mp4"));
            let out = out.to_str().unwrap();
            let started = std::time::Instant::now();
            let result = compress_video(
                &tools,
                CompressionInput {
                    video_path: source.into(),
                    output_path: out.into(),
                    metadata: Some(source_meta.clone()),
                    target_size_mb: 1.0,
                    codec_preference: "auto".into(),
                    encoder_preference: choice.into(),
                    clip_start: 0.0,
                    clip_duration: 0.0,
                },
                &Mutex::new(ExportJob::default()),
                &|_| {},
            )
            .unwrap();
            let elapsed = started.elapsed();
            assert_eq!(
                result.encoder, expected,
                "Apple test must really use hardware"
            );
            assert!(result.bytes <= ceiling_bytes(1.0));
            assert_eq!(frame_times(&tools, out), source_times);
            assert!(!result.metadata.audio_streams.is_empty());
            let estimate =
                plan_compression(source_meta.clone(), 1.0, false, choice.into()).unwrap();
            assert_eq!(result.metadata.height, estimate.output_height);
            println!("Compress {expected}: {:.0?}, {} bytes, {} bps ({} planned), {}x{}, {} fps, {} frames",
                elapsed, result.bytes, result.metadata.bit_rate, estimate.video_bitrate,
                result.metadata.width, result.metadata.height, fps(&result.metadata.fps), source_times.len());
        }
    }

    fn run_compression(
        tools: &Tools,
        source: &Path,
        out: &Path,
        mb: f64,
        clip: (f64, f64),
    ) -> Result<CompressionResult, String> {
        let active = Mutex::new(ExportJob::default());
        let last = Mutex::new(0.0_f64);
        let emit = |v: f64| *last.lock().unwrap() = v;
        let result = compress_video(
            tools,
            CompressionInput {
                video_path: source.to_string_lossy().into_owned(),
                output_path: out.to_string_lossy().into_owned(),
                metadata: None,
                target_size_mb: mb,
                codec_preference: "auto".into(),
                encoder_preference: std::env::var("SUBAROLL_TEST_ENCODER").unwrap_or_else(|_| "auto".into()),
                clip_start: clip.0,
                clip_duration: clip.1,
            },
            &active,
            &emit,
        );
        if result.is_ok() {
            assert!(*last.lock().unwrap() >= 0.99, "progress never reached 100%");
        }
        assert!(active.lock().unwrap().child.is_none(), "ffmpeg process was left registered");
        result
    }

    #[test]
    #[ignore = "runs real ffmpeg compressions; use: cargo test real_compression -- --ignored --nocapture"]
    fn real_compression_meets_target_without_crashing_or_degrading() {
        let tools = find_tools(None).expect("ffmpeg/ffprobe not found");
        let dir = tempfile::tempdir().unwrap();
        let source = make_clip(&tools, dir.path(), 40);
        let source_bytes = fs::metadata(&source).unwrap().len();
        let source_meta = probe_media(&tools.ffprobe, source.to_str().unwrap()).unwrap();

        // 1. A normal Discord-style target: stays under the limit, keeps full length, audio, watchable size.
        let out = dir.path().join("a.mp4");
        let r = run_compression(&tools, &source, &out, 8.0, (0.0, 0.0)).expect("8 MB compression");
        let limit = ceiling_bytes(8.0);
        println!("8 MB target -> {} bytes via {} at {}p {}fps", r.bytes, r.encoder, r.metadata.height, r.metadata.fps);
        assert!(r.bytes <= limit, "{} > {limit}", r.bytes);
        assert!(r.metadata.height >= 480 && r.metadata.height <= source_meta.height);
        assert!((r.metadata.duration - source_meta.duration).abs() < 1.5);
        assert!(!r.metadata.audio_streams.is_empty(), "audio was dropped");
        assert!(r.bytes > 100_000, "output suspiciously tiny");

        // 1b. Under 5 MB nothing is squeezed for a margin: the plain 4 MB ceiling applies.
        let r = run_compression(&tools, &source, &dir.path().join("a4.mp4"), 4.0, (0.0, 0.0)).expect("4 MB compression");
        println!("4 MB target -> {} bytes (ceiling {})", r.bytes, ceiling_bytes(4.0));
        assert!(r.bytes <= ceiling_bytes(4.0));
        assert!(r.bytes > ceiling_bytes(4.0) / 2, "4 MB result wastes more than half the budget");

        // 2. A target too small for this length is refused up front, before any encoding.
        let err = run_compression(&tools, &source, &dir.path().join("b.mp4"), 1.0, (0.0, 0.0)).err().expect("should be refused");
        println!("1 MB target -> refused: {err}");
        assert!(err.contains("at least"), "{err}");
        assert!(!dir.path().join("b.mp4").exists(), "a refused target still produced a file");

        // 3. The maximum target must not bloat past the source or the 250 MB cap.
        let out = dir.path().join("c.mp4");
        let r = run_compression(&tools, &source, &out, 250.0, (0.0, 0.0)).expect("250 MB compression");
        println!("250 MB target -> {} bytes (source {source_bytes}) via {}", r.bytes, r.encoder);
        assert!(r.bytes as f64 <= source_bytes as f64 * 2.0, "output bloated past 2x the source");
        assert!(r.bytes <= ceiling_bytes(250.0));

        // 4. Out-of-range requests are refused, never encoded.
        for bad in [0.0, 750.1, f64::NAN] {
            assert!(run_compression(&tools, &source, &dir.path().join("d.mp4"), bad, (0.0, 0.0)).is_err());
        }

        // 5. A 10 s test clip uses the same bitrate as the full encode, so its size scales with its length.
        let out = dir.path().join("e.mp4");
        let r = run_compression(&tools, &source, &out, 8.0, (5.0, 10.0)).expect("clip compression");
        println!("10 s clip of the 8 MB plan -> {} bytes, {:.1}s", r.bytes, r.metadata.duration);
        assert!((r.metadata.duration - 10.0).abs() < 1.5, "clip length {}", r.metadata.duration);
        assert!(r.bytes <= (ceiling_bytes(8.0) as f64 * (10.0 / 40.0)) as u64);
    }

    #[test]
    #[ignore = "needs SUBAROLL_TEST_SOURCE and SUBAROLL_TEST_OUT (optional SUBAROLL_TEST_RATIO, default 0.2)"]
    fn real_compression_to_fraction_of_source() {
        let var = |k: &str| std::env::var(k).unwrap_or_else(|_| panic!("set {k}"));
        let source = PathBuf::from(var("SUBAROLL_TEST_SOURCE"));
        let out = PathBuf::from(var("SUBAROLL_TEST_OUT"));
        let ratio: f64 = std::env::var("SUBAROLL_TEST_RATIO").ok().and_then(|v| v.parse().ok()).unwrap_or(0.2);
        let tools = find_tools(None).expect("ffmpeg/ffprobe not found");
        let src_bytes = fs::metadata(&source).unwrap().len();
        let src = probe_media(&tools.ffprobe, source.to_str().unwrap()).unwrap();
        // The app takes whole megabytes, so round the same way a user would type it.
        let target_mb = std::env::var("SUBAROLL_TEST_MB").ok().and_then(|v| v.parse().ok())
            .unwrap_or_else(|| (src_bytes as f64 * ratio / 1_048_576.0).round());
        let plan = compression_plan(&src, target_mb);
        let started = std::time::Instant::now();
        let r = run_compression(&tools, &source, &out, target_mb, (0.0, 0.0)).expect("compression failed");
        let secs = started.elapsed().as_secs_f64();
        println!("SOURCE   {}x{} @ {} fps, {:.1}s, video {:.2} Mbps, file {:.1} MB", src.width, src.height, src.fps, src.duration, src.bit_rate as f64 / 1e6, src_bytes as f64 / 1_048_576.0);
        println!("TARGET   {target_mb} MB ({} bytes ceiling); plan video {:.2} Mbps + audio {} kbps, {}p @ {:.1} fps", ceiling_bytes(target_mb), plan.video_bitrate as f64 / 1e6, plan.audio_bitrate / 1000, plan.output_height, fps(&src.fps));
        println!("ENCODER  {} (hardware: {}){}", r.encoder, r.hardware, r.nvenc_qp.map(|q| format!(", NVENC QP {q}")).unwrap_or_default());
        println!("OUTPUT   {}x{} @ {} fps, {:.1}s, video {:.2} Mbps, file {:.1} MB = {:.1}% of source", r.metadata.width, r.metadata.height, r.metadata.fps, r.metadata.duration, r.metadata.bit_rate as f64 / 1e6, r.bytes as f64 / 1_048_576.0, r.bytes as f64 * 100.0 / src_bytes as f64);
        println!("TIME     {secs:.0}s ({:.1}x realtime)", src.duration / secs);
        assert!(r.bytes <= ceiling_bytes(target_mb), "over target");
        assert!((r.metadata.duration - src.duration).abs() < 1.5, "length changed");
        assert!(!r.metadata.audio_streams.is_empty(), "audio dropped");
    }

    #[test]
    fn nvenc_qp_search_converges_on_the_target_without_repeating() {
        // Model: size halves every 6 QP; the true size at QP 20 is 6x the target (needs about QP 35).
        let target = 100_000_000u64;
        let size_at = |qp: u64| (target as f64 * 6.0 * 2f64.powf(-((qp as f64) - 20.0) / 6.0)) as u64;
        let mut tried = vec![(20, size_at(20))];
        while let Some(qp) = next_nvenc_qp(&tried, target) {
            assert!(!tried.iter().any(|&(q, _)| q == qp), "repeated QP {qp}");
            assert!((NVENC_QP_MIN..=NVENC_QP_MAX).contains(&qp));
            tried.push((qp, size_at(qp)));
            assert!(tried.len() <= NVENC_MAX_ATTEMPTS, "did not settle: {tried:?}");
        }
        let (_, best) = *tried.last().unwrap();
        assert!(best as f64 >= target as f64 * 0.90 && best <= target, "{tried:?}");
    }

    #[test]
    fn nvenc_qp_search_handles_undershoot_overshoot_and_limits() {
        let target = 50_000_000u64;
        // Already in the window: stop.
        assert_eq!(next_nvenc_qp(&[(24, 47_000_000)], target), None);
        // First pass far too small (simple content): lowers QP, never below the floor.
        let down = next_nvenc_qp(&[(20, 5_000_000)], target).unwrap();
        assert!(down < 20 && down >= NVENC_QP_MIN, "{down}");
        // First pass far too big: raises QP, never above the ceiling.
        let up = next_nvenc_qp(&[(20, 5_000_000_000)], target).unwrap();
        assert!(up > 20 && up <= NVENC_QP_MAX, "{up}");
        // Already at the ceiling and still too big: nothing more to try.
        assert_eq!(next_nvenc_qp(&[(NVENC_QP_MAX - 1, 900_000_000), (NVENC_QP_MAX, 800_000_000)], target), None);
        // Already at the floor and still too small: nothing more to try.
        assert_eq!(next_nvenc_qp(&[(NVENC_QP_MIN + 1, 1_000_000), (NVENC_QP_MIN, 2_000_000)], target), None);
        assert_eq!(next_nvenc_qp(&[], target), None);
    }

    #[test]
    fn compression_never_upscales() {
        let m = MediaMetadata {
            height: 480,
            duration: 60.0,
            ..Default::default()
        };
        assert_eq!(compression_plan(&m, 100.0).output_height, 480);
    }

    #[test]
    fn compression_hardware_recipes_balance_quality_and_low_end_load() {
        let nvenc = compression_hardware_recipe("h264_nvenc", 4_000_000);
        let qsv = compression_hardware_recipe("h264_qsv", 4_000_000);
        let amf = compression_hardware_recipe("h264_amf", 4_000_000);
        let has = |args: &[String], key: &str, value: &str| {
            args.windows(2)
                .any(|pair| pair[0] == key && pair[1] == value)
        };
        assert!(has(&nvenc.args, "-preset", "p5"));
        assert!(has(&nvenc.args, "-tune", "hq"));
        assert!(has(&nvenc.args, "-multipass", "qres"));
        assert!(has(&nvenc.args, "-rc", "constqp"));
        assert!(has(&nvenc.args, "-qp", "20"));
        assert!(!nvenc.args.iter().any(|value| value == "-b:v"));
        assert_eq!(nvenc.nvenc_qp, Some(20));
        assert!(!nvenc.args.iter().any(|value| value == "p7"));
        assert!(has(&qsv.args, "-preset", "slow"));
        assert!(has(&amf.args, "-quality", "quality"));
        assert!(has(&amf.args, "-rc", "vbr_peak"));
        for recipe in [&nvenc, &qsv, &amf] {
            assert!(recipe.hardware);
        }
        for recipe in [&qsv, &amf] {
            assert!(has(&recipe.args, "-b:v", "4000000"));
            assert!(has(&recipe.args, "-maxrate", "5000000"));
        }
    }

    #[test]
    fn av1_hardware_recipes_cover_nvidia_intel_and_amd() {
        let nvenc = compression_hardware_recipe("av1_nvenc", 3_200_000);
        let qsv = compression_hardware_recipe("av1_qsv", 3_200_000);
        let amf = compression_hardware_recipe("av1_amf", 3_200_000);
        let has = |args: &[String], key: &str, value: &str| {
            args.windows(2)
                .any(|pair| pair[0] == key && pair[1] == value)
        };
        assert!(has(&nvenc.args, "-rc", "constqp"));
        assert!(has(&nvenc.args, "-qp", "20"));
        assert_eq!(nvenc.nvenc_qp, Some(20));
        assert!(!nvenc.args.iter().any(|value| value == "-b_ref_mode"));
        assert!(has(&qsv.args, "-preset", "slow"));
        assert!(has(&qsv.args, "-b:v", "3200000"));
        assert!(has(&amf.args, "-quality", "quality"));
        assert!(has(&amf.args, "-rc", "vbr_peak"));
    }

    #[test]
    fn av1_keeps_more_resolution_at_discord_bitrates_without_upscaling() {
        assert_eq!(compression_height_for_bitrate(3_200_000, false), 1080);
        assert_eq!(compression_height_for_bitrate(3_200_000, true), 1440);
        assert_eq!(compression_height_for_bitrate(2_287_919, false), 1080);
        assert_eq!(compression_height_for_bitrate(1_100_000, false), 720);
        assert_eq!(compression_height_for_bitrate(1_100_000, true), 900);
        assert_eq!(compression_height(2_287_919, false, 30.0), 1080);
        assert_eq!(compression_height(2_287_919, false, 60.0), 720);
        assert_eq!(compression_height(1_100_000, false, 60.0), 540);
        let source_height = 480;
        assert_eq!(
            source_height.min(compression_height_for_bitrate(8_000_000, true)),
            480
        );
    }

    #[test]
    fn thin_60fps_budgets_use_720p_but_nothing_else_changes() {
        // 60 fps, 2.4-3.2 Mbps: 720p60 instead of 900p60 (measured better at the same size).
        assert_eq!(compression_height(3_040_000, false, 60.0), 720);
        assert_eq!(compression_height(2_500_000, false, 60.0), 720);
        // More budget still earns 900p / 1080p, and the frame rate is never traded for this.
        assert_eq!(compression_height(3_400_000, false, 60.0), 900);
        assert_eq!(compression_height(4_200_000, false, 60.0), 1080);
        // 30 fps is untouched: a source that fits 1080p keeps it.
        assert_eq!(compression_height(3_040_000, false, 30.0), 1080);
        assert_eq!(compression_height(2_287_919, false, 30.0), 1080);
        // AV1 is more efficient and was not measured, so it keeps its own tiers.
        assert_eq!(compression_height(3_040_000, true, 60.0), 1080);
    }

    #[test]
    fn nvenc_bitrate_fallback_uses_vbr_at_the_requested_bitrate() {
        let args = nvenc_vbr_args("h264_nvenc", 473_000);
        let has = |k: &str, v: &str| args.windows(2).any(|p| p[0] == k && p[1] == v);
        assert!(has("-rc", "vbr") && has("-b:v", "473000") && has("-maxrate", "591250") && has("-bufsize", "946000"));
        assert!(!args.contains(&"-qp".to_string()), "must not pin a constant QP");
        assert!(nvenc_vbr_args("av1_nvenc", 473_000).iter().all(|a| a != "-bf"));
    }

    #[test]
    fn size_ceiling_uses_decimal_mb_with_margin_from_5_mb_only() {
        // Under 5 MB: untouched (1 MB = 1,048,576 bytes), no margin.
        assert_eq!(ceiling_bytes(1.0), 1_048_576);
        assert_eq!(ceiling_bytes(4.0), 4_194_304);
        // From 5 MB: 1,000,000-byte MB minus 1%, so it also fits services counting that way.
        assert_eq!(ceiling_bytes(5.0), 4_950_000);
        assert_eq!(ceiling_bytes(20.0), 19_800_000);
        assert_eq!(ceiling_bytes(50.0), 49_500_000);
        assert_eq!(ceiling_bytes(250.0), 247_500_000);
        for mb in [5.0, 8.0, 20.0, 50.0, 100.0, 250.0] {
            assert!(ceiling_bytes(mb) < (mb * 1_000_000.0) as u64, "{mb} MB must stay under {mb} decimal MB");
        }
        // Whole-MB picks never shrink the budget as the size grows.
        let mut last = 0;
        for mb in 1..=750 {
            let bytes = ceiling_bytes(mb as f64);
            assert!(bytes > last, "{mb} MB");
            last = bytes;
        }
    }

    #[test]
    fn refusal_below_150_kbps_follows_the_ceiling() {
        let m = meta(600.0, 0);
        let min = min_target_mb(&m);
        assert!(check_compression_target(&m, min).is_ok());
        assert!(check_compression_target(&m, min - 1.0).is_err());
        let msg = check_compression_target(&m, min - 1.0).unwrap_err();
        assert!(msg.contains("150 kbps") && msg.contains(&format!("{min}")), "{msg}");
    }

    #[test]
    fn every_encoder_choice_switches_to_the_cpu_below_1_mbps() {
        // macOS keeps VideoToolbox at low bitrates (see use_cpu_encoder); every other platform switches.
        let low_goes_cpu = !cfg!(target_os = "macos");
        for choice in ["auto", "nvenc", "qsv", "amf", "videotoolbox"] {
            assert_eq!(use_cpu_encoder(999_999, choice), low_goes_cpu, "{choice} below 1 Mbps");
            assert_eq!(use_cpu_encoder(190_000, choice), low_goes_cpu, "{choice} at 0.19 Mbps");
            assert!(!use_cpu_encoder(1_000_000, choice), "{choice} at 1 Mbps");
            assert!(!use_cpu_encoder(3_000_000, choice), "{choice} at 3 Mbps");
        }
        // "CPU" always means x264, at any bitrate.
        assert!(use_cpu_encoder(5_000_000, "software"));
        // The estimate reports it, and never promises AV1 on the CPU.
        let low = plan_compression(meta(600.0, 0), 20.0, true, "nvenc".into()).unwrap();
        assert_eq!(low.uses_cpu, low_goes_cpu);
        assert!(low.output_height <= 540, "{}p", low.output_height);
        let high = plan_compression(meta(300.0, 0), 100.0, false, "auto".into()).unwrap();
        assert!(!high.uses_cpu);
    }

    #[test]
    fn videotoolbox_compression_recipe_is_average_bitrate_with_a_size_ceiling() {
        let recipe = compression_hardware_recipe("h264_videotoolbox", 2_000_000);
        let has = |k: &str, v: &str| recipe.args.windows(2).any(|p| p[0] == k && p[1] == v);
        assert_eq!(recipe.name, "h264_videotoolbox");
        assert!(recipe.hardware && recipe.nvenc_qp.is_none());
        assert!(has("-c:v", "h264_videotoolbox") && has("-b:v", "2000000"));
        assert!(has("-maxrate", "2500000") && has("-bufsize", "4000000") && has("-pix_fmt", "yuv420p"));
        // Must not inherit another vendor's options.
        assert!(!recipe.args.iter().any(|a| a == "-rc" || a == "-quality" || a == "-preset" || a == "-qp"));
    }

    #[test]
    fn compression_falls_back_to_cpu_when_hardware_is_unavailable() {
        let encoder = automatic_compression_encoder(
            "/definitely/missing/ffmpeg",
            4_000_000,
            &meta(30.0, 4_000_000),
            "auto",
            "auto",
            false,
        );
        assert_eq!(encoder.name, "libx264");
        assert!(!encoder.hardware);
    }

    #[test]
    fn cpu_compression_reserves_capacity_on_lower_end_systems() {
        assert_eq!(cpu_compression_tuning(4), ("fast", 2));
        assert_eq!(cpu_compression_tuning(8), ("medium", 6));
        assert_eq!(cpu_compression_tuning(16), ("slow", 14));
    }

    #[test]
    fn windows_gpu_recipes_cover_nvidia_amd_and_intel_main10() {
        let metadata = MediaMetadata {
            bit_rate: 15_000_000,
            is_hdr: true,
            bit_depth: 10,
            color_transfer: "smpte2084".into(),
            color_primaries: "bt2020".into(),
            ..Default::default()
        };
        let nvenc = windows_encoder_args("hevc_nvenc", true, 90, "quality", &metadata);
        let amf = windows_encoder_args("hevc_amf", true, 90, "quality", &metadata);
        let qsv = windows_encoder_args("hevc_qsv", true, 90, "quality", &metadata);
        assert!(nvenc.contains(&"-qp".into()) && nvenc.contains(&"p7".into()));
        assert!(amf.contains(&"-qp_i".into()) && amf.contains(&"quality".into()));
        assert!(qsv.contains(&"-global_quality".into()) && qsv.contains(&"-look_ahead".into()));
        let value_after = |args: &[String], key: &str| {
            args.iter()
                .position(|value| value == key)
                .map(|index| args[index + 1].clone())
                .unwrap()
        };
        assert_eq!(value_after(&nvenc, "-rc"), "constqp");
        assert_eq!(value_after(&nvenc, "-qp"), "18");
        assert!(!nvenc.contains(&"-cq".into()));
        assert!(!nvenc.contains(&"-maxrate".into()));
        assert_eq!(value_after(&amf, "-qp_i"), "18");
        assert_eq!(value_after(&amf, "-qp_p"), "20");
        assert_eq!(value_after(&amf, "-qp_b"), "24");
        assert_eq!(value_after(&qsv, "-global_quality"), "18");
        for recipe in [&nvenc, &amf, &qsv] {
            assert!(recipe.contains(&"main10".into()));
            assert!(recipe.contains(&"p010le".into()));
        }
        assert!(amf.contains(&"-maxrate".into()));
        assert!(qsv.contains(&"-maxrate".into()));
    }

    #[test]
    fn windows_speed_menu_maps_to_each_vendor_native_preset() {
        let metadata = MediaMetadata::default();
        let nvenc = windows_encoder_args("h264_nvenc", false, 90, "fast", &metadata);
        let amf = windows_encoder_args("h264_amf", false, 90, "balanced", &metadata);
        let qsv = windows_encoder_args("h264_qsv", false, 90, "fast", &metadata);
        assert!(nvenc.windows(2).any(|pair| pair == ["-preset", "p4"]));
        assert!(amf.windows(2).any(|pair| pair == ["-quality", "balanced"]));
        assert!(qsv.windows(2).any(|pair| pair == ["-preset", "medium"]));
    }

    #[test]
    fn real_1080p_libass_preview_renders() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let video = root.join("test/quick-1080p/fake-10-second-video-1080p.mp4");
        let subtitles = root.join("test/quick-1080p/fake-crunchyroll-test.srt");
        if !video.exists() || !subtitles.exists() {
            return;
        }
        let tools =
            find_tools(None).expect("FFmpeg with libass is required for the real-media test");
        let metadata = probe_media(&tools.ffprobe, video.to_string_lossy().as_ref()).unwrap();
        assert_eq!((metadata.width, metadata.height), (1920, 1080));
        let settings = SubtitleSettings {
            delay_seconds: 0.0,
            position_percent: 93.0,
            scale_percent: 122.0,
            quality: 90.0,
            windows_encoder: default_windows_encoder(),
            windows_preset: default_windows_preset(),
        };
        let prepared = prepare_subtitle(
            &tools.ffmpeg,
            subtitles.to_string_lossy().as_ref(),
            &metadata,
            &settings,
        )
        .unwrap();
        let output = prepared._dir.path().join("rust-preview.png");
        let filter = format!(
            "format=rgba,{}",
            subtitle_filter(&prepared, 0.0, 3.5)
        );
        run_output(
            &tools.ffmpeg,
            &[
                "-hide_banner".into(),
                "-loglevel".into(),
                "error".into(),
                "-y".into(),
                "-f".into(),
                "lavfi".into(),
                "-i".into(),
                "color=c=black@0:s=1920x1080:r=1,format=rgba,settb=AVTB".into(),
                "-vf".into(),
                filter,
                "-frames:v".into(),
                "1".into(),
                "-c:v".into(),
                "png".into(),
                "-pix_fmt".into(),
                "rgba".into(),
                output.to_string_lossy().into_owned(),
            ],
        )
        .unwrap();
        assert!(fs::metadata(output).unwrap().len() > 10_000);
    }

    fn real_burn(
        source: &Path,
        subtitles: &Path,
        quality: f64,
    ) -> (MediaMetadata, MediaMetadata, u64) {
        let tools = find_tools(None).expect("FFmpeg with libass is required");
        let input = probe_media(&tools.ffprobe, source.to_string_lossy().as_ref()).unwrap();
        let settings = SubtitleSettings {
            delay_seconds: 0.0,
            position_percent: 93.0,
            scale_percent: 122.0,
            quality,
            windows_encoder: default_windows_encoder(),
            windows_preset: default_windows_preset(),
        };
        let prepared = prepare_subtitle(
            &tools.ffmpeg,
            subtitles.to_string_lossy().as_ref(),
            &input,
            &settings,
        )
        .unwrap();
        let c = canvas(&input);
        let ten = input.is_hdr || input.bit_depth > 8;
        let mut filters = Vec::new();
        if c.padded {
            filters.push(format!("pad={}:{}:0:0:black", c.width, c.height));
        }
        filters.push(
            if ten {
                "format=p010le"
            } else {
                "format=yuv420p"
            }
            .into(),
        );
        filters.push(subtitle_filter(&prepared, 0.0, 0.0));
        filters.push(
            if ten {
                "format=p010le"
            } else {
                "format=yuv420p"
            }
            .into(),
        );
        let (_, encoder, _) = encoder_args(&tools.ffmpeg, &input, &settings);
        let output = prepared._dir.path().join("fresh-rust-burn.mp4");
        let mut args = vec![
            "-hide_banner".into(),
            "-y".into(),
            "-noautorotate".into(),
            "-i".into(),
            source.to_string_lossy().into_owned(),
            "-map".into(),
            "0:V:0".into(),
            "-vf".into(),
            filters.join(","),
        ];
        args.extend(encoder);
        args.extend([
            "-an".into(),
            "-fps_mode".into(),
            "passthrough".into(),
            output.to_string_lossy().into_owned(),
        ]);
        run_output(&tools.ffmpeg, &args).unwrap();
        let output_metadata =
            probe_media(&tools.ffprobe, output.to_string_lossy().as_ref()).unwrap();
        let bytes = fs::metadata(output).unwrap().len();
        (input, output_metadata, bytes)
    }

    #[test]
    #[ignore = "explicit real VideoToolbox bitrate test"]
    fn real_1080p_bitrate_and_timing_are_preserved() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let (source, output, bytes) = real_burn(
            &root.join("test/quick-1080p/fake-10-second-video-1080p.mp4"),
            &root.join("test/quick-1080p/fake-crunchyroll-test.srt"),
            90.0,
        );
        assert_eq!((output.width, output.height), (1920, 1080));
        assert!((fps(&source.fps) - fps(&output.fps)).abs() < 0.01);
        println!(
            "1080p bitrate: source={:.3} Mbps output={:.3} Mbps ratio={:.1}% file={:.2} MiB",
            source.bit_rate as f64 / 1_000_000.0,
            output.bit_rate as f64 / 1_000_000.0,
            output.bit_rate as f64 / source.bit_rate as f64 * 100.0,
            bytes as f64 / 1_048_576.0
        );
        assert!(
            output.bit_rate >= (source.bit_rate as f64 * 0.65) as u64,
            "{} < 65% of {}",
            output.bit_rate,
            source.bit_rate
        );
        assert!(
            output.bit_rate <= (source.bit_rate as f64 * 1.50) as u64,
            "{} is runaway bitrate versus {}",
            output.bit_rate,
            source.bit_rate
        );
        assert!(bytes > 0);
    }

    #[test]
    #[ignore = "real 4K HDR exports require FFmpeg with libass, x265 and zscale"]
    fn real_4k_hdr_preserves_main10_for_burn_and_timing_for_compress() {
        let tools = tools_for(&AppState::default()).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.mp4");
        let video = source.to_str().unwrap();
        run_output(
            &tools.ffmpeg,
            &[
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=3840x2160:rate=24",
                "-frames:v",
                "12",
                "-c:v",
                "libx265",
                "-preset",
                "ultrafast",
                "-x265-params",
                "pools=4:frame-threads=1:colorprim=bt2020:transfer=smpte2084:colormatrix=bt2020nc",
                "-pix_fmt",
                "yuv420p10le",
                "-color_primaries",
                "bt2020",
                "-color_trc",
                "smpte2084",
                "-colorspace",
                "bt2020nc",
                "-tag:v",
                "hvc1",
                video,
            ]
            .map(str::to_owned),
        )
        .unwrap();
        let subtitle = dir.path().join("source.srt");
        fs::write(
            &subtitle,
            "1\n00:00:00,000 --> 00:00:01,000\nHDR subtitle\n",
        )
        .unwrap();
        let metadata = probe_media(&tools.ffprobe, video).unwrap();
        assert!(metadata.is_hdr && metadata.bit_depth >= 10, "{metadata:?}");
        let frames = frame_times(&tools, video);
        let active = Mutex::new(ExportJob::default());
        let mut encoders = vec!["software"];
        if cfg!(target_os = "macos") && encoder_usable(&tools.ffmpeg, "hevc_videotoolbox", true) {
            encoders.push("videotoolbox");
        }
        for encoder in encoders {
            let output_path = dir.path().join(format!("burn-{encoder}.mp4"));
            let result = burn_video(
                &tools,
                ExportInput {
                    video_path: video.into(),
                    subtitle_path: subtitle.to_string_lossy().into_owned(),
                    output_path: output_path.to_string_lossy().into_owned(),
                    metadata: Some(metadata.clone()),
                    settings: SubtitleSettings {
                        windows_encoder: encoder.into(),
                        quality: 90.0,
                        ..Default::default()
                    },
                    clip_start: 0.2,
                    clip_duration: 30.0,
                },
                &active,
                &|_| {},
            )
            .unwrap();
            let out = probe_media(&tools.ffprobe, &result.output).unwrap();
            assert_eq!((out.width, out.height), (3840, 2160));
            assert!(out.is_hdr && out.bit_depth >= 10, "{out:?}");
            assert_eq!(frame_times(&tools, &result.output), frames);
            if encoder == "videotoolbox" {
                assert_eq!(result.encoder, "hevc_videotoolbox");
            }
            println!(
                "4K HDR burn {}: {} video bps, {} frames",
                result.encoder,
                out.bit_rate,
                frames.len()
            );
        }
        let out = run_compression(
            &tools,
            &source,
            &dir.path().join("compressed.mp4"),
            2.0,
            (0.2, 30.0),
        )
        .unwrap();
        assert_eq!(frame_times(&tools, &out.output), frames);
        assert!(!out.metadata.is_hdr);
        assert!(out.bytes <= ceiling_bytes(2.0));
        println!(
            "4K HDR compression {}: {}x{}, {} video bps, {} bytes",
            out.encoder, out.metadata.width, out.metadata.height, out.metadata.bit_rate, out.bytes
        );
    }

    #[test]
    #[ignore = "explicit CQ 100 bitrate ceiling test"]
    fn real_cq100_avoids_runaway_bitrate() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let (source, output, bytes) = real_burn(
            &root.join("test/quick-1080p/fake-10-second-video-1080p.mp4"),
            &root.join("test/quick-1080p/fake-crunchyroll-test.srt"),
            100.0,
        );
        println!(
            "CQ100 bitrate: source={:.3} Mbps output={:.3} Mbps ratio={:.1}% file={:.2} MiB",
            source.bit_rate as f64 / 1_000_000.0,
            output.bit_rate as f64 / 1_000_000.0,
            output.bit_rate as f64 / source.bit_rate as f64 * 100.0,
            bytes as f64 / 1_048_576.0
        );
        assert!(output.bit_rate >= (source.bit_rate as f64 * 0.65) as u64);
        assert!(output.bit_rate <= (source.bit_rate as f64 * 1.60) as u64);

        let (hdr_source, hdr_output, hdr_bytes) = real_burn(
            &root.join("test/hdr-main10/source-4k-hdr10.mp4"),
            &root.join("test/hdr-main10/subtitles.srt"),
            100.0,
        );
        println!(
            "CQ100 4K HDR bitrate: source={:.3} Mbps output={:.3} Mbps ratio={:.1}% file={:.2} MiB",
            hdr_source.bit_rate as f64 / 1_000_000.0,
            hdr_output.bit_rate as f64 / 1_000_000.0,
            hdr_output.bit_rate as f64 / hdr_source.bit_rate as f64 * 100.0,
            hdr_bytes as f64 / 1_048_576.0
        );
        assert!(hdr_output.is_hdr && hdr_output.bit_depth >= 10);
        assert!(hdr_output.bit_rate >= (hdr_source.bit_rate as f64 * 0.65) as u64);
        assert!(hdr_output.bit_rate <= (hdr_source.bit_rate as f64 * 1.60) as u64);
    }
}
