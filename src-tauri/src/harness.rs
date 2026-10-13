//! Offline evaluation harness behind `--manifest` (phase F0a of
//! `docs/ULTRAPLAN.md`): runs a list of WAVs through the app's own VAD, batch
//! transcription and post-ASR text stages, and prints one JSON line per
//! utterance plus a summary line for metrics and baselines to consume.
//!
//! With `--live-model` (phase F6) it first replays each WAV at 1x real time
//! through the live slot, as the microphone feeds a recording whose
//! transcription model has no live text of its own, to measure when the first
//! live text appears and how long the final pass takes after release.
//!
//! Also holds the WAV and model checks the single-file `--transcribe-file`
//! path shares with it.

use crate::actions::maybe_convert_chinese_variant;
use crate::audio_toolkit::{read_wav_samples, OfflineVad};
use crate::cli::{CliArgs, VadMode};
use crate::managers::audio::{build_smoothed_vad, pad_short_recording};
use crate::managers::transcription::{
    StreamTextEvent, TextStage, TranscriptionDetail, TranscriptionManager,
};
use crate::settings::{get_settings, AppSettings, VadBackend};
use serde_json::json;
use std::collections::HashSet;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, EventId, Listener, Manager};
use tauri_specta::Event;

const SAMPLE_RATE_HZ: u64 = 16_000;
const SAMPLE_RATE: f64 = SAMPLE_RATE_HZ as f64;

/// Samples per replayed `--live-model` frame: 20 ms at 16 kHz.
const LIVE_FRAME_SAMPLES: usize = 320;

/// How long to wait for a cancelled live stream's worker to exit before the
/// next stream may start. It skips the frames still queued and only finishes
/// the one being decoded, so running out means the worker is stuck.
const LIVE_WORKER_EXIT_TIMEOUT: Duration = Duration::from_secs(30);

/// Private event that flushes Tauri's queued events (see
/// [`LiveListener::detach`]).
const LIVE_FENCE_EVENT: &str = "harness-live-fence";

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
/// (VAD, model or `--live-model` load, writing results, or any utterance —
/// reported on its own line), 2 on bad input (unreadable manifest, invalid
/// entry or WAV, unwritable `--out`).
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

    let mut live_totals = match &args.live_model {
        None => None,
        Some(id) => match load_live_slot(&tm, id) {
            Ok(totals) => Some(totals),
            Err(code) => return code,
        },
    };
    let live_slot = args
        .live_model
        .as_deref()
        .map(|model_id| LiveSlot { app, model_id });

    let mut failed = 0usize;
    for entry in &entries {
        let result = transcribe_entry(
            &tm,
            &model,
            args,
            &settings,
            vad.as_mut(),
            live_slot.as_ref(),
            entry,
        );
        let line = match result {
            Ok((line, live)) => {
                if let (Some(totals), Some(live)) = (live_totals.as_mut(), live.as_ref()) {
                    totals.record(live);
                }
                line
            }
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

    let mut summary = json!({
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
    if let Some(totals) = live_totals.as_mut() {
        totals.add_to(&mut summary);
    }
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
///
/// With a live slot, the whole clip is first replayed through it at 1x (see
/// [`replay_live`]) and the final pass is timed from release; the line gains
/// the `--live-model` fields and the measurement is returned for the summary.
fn transcribe_entry(
    tm: &TranscriptionManager,
    model: &LoadedModel,
    args: &CliArgs,
    settings: &AppSettings,
    vad: Option<&mut OfflineVad>,
    live: Option<&LiveSlot>,
    entry: &ManifestEntry,
) -> Result<(serde_json::Value, Option<LiveMeasure>), String> {
    let samples =
        read_wav_samples(&entry.audio).map_err(|e| format!("failed to read WAV: {}", e))?;
    let audio_secs = samples.len() as f64 / SAMPLE_RATE;

    // `None` with VAD off: the whole clip is the speech. The recorder filters
    // while recording, so this stays out of `release_to_final_ms`.
    let vad_start = Instant::now();
    let filtered = vad.map(|vad| vad.filter(&samples));
    let vad_ms = vad_start.elapsed().as_millis() as u64;
    let speech_secs = filtered.as_ref().map_or(samples.len(), Vec::len) as f64 / SAMPLE_RATE;

    ensure_loaded(tm, model, args.device_index).map_err(|e| format!("reload failed: {}", e))?;
    // The live stream hears every microphone frame; VAD only shapes what the
    // final model gets.
    let replay = live
        .map(|live| replay_live(tm, live, &samples))
        .transpose()?;

    let final_result = final_pass(tm, settings, filtered.unwrap_or(samples));
    // Detach the live listener even when the final pass failed.
    let live_measure = replay.map(|replay| {
        let release_to_final_ms = replay.release.elapsed().as_millis() as u64;
        replay.finish(tm, release_to_final_ms)
    });
    let (detail, transcribe_ms) = final_result?;
    let live_measure = live_measure.transpose()?;

    let timings = UtteranceTimings {
        audio_secs,
        speech_secs,
        vad_ms,
        transcribe_ms,
    };
    let line = utterance_line(
        entry,
        &detail,
        &timings,
        live.zip(live_measure.as_ref())
            .map(|(live, measure)| (live.model_id, measure)),
    );
    Ok((line, live_measure))
}

/// The final pass the app runs on a recording before pasting: batch
/// transcription with its text stages, then OpenCC for zh-Hans/zh-Hant.
/// Returns the detail and `transcribe_ms` (the batch transcription alone).
fn final_pass(
    tm: &TranscriptionManager,
    settings: &AppSettings,
    speech: Vec<f32>,
) -> Result<(TranscriptionDetail, u64), String> {
    let transcribe_start = Instant::now();
    let mut detail = tm
        .transcribe_detailed(pad_short_recording(speech), settings)
        .map_err(|e| format!("transcribe failed: {}", e))?;
    let transcribe_ms = transcribe_start.elapsed().as_millis() as u64;

    if let Some(converted) = tauri::async_runtime::block_on(maybe_convert_chinese_variant(
        &detail.language,
        &detail.final_text,
    )) {
        detail.stages.push(TextStage {
            name: "chinese_variant",
            text: converted.clone(),
        });
        detail.final_text = converted;
    }
    Ok((detail, transcribe_ms))
}

/// Timings of one transcribed utterance, as its JSON line reports them.
struct UtteranceTimings {
    audio_secs: f64,
    speech_secs: f64,
    vad_ms: u64,
    transcribe_ms: u64,
}

/// The JSON line of one transcribed utterance. With a live model it gains
/// `live_model`, `live_first_text_ms` (null when no live text appeared before
/// release), `live_text`, `live_updates` and `release_to_final_ms`.
fn utterance_line(
    entry: &ManifestEntry,
    detail: &TranscriptionDetail,
    timings: &UtteranceTimings,
    live: Option<(&str, &LiveMeasure)>,
) -> serde_json::Value {
    let mut line = json!({
        "type": "utterance",
        "id": entry.id,
        "audio": entry.audio.display().to_string(),
        "audio_secs": timings.audio_secs,
        "speech_secs": timings.speech_secs,
        "language": detail.language,
        "raw": detail.raw,
        "final": detail.final_text,
        "stages": detail.stages,
        "vad_ms": timings.vad_ms,
        "transcribe_ms": timings.transcribe_ms,
    });
    if let Some((model, measure)) = live {
        line["live_model"] = json!(model);
        line["live_first_text_ms"] = json!(measure.first_text_ms);
        line["live_text"] = json!(measure.text);
        line["live_updates"] = json!(measure.updates);
        line["release_to_final_ms"] = json!(measure.release_to_final_ms);
    }
    line
}

/// The live slot a `--live-model` run replays every utterance through, and the
/// app handle its [`StreamTextEvent`]s are observed on.
struct LiveSlot<'a> {
    app: &'a AppHandle,
    model_id: &'a str,
}

/// What `--live-model` measured for one utterance.
struct LiveMeasure {
    /// Replay start to the first non-blank live text; `None` when none
    /// appeared before release.
    first_text_ms: Option<u64>,
    /// The last live text seen by release (`committed` + `tentative`).
    text: String,
    /// Live text updates seen by release.
    updates: usize,
    /// Release (end of audio) to the final text.
    release_to_final_ms: u64,
}

/// The `--live-model` part of the summary line.
struct LiveTotals {
    model: String,
    load_ms: u64,
    /// Only utterances that showed live text before release.
    first_text_ms: Vec<u64>,
    release_to_final_ms: Vec<u64>,
}

impl LiveTotals {
    fn record(&mut self, measure: &LiveMeasure) {
        self.first_text_ms.extend(measure.first_text_ms);
        self.release_to_final_ms.push(measure.release_to_final_ms);
    }

    /// Add `live_model`, `live_load_ms`, `live_first_text_ms_median` and
    /// `release_to_final_ms_median` (null without values) to the summary line.
    fn add_to(&mut self, summary: &mut serde_json::Value) {
        summary["live_model"] = json!(self.model);
        summary["live_load_ms"] = json!(self.load_ms);
        summary["live_first_text_ms_median"] = json!(median_ms(&mut self.first_text_ms));
        summary["release_to_final_ms_median"] = json!(median_ms(&mut self.release_to_final_ms));
    }
}

/// Median of `values` (the mean of the middle two for an even count); `None`
/// when empty.
fn median_ms(values: &mut [u64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    values.sort_unstable();
    let mid = values.len() / 2;
    Some(if values.len() % 2 == 1 {
        values[mid] as f64
    } else {
        (values[mid - 1] as f64 + values[mid] as f64) / 2.0
    })
}

/// Load `--live-model` into the live slot, timed like the main model. On
/// failure, prints the error and returns exit code 1.
fn load_live_slot(tm: &TranscriptionManager, id: &str) -> Result<LiveTotals, i32> {
    let load_start = Instant::now();
    if let Err(e) = tm.load_live_model(id) {
        eprintln!("error: load_live_model('{}') failed: {}", id, e);
        return Err(1);
    }
    Ok(LiveTotals {
        model: id.to_string(),
        load_ms: load_start.elapsed().as_millis() as u64,
        first_text_ms: Vec::new(),
        release_to_final_ms: Vec::new(),
    })
}

/// Like [`ensure_loaded`] for the live slot, which the "Immediately" unload
/// timeout drops along with the main model after every final pass.
fn ensure_live_loaded(tm: &TranscriptionManager, model_id: &str) -> anyhow::Result<()> {
    if tm.live_model_id().as_deref() != Some(model_id) {
        tm.load_live_model(model_id)?;
    }
    Ok(())
}

/// Wait (untimed) for the last stream worker to exit: until it has, the next
/// `start_live_stream` is refused by the one-worker guard.
fn wait_for_stream_worker(tm: &TranscriptionManager) -> Result<(), String> {
    let deadline = Instant::now() + LIVE_WORKER_EXIT_TIMEOUT;
    while tm.is_stream_worker_active() {
        if Instant::now() >= deadline {
            return Err(format!(
                "live stream worker still running after {}s",
                LIVE_WORKER_EXIT_TIMEOUT.as_secs()
            ));
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    Ok(())
}

/// One replayed microphone frame: samples `start..end` of the clip, fed `due`
/// after the replay start, when its last sample would have been captured.
struct ReplayFrame {
    start: usize,
    end: usize,
    due: Duration,
}

/// The 1x replay schedule of a `sample_count`-sample clip: 20 ms frames, the
/// last one shorter when the clip is not a whole number of frames. Due times
/// are offsets from one start instant, so sleep overshoot never accumulates.
fn replay_schedule(sample_count: usize) -> impl Iterator<Item = ReplayFrame> {
    (0..sample_count)
        .step_by(LIVE_FRAME_SAMPLES)
        .map(move |start| {
            let end = (start + LIVE_FRAME_SAMPLES).min(sample_count);
            ReplayFrame {
                start,
                end,
                due: Duration::from_nanos(end as u64 * 1_000_000_000 / SAMPLE_RATE_HZ),
            }
        })
}

/// Replay `samples` through the live slot at 1x real time, as the microphone
/// feeds a recording: start the live stream, feed 20 ms frames paced against
/// one monotonic start instant, then at release (end of audio) cancel the
/// stream, as the app does before its final pass: live text is never the final
/// text. The listener stays attached until [`LiveReplay::finish`].
fn replay_live(
    tm: &TranscriptionManager,
    live: &LiveSlot,
    samples: &[f32],
) -> Result<LiveReplay, String> {
    wait_for_stream_worker(tm)?;
    ensure_live_loaded(tm, live.model_id).map_err(|e| format!("live reload failed: {}", e))?;

    let listener = LiveListener::attach(live.app);
    let router = tm.stream_router();
    tm.start_live_stream();
    let replay_start = Instant::now();
    for frame in replay_schedule(samples.len()) {
        let due = replay_start + frame.due;
        let now = Instant::now();
        if due > now {
            std::thread::sleep(due - now);
        }
        router.feed(&samples[frame.start..frame.end]);
    }
    let release = Instant::now();
    let seen = listener.close();
    tm.cancel_stream();
    Ok(LiveReplay {
        listener,
        seen,
        replay_start,
        release,
    })
}

/// One utterance replayed through the live slot, waiting for its final pass.
struct LiveReplay {
    listener: LiveListener,
    /// What the listener had seen at release.
    seen: LiveObservation,
    replay_start: Instant,
    release: Instant,
}

impl LiveReplay {
    /// After the final pass, `release_to_final_ms` after release: detach the
    /// listener once the cancelled stream's worker has exited and report.
    fn finish(
        self,
        tm: &TranscriptionManager,
        release_to_final_ms: u64,
    ) -> Result<LiveMeasure, String> {
        self.listener.detach(tm)?;
        Ok(LiveMeasure {
            first_text_ms: self.seen.first_text_ms(self.replay_start),
            text: self.seen.text,
            updates: self.seen.updates,
            release_to_final_ms,
        })
    }
}

/// The live text one replay produced, as the overlay showed it.
#[derive(Clone, Default)]
struct LiveObservation {
    /// When the first non-blank live text arrived.
    first_text_at: Option<Instant>,
    /// The latest live text: `committed` + `tentative`, as the overlay
    /// renders it.
    text: String,
    /// [`StreamTextEvent`]s received.
    updates: usize,
    /// Set at release. Text decoded afterwards (a frame still in flight when
    /// the stream was cancelled) came after the user stopped speaking and is
    /// not counted.
    closed: bool,
}

impl LiveObservation {
    /// Take one live text update, received at `at`.
    fn record(&mut self, at: Instant, committed: &str, tentative: &str) {
        if self.closed {
            return;
        }
        self.updates += 1;
        self.text.clear();
        self.text.push_str(committed);
        self.text.push_str(tentative);
        // The same test as the app's `FirstText` latency mark.
        if self.first_text_at.is_none() && !self.text.trim().is_empty() {
            self.first_text_at = Some(at);
        }
    }

    /// Milliseconds from `replay_start` to the first non-blank live text.
    fn first_text_ms(&self, replay_start: Instant) -> Option<u64> {
        self.first_text_at
            .map(|at| at.saturating_duration_since(replay_start).as_millis() as u64)
    }
}

/// Collects the live slot's [`StreamTextEvent`]s for one replay through a Rust
/// listener on the app handle. Tauri runs Rust listeners synchronously inside
/// `emit`, on the stream worker's thread, so each update is timestamped as the
/// overlay is sent it; no webview or event-loop turn is involved, which is why
/// this works headless too.
struct LiveListener {
    app: AppHandle,
    id: EventId,
    seen: Arc<Mutex<LiveObservation>>,
}

impl LiveListener {
    fn attach(app: &AppHandle) -> Self {
        let seen = Arc::new(Mutex::new(LiveObservation::default()));
        let sink = Arc::clone(&seen);
        let id = StreamTextEvent::listen(app, move |event| {
            let at = Instant::now();
            let StreamTextEvent {
                committed,
                tentative,
            } = event.payload;
            sink.lock().unwrap().record(at, &committed, &tentative);
        });
        Self {
            app: app.clone(),
            id,
            seen,
        }
    }

    /// Release: stop counting updates and return what was seen so far.
    fn close(&self) -> LiveObservation {
        let mut seen = self.seen.lock().unwrap();
        seen.closed = true;
        seen.clone()
    }

    /// Remove the listener once the cancelled stream's worker has exited, so
    /// nothing emits for this replay any more.
    fn detach(self, tm: &TranscriptionManager) -> Result<(), String> {
        let exited = wait_for_stream_worker(tm);
        // An emit that finds Tauri's listener table busy (another thread
        // emitting, e.g. the final pass unloading a model) is queued and only
        // delivered by the next emit that has a listener, which could be the
        // next replay's. A private fence flushes that queue into this closed
        // listener first.
        let fence = self.app.listen(LIVE_FENCE_EVENT, |_| {});
        let _ = self.app.emit(LIVE_FENCE_EVENT, ());
        self.app.unlisten(fence);
        self.app.unlisten(self.id);
        exited
    }
}

#[cfg(test)]
mod tests {
    use super::{
        median_ms, parse_manifest, replay_schedule, utterance_line, LiveMeasure, LiveObservation,
        LiveTotals, ManifestEntry, UtteranceTimings,
    };
    use crate::cli::CliArgs;
    use crate::managers::transcription::TranscriptionDetail;
    use clap::error::ErrorKind;
    use clap::Parser;
    use serde_json::json;
    use std::path::{Path, PathBuf};
    use std::time::{Duration, Instant};

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

    #[test]
    fn replay_schedule_feeds_20ms_frames_once_captured() {
        let frames: Vec<_> = replay_schedule(700)
            .map(|frame| (frame.start, frame.end, frame.due))
            .collect();
        assert_eq!(
            frames,
            [
                (0, 320, Duration::from_millis(20)),
                (320, 640, Duration::from_millis(40)),
                // The short last frame is due when the clip ends.
                (640, 700, Duration::from_micros(43_750)),
            ]
        );
        assert_eq!(replay_schedule(0).count(), 0);
        let last = replay_schedule(640)
            .last()
            .map(|frame| (frame.end, frame.due));
        assert_eq!(last, Some((640, Duration::from_millis(40))));

        // Due times come from the sample position, not from summing frame
        // lengths: a minute of audio is fed contiguously and ends at 60 s.
        let frames: Vec<_> = replay_schedule(60 * 16_000).collect();
        assert_eq!(frames.len(), 3_000);
        assert!(frames
            .windows(2)
            .all(|pair| pair[0].end == pair[1].start && pair[0].due < pair[1].due));
        assert_eq!(frames[2_999].due, Duration::from_secs(60));
    }

    #[test]
    fn live_model_flag_needs_a_manifest_and_a_model_id() {
        let parse = |argv: &[&str]| CliArgs::try_parse_from(argv.iter().copied());

        let args = parse(&["handy", "--manifest", "m.jsonl", "--live-model", "nemotron"])
            .expect("--live-model with --manifest");
        assert_eq!(args.live_model.as_deref(), Some("nemotron"));
        let args = parse(&["handy", "--manifest", "m.jsonl"]).expect("--manifest alone");
        assert_eq!(args.live_model, None);

        let cases: [(&[&str], ErrorKind); 3] = [
            (
                &["handy", "--live-model", "nemotron"],
                ErrorKind::MissingRequiredArgument,
            ),
            (
                &[
                    "handy",
                    "--transcribe-file",
                    "a.wav",
                    "--live-model",
                    "nemotron",
                ],
                ErrorKind::MissingRequiredArgument,
            ),
            (
                &["handy", "--manifest", "m.jsonl", "--live-model", ""],
                ErrorKind::InvalidValue,
            ),
        ];
        for (argv, kind) in cases {
            let err = parse(argv).unwrap_err();
            assert_eq!(err.kind(), kind, "{argv:?}");
            // clap exits with 2, the harness's bad-input code.
            assert_eq!(err.exit_code(), 2, "{argv:?}");
        }
    }

    #[test]
    fn utterance_lines_gain_the_live_fields_only_with_a_live_model() {
        let entry = ManifestEntry {
            id: "b01-f01".to_string(),
            audio: PathBuf::from("b01-f01.wav"),
        };
        let detail = TranscriptionDetail {
            raw: "oi tudo bem".to_string(),
            language: "pt".to_string(),
            stages: Vec::new(),
            final_text: "Oi, tudo bem?".to_string(),
        };
        let timings = UtteranceTimings {
            audio_secs: 1.5,
            speech_secs: 1.25,
            vad_ms: 3,
            transcribe_ms: 210,
        };
        let keys = |line: &serde_json::Value| {
            let mut keys: Vec<String> = line.as_object().unwrap().keys().cloned().collect();
            keys.sort();
            keys
        };

        let plain = utterance_line(&entry, &detail, &timings, None);
        assert_eq!(
            keys(&plain),
            [
                "audio",
                "audio_secs",
                "final",
                "id",
                "language",
                "raw",
                "speech_secs",
                "stages",
                "transcribe_ms",
                "type",
                "vad_ms",
            ]
        );
        assert_eq!(plain["final"], "Oi, tudo bem?");
        assert_eq!(plain["transcribe_ms"], 210);

        let silent = LiveMeasure {
            first_text_ms: None,
            text: String::new(),
            updates: 0,
            release_to_final_ms: 240,
        };
        let line = utterance_line(&entry, &detail, &timings, Some(("nemotron", &silent)));
        for key in keys(&plain) {
            assert_eq!(line[&key], plain[&key], "{key}");
        }
        assert_eq!(keys(&line).len(), keys(&plain).len() + 5);
        assert_eq!(line["live_model"], "nemotron");
        assert_eq!(line["live_first_text_ms"], serde_json::Value::Null);
        assert_eq!(line["live_text"], "");
        assert_eq!(line["live_updates"], 0);
        assert_eq!(line["release_to_final_ms"], 240);

        let spoke = LiveMeasure {
            first_text_ms: Some(480),
            text: "oi tudo".to_string(),
            updates: 7,
            release_to_final_ms: 260,
        };
        let line = utterance_line(&entry, &detail, &timings, Some(("nemotron", &spoke)));
        assert_eq!(line["live_first_text_ms"], 480);
        assert_eq!(line["live_text"], "oi tudo");
        assert_eq!(line["live_updates"], 7);
        assert_eq!(line["release_to_final_ms"], 260);
    }

    #[test]
    fn live_summary_reports_load_time_and_medians() {
        let measure = |first_text_ms, release_to_final_ms| LiveMeasure {
            first_text_ms,
            text: String::new(),
            updates: 0,
            release_to_final_ms,
        };
        let totals = || LiveTotals {
            model: "nemotron".to_string(),
            load_ms: 900,
            first_text_ms: Vec::new(),
            release_to_final_ms: Vec::new(),
        };

        let mut live = totals();
        live.record(&measure(Some(400), 300));
        live.record(&measure(None, 200));
        let mut summary = json!({ "type": "summary" });
        live.add_to(&mut summary);
        assert_eq!(
            summary,
            json!({
                "type": "summary",
                "live_model": "nemotron",
                "live_load_ms": 900,
                "live_first_text_ms_median": 400.0,
                "release_to_final_ms_median": 250.0,
            })
        );

        // No utterance showed live text: a null median, not 0.
        let mut live = totals();
        live.record(&measure(None, 200));
        let mut summary = json!({ "type": "summary" });
        live.add_to(&mut summary);
        assert_eq!(
            summary["live_first_text_ms_median"],
            serde_json::Value::Null
        );
        assert_eq!(summary["release_to_final_ms_median"], 200.0);
    }

    #[test]
    fn median_ms_handles_empty_odd_and_even_counts() {
        assert_eq!(median_ms(&mut []), None);
        assert_eq!(median_ms(&mut [30, 10, 20]), Some(20.0));
        assert_eq!(median_ms(&mut [40, 10, 30, 20]), Some(25.0));
    }

    #[test]
    fn live_observation_times_the_first_non_blank_text_until_release() {
        let start = Instant::now();
        let at = |ms| start + Duration::from_millis(ms);

        let mut seen = LiveObservation::default();
        seen.record(at(60), "", " ");
        assert_eq!(seen.first_text_ms(start), None);
        seen.record(at(420), "", "oi");
        seen.record(at(500), "oi ", "tudo");
        assert_eq!(seen.first_text_ms(start), Some(420));
        assert_eq!((seen.text.as_str(), seen.updates), ("oi tudo", 3));

        // After release, a frame still in flight is not counted.
        seen.closed = true;
        seen.record(at(900), "oi tudo ", "bem");
        assert_eq!((seen.text.as_str(), seen.updates), ("oi tudo", 3));
        assert_eq!(seen.first_text_ms(start), Some(420));
    }
}
