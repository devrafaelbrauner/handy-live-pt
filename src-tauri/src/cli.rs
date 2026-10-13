use clap::Parser;
use std::path::PathBuf;

#[derive(Parser, Debug, Clone, Default)]
#[command(name = "handy", about = "Handy - Speech to Text")]
pub struct CliArgs {
    /// Start with the main window hidden
    #[arg(long)]
    pub start_hidden: bool,

    /// Disable the system tray icon
    #[arg(long)]
    pub no_tray: bool,

    /// Toggle transcription on/off (sent to running instance)
    #[arg(long)]
    pub toggle_transcription: bool,

    /// Toggle transcription with post-processing on/off (sent to running instance)
    #[arg(long)]
    pub toggle_post_process: bool,

    /// Cancel the current operation (sent to running instance)
    #[arg(long)]
    pub cancel: bool,

    /// Enable debug mode with verbose logging
    #[arg(long)]
    pub debug: bool,

    /// Transcribe this WAV (16 kHz mono) headlessly and exit. Runs the same
    /// batch transcription path as the app — no mic, no VAD, no download
    /// (the model must already be installed).
    #[arg(short = 'f', long, value_name = "WAV")]
    pub transcribe_file: Option<PathBuf>,

    /// Model id to load for --transcribe-file / --manifest (default: the
    /// selected model).
    #[arg(long)]
    pub model: Option<String>,

    /// Hard-select the compute device for --transcribe-file / --manifest by its
    /// registry index (see --list-devices). Omit to use the persisted
    /// accelerator setting. transcribe-cpp (whisper-family) models only.
    #[arg(long, value_name = "N")]
    pub device_index: Option<usize>,

    /// List the transcribe-cpp compute devices (with indices) and exit.
    #[arg(long)]
    pub list_devices: bool,

    /// List the available models (with ids) and exit. Pass an id to --model.
    /// Honors --json for machine-readable output.
    #[arg(long)]
    pub list_models: bool,

    /// Repeat the transcription N times (best_ms reports the fastest run).
    #[arg(long, value_name = "N")]
    pub repeat: Option<usize>,

    /// Emit --transcribe-file results as JSON.
    #[arg(long)]
    pub json: bool,

    /// Evaluation harness: transcribe every WAV (16 kHz mono 16-bit) listed in
    /// this JSONL manifest headlessly and exit. One object per line with
    /// "audio" (a path, relative to the manifest) and an optional unique "id"
    /// (default: the audio path); other fields are ignored. Prints one JSON
    /// line per utterance (raw and final text, each post-ASR text stage,
    /// timings) and a final summary line. With --live-model, each WAV is first
    /// replayed at 1x real time through that live model to time its first live
    /// text.
    #[arg(long, value_name = "JSONL", conflicts_with_all = ["transcribe_file", "repeat"])]
    pub manifest: Option<PathBuf>,

    /// Language intent for --manifest (e.g. "pt", "auto") in place of the
    /// selected language; coerced to the model's languages as in the app.
    #[arg(long, value_name = "CODE", requires = "manifest")]
    pub language: Option<String>,

    /// Voice activity detection for --manifest. "off" (default) sends the whole
    /// clip, as --transcribe-file does; "silero" / "earshot" first filter it
    /// through the recorder's VAD with the offline profile, as a recording is.
    #[arg(long, value_enum, requires = "manifest")]
    pub vad: Option<VadMode>,

    /// Live model for --manifest: load this downloaded transcribe-cpp model
    /// with native streaming (supports_streaming in --list-models --json) into
    /// the live slot and replay each WAV through it at 1x real time before the
    /// final pass, as the app does when the transcription model has no live
    /// text of its own. The replay feeds what the recorder would: every frame
    /// with --vad off, only the frames the --vad backend lets through (offline
    /// profile) otherwise. The live text is discarded. Utterance lines gain
    /// live_model, live_first_text_ms, live_text, live_updates and
    /// release_to_final_ms; the summary gains live_load_ms and the medians of
    /// live_first_text_ms and release_to_final_ms. The run takes at least as
    /// long as the audio.
    #[arg(
        long,
        value_name = "ID",
        requires = "manifest",
        conflicts_with = "transcribe_file",
        value_parser = clap::builder::NonEmptyStringValueParser::new()
    )]
    pub live_model: Option<String>,

    /// Write the --manifest results to this file (UTF-8 JSONL, one line per
    /// utterance as it finishes) instead of stdout. Use it on Windows: the app
    /// is a GUI program there, so PowerShell and cmd neither wait for it nor
    /// capture its stdout. Run it with
    /// `Start-Process handy-live-pt.exe -ArgumentList ... -Wait -PassThru`.
    #[arg(long, value_name = "JSONL", requires = "manifest")]
    pub out: Option<PathBuf>,
}

/// `--vad` choices for the --manifest harness.
#[derive(clap::ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum VadMode {
    Off,
    Silero,
    Earshot,
}
