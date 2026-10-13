//! Offline evaluation harness behind `--manifest` (phase F0a of
//! `docs/ULTRAPLAN.md`): runs a list of WAVs through the app's own VAD, batch
//! transcription and post-ASR text stages, and prints one JSON line per
//! utterance plus a summary line for metrics and baselines to consume.
//!
//! Also holds the WAV and model checks the single-file `--transcribe-file`
//! path shares with it.

use crate::actions::maybe_convert_chinese_variant;
use crate::audio_toolkit::{read_wav_samples, OfflineVad};
use crate::cli::{CliArgs, VadMode};
use crate::managers::audio::{build_smoothed_vad, pad_short_recording};
use crate::managers::transcription::{TextStage, TranscriptionDetail, TranscriptionManager};
use crate::settings::{get_settings, AppSettings, VadBackend};
use serde_json::json;
use std::collections::HashSet;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;
use tauri::{AppHandle, Manager};

const SAMPLE_RATE: f64 = 16_000.0;

/// Reject anything but the 16 kHz mono 16-bit PCM WAV the app records:
/// `read_wav_samples` does no validation, so other formats would be
/// mis-timed or mis-decoded instead of failing.
pub(crate) fn check_wav_format(path: &Path) -> Result<(), String> {
    let reader = hound::WavReader::open(path)
        .map_err(|e| format!("cannot open {}: {}", path.display(), e))?;
    let spec = reader.spec();
    if spec.sample_rate != 16_000
        || spec.channels != 1
        || spec.bits_per_sample != 16
        || spec.sample_format != hound::SampleFormat::Int
    {
        return Err(format!(
            "expected 16 kHz mono 16-bit PCM WAV, got {} Hz / {} ch / {}-bit {:?}",
            spec.sample_rate, spec.channels, spec.bits_per_sample, spec.sample_format
        ));
    }
    Ok(())
}

/// The model a headless run transcribes with, after a timed cold load.
pub(crate) struct LoadedModel {
    pub id: String,
    pub requested_device: String,
    pub load_ms: u64,
    pub bound_backend: Option<String>,
}

/// Load `--model` (default: `selected_model`) on `--device-index` (default:
/// the persisted accelerator setting). On failure, prints the error and
/// returns the exit code: 2 when no model is selected, 1 when loading fails.
pub(crate) fn load_model(
    tm: &TranscriptionManager,
    args: &CliArgs,
    selected_model: &str,
) -> Result<LoadedModel, i32> {
    let id = args
        .model
        .clone()
        .unwrap_or_else(|| selected_model.to_string());
    if id.is_empty() {
        eprintln!("error: no model selected (pass --model or pick one in the app)");
        return Err(2);
    }

    // --device-index hard-selects a compute device by its --list-devices registry
    // index (transcribe-cpp / whisper-family models only; not persisted). Omit it
    // to use the persisted accelerator setting.
    let requested_device = match args.device_index {
        Some(idx) => format!("index {}", idx),
        None => "settings".to_string(),
    };

    let load_start = Instant::now();
    if let Err(e) = tm.load_model_with_device(&id, args.device_index) {
        eprintln!("error: load_model('{}') failed: {}", id, e);
        return Err(1);
    }
    Ok(LoadedModel {
        load_ms: load_start.elapsed().as_millis() as u64,
        bound_backend: tm.current_backend(),
        id,
        requested_device,
    })
}

/// With the model unload timeout set to "Immediately", `transcribe` unloads the
/// engine after every run; reload it (untimed) so the next run works and its
/// timing stays clean.
pub(crate) fn ensure_loaded(
    tm: &TranscriptionManager,
    model: &LoadedModel,
    device_index: Option<usize>,
) -> anyhow::Result<()> {
    if !tm.is_model_loaded() {
        tm.load_model_with_device(&model.id, device_index)?;
    }
    Ok(())
}

/// One utterance listed in a manifest.
#[derive(Debug, PartialEq)]
pub(crate) struct ManifestEntry {
    pub id: String,
    pub audio: PathBuf,
}

/// Parse a JSONL manifest: one object per non-blank line with `audio` (a WAV
/// path, resolved against `base_dir` when relative) and an optional `id`
/// (default: the `audio` value). Ids must be unique so results can be joined
/// back to the manifest; any other fields (reference texts, tags) are ignored.
pub(crate) fn parse_manifest(text: &str, base_dir: &Path) -> Result<Vec<ManifestEntry>, String> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut entries = Vec::new();
    let mut seen = HashSet::new();
    for (index, line) in text.lines().enumerate() {
        let line_no = index + 1;
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let value: serde_json::Value =
            serde_json::from_str(line).map_err(|e| format!("line {line_no}: invalid JSON: {e}"))?;
        let audio = value
            .get("audio")
            .and_then(serde_json::Value::as_str)
            .filter(|audio| !audio.is_empty())
            .ok_or_else(|| format!("line {line_no}: missing string field \"audio\""))?;
        let id = match value.get("id") {
            None => audio.to_string(),
            Some(serde_json::Value::String(id)) if !id.is_empty() => id.clone(),
            Some(_) => return Err(format!("line {line_no}: \"id\" must be a non-empty string")),
        };
        if !seen.insert(id.clone()) {
            return Err(format!("line {line_no}: duplicate id \"{id}\""));
        }
        entries.push(ManifestEntry {
            id,
            audio: base_dir.join(audio),
        });
    }
    if entries.is_empty() {
        return Err("no utterances listed".to_string());
    }
    Ok(entries)
}

/// Run the `--manifest` harness. Every WAV is checked before the model loads.
/// Results go to `--out` (UTF-8 JSONL, flushed line by line) or stdout.
/// Exit code: 0 when every utterance was transcribed, 1 on a runtime failure
/// (VAD or model load, writing results, or any utterance — reported on its own
/// line), 2 on bad input (unreadable manifest, invalid entry or WAV, unwritable
/// `--out`).
pub(crate) fn run_manifest(app: &AppHandle, args: &CliArgs, manifest: &Path) -> i32 {
    let text = match std::fs::read_to_string(manifest) {
        Ok(text) => text,
        Err(e) => {
            eprintln!("error: cannot read {}: {}", manifest.display(), e);
            return 2;
        }
    };
    let base_dir = manifest.parent().unwrap_or_else(|| Path::new(""));
    let entries = match parse_manifest(&text, base_dir) {
        Ok(entries) => entries,
        Err(e) => {
            eprintln!("error: {}: {}", manifest.display(), e);
            return 2;
        }
    };
    let mut invalid = false;
    for entry in &entries {
        if let Err(e) = check_wav_format(&entry.audio) {
            eprintln!("error: {}: {}", entry.id, e);
            invalid = true;
        }
    }
    if invalid {
        return 2;
    }

    let mut out: Box<dyn Write> = match &args.out {
        Some(path) => match std::fs::File::create(path) {
            Ok(file) => Box::new(std::io::BufWriter::new(file)),
            Err(e) => {
                eprintln!("error: cannot write {}: {}", path.display(), e);
                return 2;
            }
        },
        None => Box::new(std::io::stdout().lock()),
    };

    let mut settings = get_settings(app);
    if let Some(language) = &args.language {
        settings.selected_language = language.clone();
    }

    let vad_mode = args.vad.unwrap_or(VadMode::Off);
    let (vad_name, backend) = match vad_mode {
        VadMode::Off => ("off", None),
        VadMode::Silero => ("silero", Some(VadBackend::Silero)),
        VadMode::Earshot => ("earshot", Some(VadBackend::Earshot)),
    };
    let mut vad = match backend {
        None => None,
        Some(backend) => match build_smoothed_vad(backend, app) {
            Ok((detector, offline_hangover_frames, _)) => {
                Some(OfflineVad::new(Box::new(detector), offline_hangover_frames))
            }
            Err(e) => {
                eprintln!("error: {}", e);
                return 1;
            }
        },
    };

    let tm = app.state::<Arc<TranscriptionManager>>();
    let model = match load_model(&tm, args, &settings.selected_model) {
        Ok(model) => model,
        Err(code) => return code,
    };

    let mut failed = 0usize;
    for entry in &entries {
        let line = match transcribe_entry(&tm, &model, args, &settings, vad.as_mut(), entry) {
            Ok(line) => line,
            Err(e) => {
                failed += 1;
                json!({
                    "type": "utterance",
                    "id": entry.id,
                    "audio": entry.audio.display().to_string(),
                    "error": e,
                })
            }
        };
        if let Err(e) = write_line(&mut out, &line) {
            eprintln!("error: cannot write results: {}", e);
            return 1;
        }
    }

    let summary = json!({
        "type": "summary",
        "model": model.id,
        "requested_device": model.requested_device,
        "bound_backend": model.bound_backend,
        "load_ms": model.load_ms,
        "language": settings.selected_language,
        "vad": vad_name,
        "utterances": entries.len(),
        "failed": failed,
    });
    if let Err(e) = write_line(&mut out, &summary) {
        eprintln!("error: cannot write results: {}", e);
        return 1;
    }
    if failed > 0 {
        1
    } else {
        0
    }
}

/// One JSONL line, flushed so a long run can be followed while it goes.
fn write_line(out: &mut dyn Write, line: &serde_json::Value) -> std::io::Result<()> {
    writeln!(out, "{}", line)?;
    out.flush()
}

/// One utterance through the recording path: VAD (when on), the short-clip
/// padding `stop_recording` applies, batch transcription with its text stages,
/// then the offline stage the app runs before pasting (OpenCC for
/// zh-Hans/zh-Hant). LLM post-processing is not run.
fn transcribe_entry(
    tm: &TranscriptionManager,
    model: &LoadedModel,
    args: &CliArgs,
    settings: &AppSettings,
    vad: Option<&mut OfflineVad>,
    entry: &ManifestEntry,
) -> Result<serde_json::Value, String> {
    let samples =
        read_wav_samples(&entry.audio).map_err(|e| format!("failed to read WAV: {}", e))?;
    let audio_secs = samples.len() as f64 / SAMPLE_RATE;

    let vad_start = Instant::now();
    let speech = match vad {
        Some(vad) => vad.filter(&samples),
        None => samples,
    };
    let vad_ms = vad_start.elapsed().as_millis() as u64;
    let speech_secs = speech.len() as f64 / SAMPLE_RATE;

    ensure_loaded(tm, model, args.device_index).map_err(|e| format!("reload failed: {}", e))?;
    let transcribe_start = Instant::now();
    let TranscriptionDetail {
        raw,
        language,
        mut stages,
        mut final_text,
    } = tm
        .transcribe_detailed(pad_short_recording(speech), settings)
        .map_err(|e| format!("transcribe failed: {}", e))?;
    let transcribe_ms = transcribe_start.elapsed().as_millis() as u64;

    if let Some(converted) =
        tauri::async_runtime::block_on(maybe_convert_chinese_variant(&language, &final_text))
    {
        stages.push(TextStage {
            name: "chinese_variant",
            text: converted.clone(),
        });
        final_text = converted;
    }

    Ok(json!({
        "type": "utterance",
        "id": entry.id,
        "audio": entry.audio.display().to_string(),
        "audio_secs": audio_secs,
        "speech_secs": speech_secs,
        "language": language,
        "raw": raw,
        "final": final_text,
        "stages": stages,
        "vad_ms": vad_ms,
        "transcribe_ms": transcribe_ms,
    }))
}

#[cfg(test)]
mod tests {
    use super::{parse_manifest, ManifestEntry};
    use std::path::{Path, PathBuf};

    #[test]
    fn manifest_resolves_audio_against_its_directory_and_defaults_ids() {
        let base = Path::new("corpus");
        let absolute = std::env::temp_dir().join("abs.wav");
        let text = format!(
            "\u{feff}{{\"id\": \"b01-f01\", \"audio\": \"wav/b01-f01.wav\", \"alvo\": \"Oi.\"}}\n\
             \r\n\
             {{\"audio\": {}}}\n",
            serde_json::to_string(&absolute.display().to_string()).unwrap()
        );

        let entries = parse_manifest(&text, base).expect("valid manifest");

        assert_eq!(
            entries,
            vec![
                ManifestEntry {
                    id: "b01-f01".to_string(),
                    audio: PathBuf::from("corpus").join("wav/b01-f01.wav"),
                },
                ManifestEntry {
                    id: absolute.display().to_string(),
                    audio: absolute.clone(),
                },
            ]
        );
    }

    #[test]
    fn manifest_errors_name_the_offending_line() {
        let base = Path::new("");
        let cases = [
            ("{\"id\": \"a\"}", "line 1: missing string field \"audio\""),
            (
                "{\"id\": \"a\", \"audio\": \"a.wav\"}\n{\"id\": \"a\", \"audio\": \"b.wav\"}",
                "line 2: duplicate id \"a\"",
            ),
            (
                "{\"id\": 7, \"audio\": \"a.wav\"}",
                "line 1: \"id\" must be a non-empty string",
            ),
            ("\n\n", "no utterances listed"),
        ];
        for (text, expected) in cases {
            assert_eq!(
                parse_manifest(text, base).unwrap_err(),
                expected,
                "{text:?}"
            );
        }
        assert!(parse_manifest("{not json", base)
            .unwrap_err()
            .starts_with("line 1: invalid JSON:"));
    }
}
