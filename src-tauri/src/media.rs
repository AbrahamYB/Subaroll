use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::HashSet,
    fs,
    io::{BufRead, BufReader, Read},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{Arc, Mutex},
};
use tauri::{Emitter, State, WebviewWindow};
use tempfile::TempDir;

const VIDEO_EXTENSIONS: &[&str] = &[
    "mp4", "mkv", "mov", "avi", "webm", "m4v", "ts", "mts", "m2ts",
];
const SUBTITLE_EXTENSIONS: &[&str] = &["ass", "ssa", "srt", "vtt", "sub", "smi"];

#[derive(Default)]
pub struct AppState {
    active_pid: Arc<Mutex<Option<u32>>>,
    custom_ffmpeg: Mutex<Option<String>>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolStatus {
    ok: bool,
    ffmpeg: Option<String>,
    video_encoder: Option<String>,
    platform: String,
    available_video_encoders: Vec<String>,
    available_av1_encoders: Vec<String>,
    error: Option<String>,
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
    subtitle_path: String,
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
}

fn default_compression_codec() -> String {
    "auto".into()
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewResult {
    frame: String,
    subtitles: String,
    font_count: usize,
    layout: PreviewLayout,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewLayout {
    play_res_y: f64,
    line_count: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportResult {
    output: String,
    font_count: usize,
    encoder: String,
    quality: i64,
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
    output_fps: f64,
}

fn command_ok(command: &str, args: &[&str]) -> bool {
    Command::new(command)
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn command_text(command: &str, args: &[&str]) -> String {
    Command::new(command)
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
    Command::new(ffmpeg)
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "color=c=black:s=64x64:r=1",
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
    if let Ok(value) = std::env::var("SUBAROLL_FFMPEG").or_else(|_| std::env::var("SUBTITLE_BURNER_FFMPEG")) {
        candidates.push(value);
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
                windows_hardware_encoder(&ffmpeg, false, "auto").unwrap_or("libx264")
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
    let custom = state
        .custom_ffmpeg
        .lock()
        .map_err(|_| "Tool state is unavailable")?
        .clone();
    find_tools(custom).ok_or_else(|| "FFmpeg with libass is required.".into())
}

#[tauri::command]
pub fn tool_status(state: State<AppState>) -> ToolStatus {
    match tools_for(&state) {
        Ok(t) => {
            let available_video_encoders = if cfg!(target_os = "windows") {
                available_windows_encoders(&t.ffmpeg)
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
        *state.custom_ffmpeg.lock().unwrap() = Some(path.to_string_lossy().into_owned());
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
    let output = Command::new(command)
        .args(args)
        .output()
        .map_err(|e| e.to_string())?;
    if output.status.success() {
        Ok(output.stdout)
    } else {
        Err(last_lines(&String::from_utf8_lossy(&output.stderr)))
    }
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
        .find(|s| s["codec_type"] == "video")
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
            if v.is_empty() {
                s(&video["r_frame_rate"])
            } else {
                v
            }
        },
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
    let text = fs::read_to_string(subtitle_path).map_err(|e| e.to_string())?;
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
    ass_text: String,
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
            if !f[ti].contains("\\pos") && !f[ti].contains("\\move") {
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
            if dialogue_styles.contains(f[si].trim()) {
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
    let source = fs::read_to_string(&converted).map_err(|e| e.to_string())?;
    let c = canvas(metadata);
    let result = transform_ass(
        &source,
        settings,
        !styled,
        c.width as f64 / c.height.max(1) as f64,
    );
    fs::write(&ass, &result).map_err(|e| e.to_string())?;
    let mut count = 0;
    if let Some(parent) = Path::new(subtitle).parent() {
        if let Ok(entries) = fs::read_dir(parent) {
            for e in entries.flatten() {
                let p = e.path();
                if p.extension()
                    .and_then(|x| x.to_str())
                    .map(|x| matches!(x.to_lowercase().as_str(), "ttf" | "otf" | "ttc" | "otc"))
                    .unwrap_or(false)
                {
                    if fs::copy(&p, fonts.join(p.file_name().unwrap())).is_ok() {
                        count += 1
                    }
                }
            }
        }
    }
    Ok(Prepared {
        _dir: dir,
        ass,
        fonts,
        font_count: count,
        ass_text: result,
    })
}

fn subtitle_filter(
    p: &Prepared,
    delay: f64,
    preview: Option<f64>,
    alpha: bool,
    timeline: f64,
) -> String {
    let subs = format!(
        "subtitles=filename='{}':fontsdir='{}'{}",
        escape_filter_path(&p.ass),
        escape_filter_path(&p.fonts),
        if alpha { ":alpha=1" } else { "" }
    );
    if let Some(t) = preview {
        return format!("setpts=PTS+{}/TB,{}", fmt((t - delay).max(0.0)), subs);
    }
    let offset = timeline.max(0.0) - delay;
    if offset != 0.0 {
        format!(
            "setpts=PTS{sign}{n}/TB,{subs},setpts=PTS{back}{n}/TB",
            sign = if offset < 0.0 { "-" } else { "+" },
            back = if offset < 0.0 { "+" } else { "-" },
            n = fmt(offset.abs())
        )
    } else {
        subs
    }
}
fn fps(v: &str) -> f64 {
    ratio(v, v.parse().unwrap_or(25.0)).max(0.001)
}
fn preview_time(m: &MediaMetadata, t: f64) -> f64 {
    t.max(0.0)
        .min((m.duration - (0.1f64).max(2.0 / fps(&m.fps))).max(0.0))
}
fn ass_time(v: &str) -> f64 {
    let p: Vec<_> = v.trim().split([':', '.']).collect();
    if p.len() < 3 {
        return 0.0;
    }
    p[0].parse::<f64>().unwrap_or(0.0) * 3600.0
        + p[1].parse::<f64>().unwrap_or(0.0) * 60.0
        + p[2].parse::<f64>().unwrap_or(0.0)
        + p.get(3)
            .and_then(|x| format!("0.{x}").parse().ok())
            .unwrap_or(0.0)
}
fn active_lines(text: &str, time: f64) -> usize {
    let mut section = "".to_owned();
    let mut format = Vec::<String>::new();
    let mut max = 1;
    for line in text.lines() {
        if line.starts_with('[') && line.ends_with(']') {
            section = line[1..line.len() - 1].to_lowercase()
        }
        if section != "events" {
            continue;
        }
        if line.to_lowercase().starts_with("format:") {
            format = line[7..]
                .split(',')
                .map(|x| x.trim().to_lowercase())
                .collect()
        }
        if line.to_lowercase().starts_with("dialogue:") && !format.is_empty() {
            let f = split_row(&line[9..], format.len());
            let at = |n: &str| format.iter().position(|x| x == n).unwrap_or(0);
            if time >= ass_time(&f[at("start")]) && time < ass_time(&f[at("end")]) {
                max = max.max(f[at("text")].to_lowercase().matches("\\n").count() + 1)
            }
        }
    }
    max
}

#[tauri::command]
pub async fn render_preview(
    state: State<'_, AppState>,
    input: PreviewInput,
) -> Result<PreviewResult, String> {
    let tools = tools_for(&state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let m = match input.metadata {
            Some(m) => m,
            None => probe_media(&tools.ffprobe, &input.video_path)?,
        };
        let c = canvas(&m);
        let p = prepare_subtitle(&tools.ffmpeg, &input.subtitle_path, &m, &input.settings)?;
        let frame = p._dir.path().join("frame.png");
        let subs = p._dir.path().join("subtitles.png");
        let time = preview_time(&m, input.time);
        let mut ff = Vec::<String>::new();
        if c.padded {
            ff.push(format!("pad={}:{}:0:0:black", c.width, c.height))
        }
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
        ff.push("format=rgb24".into());
        run_output(
            &tools.ffmpeg,
            &[
                "-hide_banner",
                "-loglevel",
                "error",
                "-y",
                "-ss",
                &fmt(time),
                "-noautorotate",
                "-i",
                &input.video_path,
                "-vf",
                &ff.join(","),
                "-frames:v",
                "1",
                "-c:v",
                "png",
                "-pix_fmt",
                "rgb24",
                frame.to_string_lossy().as_ref(),
            ]
            .into_iter()
            .map(|x| x.to_owned())
            .collect::<Vec<_>>(),
        )?;
        let filter = format!(
            "format=rgba,{}",
            subtitle_filter(&p, input.settings.delay_seconds, Some(time), true, 0.0)
        );
        run_output(
            &tools.ffmpeg,
            &[
                "-hide_banner",
                "-loglevel",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
                &format!(
                    "color=c=black@0:s={}x{}:r=1,format=rgba,settb=AVTB",
                    c.width, c.height
                ),
                "-vf",
                &filter,
                "-frames:v",
                "1",
                "-c:v",
                "png",
                "-pix_fmt",
                "rgba",
                subs.to_string_lossy().as_ref(),
            ]
            .into_iter()
            .map(|x| x.to_owned())
            .collect::<Vec<_>>(),
        )?;
        let play = Regex::new(r"(?im)^PlayResY:\s*([\d.]+)")
            .unwrap()
            .captures(&p.ass_text)
            .and_then(|x| x[1].parse().ok())
            .unwrap_or(360.0);
        let line_count = active_lines(&p.ass_text, (time - input.settings.delay_seconds).max(0.0));
        Ok(PreviewResult {
            frame: format!(
                "data:image/png;base64,{}",
                BASE64.encode(fs::read(frame).map_err(|e| e.to_string())?)
            ),
            subtitles: format!(
                "data:image/png;base64,{}",
                BASE64.encode(fs::read(subs).map_err(|e| e.to_string())?)
            ),
            font_count: p.font_count,
            layout: PreviewLayout {
                play_res_y: play,
                line_count,
            },
        })
    })
    .await
    .map_err(|e| e.to_string())?
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

fn windows_encoder_args(
    hardware: &str,
    ten_bit: bool,
    quality: i64,
    preset: &str,
    m: &MediaMetadata,
) -> Vec<String> {
    // CQ 90 maps to CTT/Smoothie's high-quality values: NVENC CQ 18,
    // AMF HEVC 18/20/24, and QSV HEVC global_quality 18.
    let cq = (40.0 - quality as f64 * 0.244).round().clamp(14.0, 26.0) as i64;
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
            "vbr".into(),
            "-cq".into(),
            cq.to_string(),
            "-b:v".into(),
            "0".into(),
            "-spatial-aq".into(),
            "1".into(),
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
            if ten_bit {
                cq.to_string()
            } else {
                (cq - 2).max(0).to_string()
            },
            "-qp_p".into(),
            if ten_bit {
                (cq + 2).to_string()
            } else {
                cq.to_string()
            },
            "-qp_b".into(),
            if ten_bit {
                (cq + 6).to_string()
            } else {
                (cq + 4).to_string()
            },
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
                cq.to_string()
            } else {
                (cq - 3).max(1).to_string()
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
    args.extend(source_aware_ceiling_args(m, quality));
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
    if cfg!(target_os = "macos") && supports_encoder(ffmpeg, "hevc_videotoolbox") {
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
    } else if cfg!(target_os = "windows") {
        if settings.windows_encoder != "software" {
            if let Some(hardware) = windows_hardware_encoder(ffmpeg, ten, &settings.windows_encoder)
            {
                a = windows_encoder_args(hardware, ten, quality, &settings.windows_preset, m);
                return (hardware.into(), a, ten);
            }
        }
        if ten && supports_encoder(ffmpeg, "libx265") {
            let crf = (47.0 - quality as f64 * 0.3).round() as i64;
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
        let crf = (43.0 - quality as f64 * 0.3).round() as i64;
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
        let crf = (41.0 - quality as f64 * 0.3).round() as i64;
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
        let crf = (41.0 - quality as f64 * 0.3).round() as i64;
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

fn run_progress(
    command: &str,
    args: &[String],
    duration: f64,
    base: f64,
    span: f64,
    window: &WebviewWindow,
    active: &Arc<Mutex<Option<u32>>>,
) -> Result<(), String> {
    let mut child = Command::new(command)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;
    *active.lock().unwrap() = Some(child.id());
    #[cfg(target_os = "macos")]
    let _awake = Command::new("caffeinate")
        .args(["-i", "-w", &child.id().to_string()])
        .spawn()
        .ok();
    let stderr = child.stderr.take().unwrap();
    let err_thread = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = BufReader::new(stderr).read_to_string(&mut s);
        s
    });
    if let Some(stdout) = child.stdout.take() {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if let Some(v) = line
                .strip_prefix("out_time_us=")
                .or_else(|| line.strip_prefix("out_time_ms="))
            {
                if let Ok(us) = v.parse::<f64>() {
                    let _ = window.emit(
                        "export-progress",
                        base + (us / 1_000_000.0 / duration.max(0.1)).clamp(0.0, 1.0) * span,
                    );
                }
            }
        }
    }
    let status = child.wait().map_err(|e| e.to_string())?;
    *active.lock().unwrap() = None;
    let stderr = err_thread.join().unwrap_or_default();
    if status.success() {
        Ok(())
    } else {
        Err(last_lines(&stderr))
    }
}

#[tauri::command]
pub async fn start_export(
    window: WebviewWindow,
    state: State<'_, AppState>,
    input: ExportInput,
) -> Result<ExportResult, String> {
    if state.active_pid.lock().unwrap().is_some() {
        return Err("An export is already running.".into());
    }
    let tools = tools_for(&state)?;
    let active = state.active_pid.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let m = match input.metadata {
            Some(m) => m,
            None => probe_media(&tools.ffprobe, &input.video_path)?,
        };
        let c = canvas(&m);
        let p = prepare_subtitle(&tools.ffmpeg, &input.subtitle_path, &m, &input.settings)?;
        let start = input.clip_start.max(0.0).min(m.duration);
        let duration = if input.clip_duration > 0.0 {
            input.clip_duration.max(0.1).min(m.duration - start)
        } else {
            0.0
        };
        let burn = subtitle_filter(&p, input.settings.delay_seconds, None, false, start);
        let ten = m.is_hdr || m.bit_depth > 8;
        let mut filters = Vec::new();
        if c.padded {
            filters.push(format!("pad={}:{}:0:0:black", c.width, c.height))
        }
        filters.push(
            if ten {
                "format=p010le"
            } else {
                "format=yuv420p"
            }
            .into(),
        );
        filters.push(burn);
        filters.push(
            if ten {
                "format=p010le"
            } else {
                "format=yuv420p"
            }
            .into(),
        );
        let (encoder, enc, ten_bit) = encoder_args(&tools.ffmpeg, &m, &input.settings);
        let ext = Path::new(&input.output_path)
            .extension()
            .and_then(|x| x.to_str())
            .unwrap_or("")
            .to_lowercase();
        let copy = ext == "mkv"
            || m.audio_streams
                .iter()
                .all(|a| matches!(a.codec.as_str(), "aac" | "ac3" | "eac3" | "mp3" | "alac"));
        let mut args = vec!["-hide_banner".into(), "-y".into()];
        if start > 0.0 {
            args.extend(["-ss".into(), fmt(start)])
        }
        args.extend([
            "-noautorotate".into(),
            "-i".into(),
            input.video_path.clone(),
            "-map".into(),
            "0:v:0".into(),
            "-map".into(),
            "0:a?".into(),
            "-vf".into(),
            filters.join(","),
        ]);
        args.extend(enc);
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
            args.extend(["-t".into(), fmt(duration)])
        }
        args.extend([
            "-fps_mode".into(),
            "passthrough".into(),
            "-progress".into(),
            "pipe:1".into(),
            "-nostats".into(),
            input.output_path.clone(),
        ]);
        run_progress(
            &tools.ffmpeg,
            &args,
            if duration > 0.0 { duration } else { m.duration },
            0.0,
            1.0,
            &window,
            &active,
        )?;
        let out = probe_media(&tools.ffprobe, &input.output_path)?;
        if out.width != c.width || out.height != c.height {
            return Err(format!(
                "Export safety check failed: expected {}×{}, but output is {}×{}.",
                c.width, c.height, out.width, out.height
            ));
        }
        if m.is_hdr && !out.is_hdr {
            return Err(
                "Export safety check failed: HDR color signaling was not preserved.".into(),
            );
        }
        if m.bit_depth > 8 && out.bit_depth < 10 {
            return Err(
                "Export safety check failed: the 10-bit source was reduced to 8-bit.".into(),
            );
        }
        if (fps(&m.fps) - fps(&out.fps)).abs() > 0.01 {
            return Err(format!(
                "Export safety check failed: source is {:.3} fps, but output is {:.3} fps.",
                fps(&m.fps), fps(&out.fps)
            ));
        }
        let expected_aspect = c.width as f64 / c.height.max(1) as f64;
        if (out.display_aspect - expected_aspect).abs() > 0.001 {
            return Err("Export safety check failed: the output display aspect ratio changed.".into());
        }
        if out.rotation != m.rotation {
            return Err("Export safety check failed: the output orientation changed.".into());
        }
        if input.settings.quality >= 88.0 && m.bit_rate > 0 && out.bit_rate > 0
            && out.bit_rate < (m.bit_rate as f64 * 0.65) as u64
        {
            return Err(format!(
                "Export safety check failed: encoder bitrate undershot the source ({:.1} vs {:.1} Mbps).",
                out.bit_rate as f64 / 1_000_000.0,
                m.bit_rate as f64 / 1_000_000.0
            ));
        }
        let _ = window.emit("export-progress", 1.0);
        Ok(ExportResult {
            output: input.output_path,
            font_count: p.font_count,
            encoder,
            quality: input.settings.quality.round() as i64,
            hdr: m.is_hdr,
            ten_bit,
            dolby_vision_converted: m.dolby_vision,
            clip_start: start,
            clip_duration: duration,
            audio_copied: copy,
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

fn compression_plan(m: &MediaMetadata, mb: f64) -> CompressionPlan {
    let target = (mb.clamp(1.0, 10000.0) * 1024.0 * 1024.0) as u64;
    let total = ((target as f64 * 8.0 / m.duration.max(0.1)) * 0.96) as u64;
    let audio = if total >= 1_500_000 {
        128_000
    } else if total >= 700_000 {
        96_000
    } else {
        64_000
    };
    let video = total.saturating_sub(audio).max(120_000);
    let (max_h, output_fps) = compression_geometry(video, false, fps(&m.fps));
    CompressionPlan {
        target_bytes: target,
        total_bitrate: total,
        video_bitrate: video,
        audio_bitrate: audio,
        output_height: m.height.min(max_h),
        output_fps,
    }
}

fn compression_height_for_bitrate(video_bitrate: u64, av1: bool) -> u64 {
    if av1 {
        if video_bitrate >= 1_200_000 {
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

fn compression_geometry(video_bitrate: u64, av1: bool, source_fps: f64) -> (u64, f64) {
    let source_fps = source_fps.max(1.0);
    let rate_at_source_fps = (video_bitrate as f64 * (30.0 / source_fps).min(1.0)).round() as u64;
    let height_at_source_fps = compression_height_for_bitrate(rate_at_source_fps, av1);
    if source_fps > 30.0 && height_at_source_fps < 720 {
        let reduced_fps = (source_fps / 2.0).min(30.0);
        (
            compression_height_for_bitrate(video_bitrate, av1),
            reduced_fps,
        )
    } else {
        (height_at_source_fps, source_fps)
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
    Command::new(ffmpeg)
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
    width: u64,
    height: u64,
    codec_preference: &str,
) -> CompressionEncoder {
    if cfg!(target_os = "windows") {
        if codec_preference != "h264" {
            for encoder in ["av1_nvenc", "av1_qsv", "av1_amf"] {
                if supports_encoder(ffmpeg, encoder) {
                    let recipe = compression_hardware_recipe(encoder, video_bitrate);
                    if compression_recipe_usable(ffmpeg, &recipe, width, height) {
                        return recipe;
                    }
                }
            }
        }
        for encoder in ["h264_nvenc", "h264_qsv", "h264_amf"] {
            if supports_encoder(ffmpeg, encoder) {
                let recipe = compression_hardware_recipe(encoder, video_bitrate);
                if compression_recipe_usable(ffmpeg, &recipe, width, height) {
                    return recipe;
                }
            }
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

#[tauri::command]
pub async fn start_compression(
    window: WebviewWindow,
    state: State<'_, AppState>,
    input: CompressionInput,
) -> Result<CompressionResult, String> {
    if state.active_pid.lock().unwrap().is_some() {
        return Err("An export is already running.".into());
    }
    let tools = tools_for(&state)?;
    let active = state.active_pid.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let m = match input.metadata {
            Some(m) => m,
            None => probe_media(&tools.ffprobe, &input.video_path)?,
        };
        let mut plan = compression_plan(&m, input.target_size_mb);
        let probe_height = m.height.min(1080);
        let probe_width =
            ((m.width as f64 * probe_height as f64 / m.height.max(1) as f64).round() as u64).max(2)
                & !1;
        let mut encoder = automatic_compression_encoder(
            &tools.ffmpeg,
            plan.video_bitrate,
            probe_width,
            probe_height,
            &input.codec_preference,
        );
        if encoder.name.starts_with("av1_") {
            let (max_height, output_fps) =
                compression_geometry(plan.video_bitrate, true, fps(&m.fps));
            plan.output_height = m.height.min(max_height);
            plan.output_fps = output_fps;
        }
        let dir = tempfile::Builder::new()
            .prefix("SubarollCompress-")
            .tempdir()
            .map_err(|e| e.to_string())?;
        let pass = dir.path().join("pass");
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
            )
        }
        if plan.output_height < m.height {
            filters.push(format!("scale=-2:{}:flags=lanczos", plan.output_height))
        }
        if plan.output_fps < fps(&m.fps) - 0.01 {
            filters.push(format!("fps={}", fmt(plan.output_fps)))
        }
        filters.push("format=yuv420p".into());
        let mut common = vec![
            "-hide_banner".into(),
            "-y".into(),
            "-noautorotate".into(),
            "-i".into(),
            input.video_path.clone(),
            "-map".into(),
            "0:v:0".into(),
            "-vf".into(),
            filters.join(","),
        ];
        let common_base = common.clone();
        let mut final_args = if encoder.hardware {
            common.extend(encoder.args.clone());
            common
        } else {
            let logical_cores = std::thread::available_parallelism()
                .map(|value| value.get())
                .unwrap_or(4);
            let (cpu_preset, cpu_threads) = cpu_compression_tuning(logical_cores);
            common.extend([
                "-c:v".into(),
                "libx264".into(),
                "-preset".into(),
                cpu_preset.into(),
                "-threads".into(),
                cpu_threads.to_string(),
                "-b:v".into(),
                plan.video_bitrate.to_string(),
                "-maxrate".into(),
                ((plan.video_bitrate as f64 * 1.25) as u64).to_string(),
                "-bufsize".into(),
                (plan.video_bitrate * 2).to_string(),
                "-passlogfile".into(),
                pass.to_string_lossy().into_owned(),
            ]);
            let mut p1 = common.clone();
            p1.extend([
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
            run_progress(&tools.ffmpeg, &p1, m.duration, 0.0, 0.5, &window, &active)?;
            common.extend(["-pass".into(), "2".into()]);
            common
        };
        let append_output = |args: &mut Vec<String>| {
            args.extend([
                "-map".into(),
                "0:a:0?".into(),
                "-c:a".into(),
                "aac".into(),
                "-b:a".into(),
                plan.audio_bitrate.to_string(),
                "-map_metadata".into(),
                "0".into(),
                "-map_chapters".into(),
                "0".into(),
                "-movflags".into(),
                "+faststart".into(),
                "-progress".into(),
                "pipe:1".into(),
                "-nostats".into(),
                input.output_path.clone(),
            ]);
        };
        append_output(&mut final_args);
        run_progress(
            &tools.ffmpeg,
            &final_args,
            m.duration,
            if encoder.hardware { 0.0 } else { 0.5 },
            if encoder.name.ends_with("_nvenc") {
                0.7
            } else if encoder.hardware {
                1.0
            } else {
                0.5
            },
            &window,
            &active,
        )?;
        let mut bytes = fs::metadata(&input.output_path)
            .map_err(|e| e.to_string())?
            .len();
        if encoder.name.ends_with("_nvenc") && bytes as f64 > plan.target_bytes as f64 * 1.03 {
            let ratio = bytes as f64 / plan.target_bytes as f64;
            let qp_increase = (ratio.log2() * 6.0).ceil().max(1.0) as u64 + 1;
            let corrected_qp = (encoder.nvenc_qp.unwrap_or(20) + qp_increase).min(32);
            let encoder_name = encoder.name.clone();
            encoder = compression_hardware_recipe_with_qp(
                &encoder_name,
                plan.video_bitrate,
                corrected_qp,
            );
            let mut retry_args = common_base;
            retry_args.extend(encoder.args.clone());
            append_output(&mut retry_args);
            run_progress(
                &tools.ffmpeg,
                &retry_args,
                m.duration,
                0.7,
                0.3,
                &window,
                &active,
            )?;
            bytes = fs::metadata(&input.output_path)
                .map_err(|e| e.to_string())?
                .len();
        }
        if bytes as f64 > plan.target_bytes as f64 * 1.03 {
            return Err(format!(
                "Compressed file exceeded the {} MB target.",
                input.target_size_mb
            ));
        }
        let metadata = probe_media(&tools.ffprobe, &input.output_path)?;
        let _ = window.emit("export-progress", 1.0);
        Ok(CompressionResult {
            output: input.output_path,
            bytes,
            plan,
            metadata,
            encoder: encoder.name,
            hardware: encoder.hardware,
            nvenc_qp: encoder.nvenc_qp,
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn cancel_export(state: State<AppState>) -> bool {
    let pid = *state.active_pid.lock().unwrap();
    if let Some(pid) = pid {
        #[cfg(unix)]
        unsafe {
            libc::kill(pid as i32, libc::SIGTERM);
        }
        #[cfg(windows)]
        {
            let _ = Command::new("taskkill")
                .args(["/PID", &pid.to_string(), "/T", "/F"])
                .status();
        }
        true
    } else {
        false
    }
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

#[cfg(test)]
mod tests {
    use super::*;
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
        assert_eq!(compression_height_for_bitrate(3_200_000, true), 1080);
        assert_eq!(compression_height_for_bitrate(2_287_919, false), 1080);
        assert_eq!(compression_height_for_bitrate(1_100_000, false), 720);
        assert_eq!(compression_height_for_bitrate(1_100_000, true), 900);
        assert_eq!(compression_geometry(2_287_919, false, 30.0), (1080, 30.0));
        assert_eq!(compression_geometry(2_287_919, false, 60.0), (720, 60.0));
        assert_eq!(compression_geometry(1_100_000, false, 60.0), (720, 30.0));
        let source_height = 480;
        assert_eq!(
            source_height.min(compression_height_for_bitrate(8_000_000, true)),
            480
        );
    }

    #[test]
    fn compression_falls_back_to_cpu_when_hardware_is_unavailable() {
        let encoder = automatic_compression_encoder(
            "/definitely/missing/ffmpeg",
            4_000_000,
            1920,
            1080,
            "auto",
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
        assert!(nvenc.contains(&"-cq".into()) && nvenc.contains(&"p7".into()));
        assert!(amf.contains(&"-qp_i".into()) && amf.contains(&"quality".into()));
        assert!(qsv.contains(&"-global_quality".into()) && qsv.contains(&"-look_ahead".into()));
        let value_after = |args: &[String], key: &str| {
            args.iter()
                .position(|value| value == key)
                .map(|index| args[index + 1].clone())
                .unwrap()
        };
        assert_eq!(value_after(&nvenc, "-cq"), "18");
        assert_eq!(value_after(&amf, "-qp_i"), "18");
        assert_eq!(value_after(&amf, "-qp_p"), "20");
        assert_eq!(value_after(&amf, "-qp_b"), "24");
        assert_eq!(value_after(&qsv, "-global_quality"), "18");
        for recipe in [&nvenc, &amf, &qsv] {
            assert!(recipe.contains(&"main10".into()));
            assert!(recipe.contains(&"p010le".into()));
            assert!(recipe.contains(&"-maxrate".into()));
        }
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
            subtitle_filter(&prepared, 0.0, Some(3.5), true, 0.0)
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
        filters.push(subtitle_filter(&prepared, 0.0, None, false, 0.0));
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
            "0:v:0".into(),
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
    #[ignore = "explicit real 4K HDR Main10 test"]
    fn real_4k_hdr_main10_and_bitrate_are_preserved() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let (source, output, bytes) = real_burn(
            &root.join("test/hdr-main10/source-4k-hdr10.mp4"),
            &root.join("test/hdr-main10/subtitles.srt"),
            90.0,
        );
        assert_eq!((output.width, output.height), (3840, 2160));
        assert!(output.is_hdr && output.bit_depth >= 10);
        assert!((fps(&source.fps) - fps(&output.fps)).abs() < 0.01);
        println!(
            "4K HDR bitrate: source={:.3} Mbps output={:.3} Mbps ratio={:.1}% file={:.2} MiB",
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
