//! Per-recording latency trace (issue #4).
//!
//! A session spans one transcription recording: from the shortcut press that
//! starts it (`TranscribeAction::start`, see [`begin`]) to the paste that ends
//! it, or to its cancellation, failure or empty result. Each [`Mark`] records
//! the first time the pipeline reaches a milestone, relative to the press, and
//! the session ends with exactly one `info!` line (see [`render`]):
//!
//! ```text
//! Latency trace: press→frame 85 ms, press→speech 410 ms, press→feed 412 ms, press→first text 2310 ms; release→final 640 ms, release→paste 702 ms (model parakeet-tdt-0.6b-v3, live native, outcome pasted; ms from press: frame=85 speech=410 feed=412 text=2310 release=3050 final=3690 pasted=3752)
//! ```
//!
//! Marks never reached print `—`. Measurement only: nothing here changes
//! behaviour.
//!
//! Cost: marks on per-frame paths (recorder consumer, stream worker) check
//! their slot with a single relaxed atomic load and return once it is set;
//! only the first hit of a session reads the clock and stores. Nothing here
//! locks, allocates or logs per frame. The model/live-mode context sits
//! behind a mutex touched only by [`Session::describe`] and the final report.
//!
//! Isolation: every session gets a new generation. Slot values carry the low
//! bits of the generation that wrote them, and every producer (recorder,
//! stream worker, stop task) marks through the [`Session`] that was current
//! when it started, so a late preview chunk, a stop drain or a cancelled stop
//! task can never stamp the next recording.

use log::info;
use std::fmt::Write as _;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{LazyLock, Mutex};
use std::time::Instant;

/// Pipeline milestones of one recording. The press itself is the session's
/// zero point ([`begin`]) and has no slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    /// First captured chunk the recorder processes for this recording.
    FirstFrame,
    /// First 16 kHz frame the VAD lets through (any emitted frame with VAD off).
    FirstSpeech,
    /// First audio handed to a transcription engine for live text: the native
    /// stream's first feed, or the first batch live-preview chunk.
    FirstFeed,
    /// First non-blank live text sent to the overlay.
    FirstText,
    /// Stop requested (`TranscribeAction::stop`).
    Release,
    /// Final text ready: transcription plus output processing done.
    FinalText,
    /// Paste returned Ok.
    Pasted,
}

const MARK_COUNT: usize = 7;

impl Mark {
    /// Every mark in slot order (the declaration order above).
    const ALL: [Mark; MARK_COUNT] = [
        Mark::FirstFrame,
        Mark::FirstSpeech,
        Mark::FirstFeed,
        Mark::FirstText,
        Mark::Release,
        Mark::FinalText,
        Mark::Pasted,
    ];

    /// Short name used in the raw offset list of the log line.
    fn label(self) -> &'static str {
        match self {
            Mark::FirstFrame => "frame",
            Mark::FirstSpeech => "speech",
            Mark::FirstFeed => "feed",
            Mark::FirstText => "text",
            Mark::Release => "release",
            Mark::FinalText => "final",
            Mark::Pasted => "pasted",
        }
    }
}

/// How a session ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The final text was pasted.
    Pasted,
    /// The user cancelled the recording or its processing.
    Cancelled,
    /// No audio or no text came out, so nothing was pasted.
    Empty,
    /// Recording, transcription or paste failed.
    Error,
    /// A new press arrived before this session reported an end (for example
    /// the next recording started before a queued paste ran).
    Abandoned,
}

impl Outcome {
    fn label(self) -> &'static str {
        match self {
            Outcome::Pasted => "pasted",
            Outcome::Cancelled => "cancelled",
            Outcome::Empty => "empty",
            Outcome::Error => "error",
            Outcome::Abandoned => "abandoned",
        }
    }
}

/// Low bits of a slot value: the offset from the press in µs, plus one so 0
/// stays "unset". 2^40 µs is about 12.7 days, far beyond any recording. The
/// high bits hold the low bits of the generation that wrote the value.
const OFFSET_BITS: u32 = 40;
const OFFSET_MASK: u64 = (1u64 << OFFSET_BITS) - 1;
/// Low bit of the state word: the session is active (begun, not finished).
const ACTIVE: u64 = 1;

/// State word of an active session of `generation`.
fn active_state(generation: u64) -> u64 {
    (generation << 1) | ACTIVE
}

/// Slot value for `offset_us` written by session `generation`.
fn pack(generation: u64, offset_us: u64) -> u64 {
    (generation << OFFSET_BITS) | (offset_us.min(OFFSET_MASK - 1) + 1)
}

/// Whether slot `value` was written by session `generation`.
fn belongs(value: u64, generation: u64) -> bool {
    value != 0 && value >> OFFSET_BITS == (generation << OFFSET_BITS) >> OFFSET_BITS
}

/// Context printed with the report; tagged with the generation it describes.
struct Context {
    generation: u64,
    model: String,
    live: &'static str,
}

/// Lock-free session store. One process-global instance backs the public
/// API; tests build their own and inject times.
struct Trace {
    /// `generation << 1 | ACTIVE`. Every [`Trace::begin`] bumps the generation.
    state: AtomicU64,
    /// Press time of the current session, µs on the process trace clock.
    press_us: AtomicU64,
    /// One slot per [`Mark`]; 0 = unset, else [`pack`]ed.
    slots: [AtomicU64; MARK_COUNT],
    context: Mutex<Context>,
}

impl Trace {
    const fn new() -> Self {
        Self {
            state: AtomicU64::new(0),
            press_us: AtomicU64::new(0),
            slots: [const { AtomicU64::new(0) }; MARK_COUNT],
            context: Mutex::new(Context {
                generation: 0,
                model: String::new(),
                live: "",
            }),
        }
    }

    /// Start a new session pressed at `now_us`. Also returns the previous
    /// session's report if it never finished.
    fn begin(&self, now_us: u64) -> (Session, Option<Report>) {
        // Bump the generation inactive first: writers of the old session stop
        // matching the state before any slot is reset.
        let mut previous = self.state.load(Ordering::Acquire);
        let generation = loop {
            let next = (previous >> 1) + 1;
            match self.state.compare_exchange_weak(
                previous,
                next << 1,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => break next,
                Err(actual) => previous = actual,
            }
        };
        let abandoned =
            ((previous & ACTIVE) != 0).then(|| self.report(previous >> 1, Outcome::Abandoned));
        for slot in &self.slots {
            slot.store(0, Ordering::Relaxed);
        }
        self.press_us.store(now_us, Ordering::Relaxed);
        self.state
            .store(active_state(generation), Ordering::Release);
        (Session(generation), abandoned)
    }

    /// Record `mark` for session `generation` unless it is already set or the
    /// session is no longer the active one. The clock is only read on a store.
    fn record(&self, generation: u64, mark: Mark, now_us: impl FnOnce() -> u64) {
        let state = self.state.load(Ordering::Acquire);
        if state != active_state(generation) {
            return;
        }
        let slot = &self.slots[mark as usize];
        let current = slot.load(Ordering::Relaxed);
        if belongs(current, generation) {
            return; // first mark wins
        }
        let offset = now_us().saturating_sub(self.press_us.load(Ordering::Relaxed));
        let value = pack(generation, offset);
        if slot
            .compare_exchange(current, value, Ordering::AcqRel, Ordering::Relaxed)
            .is_err()
        {
            return;
        }
        // A session that ended or restarted between the state check and the
        // store must not keep this value.
        if self.state.load(Ordering::Acquire) != state {
            let _ = slot.compare_exchange(value, 0, Ordering::AcqRel, Ordering::Relaxed);
        }
    }

    /// Attach the model and live mode to session `generation`'s report.
    fn describe(&self, generation: u64, model: &str, live: &'static str) {
        if self.state.load(Ordering::Acquire) != active_state(generation) {
            return;
        }
        let mut context = self.context.lock().unwrap_or_else(|e| e.into_inner());
        context.generation = generation;
        context.model.clear();
        context.model.push_str(model);
        context.live = live;
    }

    /// End session `generation`. Returns its report the first time only;
    /// later calls, and calls for a session that is no longer current, get
    /// `None`.
    fn finish(&self, generation: u64, outcome: Outcome) -> Option<Report> {
        self.state
            .compare_exchange(
                active_state(generation),
                generation << 1,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .ok()?;
        Some(self.report(generation, outcome))
    }

    /// Snapshot session `generation`'s marks and context.
    fn report(&self, generation: u64, outcome: Outcome) -> Report {
        let offsets_us = std::array::from_fn(|i| {
            let value = self.slots[i].load(Ordering::Acquire);
            belongs(value, generation).then(|| (value & OFFSET_MASK) - 1)
        });
        let context = self.context.lock().unwrap_or_else(|e| e.into_inner());
        let (model, live) = if context.generation == generation {
            (context.model.clone(), context.live)
        } else {
            (String::new(), "")
        };
        Report {
            offsets_us,
            model,
            live,
            outcome,
        }
    }
}

/// Marks and context of one finished session.
#[derive(Debug, PartialEq, Eq)]
struct Report {
    /// Offset of each [`Mark`] from the press, in µs; `None` if never reached.
    offsets_us: [Option<u64>; MARK_COUNT],
    model: String,
    live: &'static str,
    outcome: Outcome,
}

impl Report {
    fn at(&self, mark: Mark) -> Option<u64> {
        self.offsets_us[mark as usize]
    }

    /// Time from the release to `mark`, if both were reached in that order.
    fn since_release(&self, mark: Mark) -> Option<u64> {
        self.at(mark)?.checked_sub(self.at(Mark::Release)?)
    }
}

/// Whole milliseconds, rounded, or `—` when the mark was never reached.
fn ms(us: Option<u64>) -> String {
    match us {
        Some(us) => format!("{} ms", us.saturating_add(500) / 1000),
        None => "—".to_string(),
    }
}

/// The single log line of a finished session.
fn render(report: &Report) -> String {
    let mut line = format!(
        "Latency trace: press→frame {}, press→speech {}, press→feed {}, press→first text {}; \
         release→final {}, release→paste {} (model {}, live {}, outcome {}; ms from press:",
        ms(report.at(Mark::FirstFrame)),
        ms(report.at(Mark::FirstSpeech)),
        ms(report.at(Mark::FirstFeed)),
        ms(report.at(Mark::FirstText)),
        ms(report.since_release(Mark::FinalText)),
        ms(report.since_release(Mark::Pasted)),
        if report.model.is_empty() {
            "?"
        } else {
            report.model.as_str()
        },
        if report.live.is_empty() {
            "?"
        } else {
            report.live
        },
        report.outcome.label(),
    );
    for mark in Mark::ALL {
        match report.at(mark) {
            Some(us) => {
                let _ = write!(line, " {}={}", mark.label(), us.saturating_add(500) / 1000);
            }
            None => {
                let _ = write!(line, " {}=—", mark.label());
            }
        }
    }
    line.push(')');
    line
}

static TRACE: Trace = Trace::new();
/// Zero of the process trace clock; forced by the first press.
static CLOCK_BASE: LazyLock<Instant> = LazyLock::new(Instant::now);

/// Microseconds on the process trace clock (monotonic, starts at first use).
fn now_us() -> u64 {
    let elapsed = CLOCK_BASE.elapsed();
    u64::try_from(elapsed.as_micros()).unwrap_or(u64::MAX)
}

/// One recording's trace. Every mark goes through the session that was
/// current when its producer started (the press, the recorder's `Cmd::Start`,
/// the stream worker's spawn, the stop), so code that outlives its recording
/// can never stamp a later one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Session(u64);

impl Session {
    /// Belongs to no recording: marks and finish through it are no-ops.
    /// Generation 0 is never active ([`Trace::begin`] starts at 1).
    pub const NONE: Session = Session(0);

    /// Record `mark` unless it is already set or this session is over. A
    /// single relaxed atomic load once the mark is set.
    #[inline]
    pub fn mark(self, mark: Mark) {
        if belongs(TRACE.slots[mark as usize].load(Ordering::Relaxed), self.0) {
            return;
        }
        TRACE.record(self.0, mark, now_us);
    }

    /// Name the model and live-text path printed with this session's report.
    pub fn describe(self, model: &str, live: &'static str) {
        TRACE.describe(self.0, model, live);
    }

    /// End this session and log its trace line. Only the first end of a
    /// session logs; later or stale calls are no-ops.
    pub fn finish(self, outcome: Outcome) {
        if let Some(report) = TRACE.finish(self.0, outcome) {
            info!("{}", render(&report));
        }
    }
}

/// Start a new session at a transcription shortcut press. Resets every mark;
/// a previous session that never ended is logged as abandoned first.
pub fn begin() -> Session {
    let (session, abandoned) = TRACE.begin(now_us());
    if let Some(report) = abandoned {
        info!("{}", render(&report));
    }
    session
}

/// The current session (possibly already finished, in which case marks and
/// finish through it are no-ops).
pub fn session() -> Session {
    Session(TRACE.state.load(Ordering::Acquire) >> 1)
}

/// End the current session, for paths that hold no [`Session`] (cancel).
pub fn finish(outcome: Outcome) {
    session().finish(outcome);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(offsets_ms: [Option<u64>; MARK_COUNT], outcome: Outcome) -> Report {
        Report {
            offsets_us: offsets_ms.map(|ms| ms.map(|ms| ms * 1000)),
            model: "parakeet".to_string(),
            live: "native",
            outcome,
        }
    }

    #[test]
    fn first_mark_wins() {
        let trace = Trace::new();
        let (session, _) = trace.begin(1_000);
        trace.record(session.0, Mark::FirstText, || 3_000);
        trace.record(session.0, Mark::FirstText, || 9_000);
        let report = trace.finish(session.0, Outcome::Pasted).unwrap();
        assert_eq!(report.at(Mark::FirstText), Some(2_000));
    }

    #[test]
    fn the_none_session_never_records_or_ends_a_session() {
        let trace = Trace::new();
        let (session, _) = trace.begin(0);
        trace.record(Session::NONE.0, Mark::FirstFrame, || 10);
        trace.describe(Session::NONE.0, "none", "off");
        assert!(trace.finish(Session::NONE.0, Outcome::Pasted).is_none());
        let report = trace.finish(session.0, Outcome::Pasted).unwrap();
        assert_eq!(report.at(Mark::FirstFrame), None);
        assert_eq!(report.model, "");
    }

    #[test]
    fn finish_reports_once_and_freezes_the_session() {
        let trace = Trace::new();
        let (session, _) = trace.begin(0);
        trace.describe(session.0, "whisper", "preview");
        trace.record(session.0, Mark::Release, || 500);
        let report = trace.finish(session.0, Outcome::Empty).unwrap();
        assert_eq!(report.model, "whisper");
        assert_eq!(report.live, "preview");
        assert_eq!(report.outcome, Outcome::Empty);
        assert!(trace.finish(session.0, Outcome::Pasted).is_none());

        // Marks after the end are ignored, so a later begin starts clean.
        trace.record(session.0, Mark::Pasted, || 900);
        assert_eq!(
            trace.report(session.0, Outcome::Pasted).at(Mark::Pasted),
            None
        );
    }

    #[test]
    fn new_session_resets_marks_and_ignores_the_previous_one() {
        let trace = Trace::new();
        let (old, _) = trace.begin(0);
        trace.describe(old.0, "old-model", "native");
        trace.record(old.0, Mark::FirstFrame, || 50);
        trace.record(old.0, Mark::FirstText, || 2_000);
        assert!(trace.finish(old.0, Outcome::Cancelled).is_some());

        let (new, abandoned) = trace.begin(10_000);
        assert!(abandoned.is_none());
        assert_ne!(old, new);

        // A late preview chunk / stop task of the old recording must not
        // stamp or describe the new one, nor end it.
        trace.record(old.0, Mark::FirstText, || 10_500);
        trace.record(old.0, Mark::Pasted, || 10_600);
        trace.describe(old.0, "stale", "off");
        assert!(trace.finish(old.0, Outcome::Pasted).is_none());

        trace.record(new.0, Mark::FirstFrame, || 10_080);
        let report = trace.finish(new.0, Outcome::Error).unwrap();
        assert_eq!(report.at(Mark::FirstFrame), Some(80));
        for mark in [
            Mark::FirstSpeech,
            Mark::FirstFeed,
            Mark::FirstText,
            Mark::Release,
            Mark::FinalText,
            Mark::Pasted,
        ] {
            assert_eq!(report.at(mark), None, "{mark:?} leaked");
        }
        assert_eq!(report.model, "");
    }

    #[test]
    fn stale_slot_value_is_neither_reported_nor_blocking() {
        let trace = Trace::new();
        let (old, _) = trace.begin(0);
        let (new, _) = trace.begin(1_000);
        // As if an old-session store raced past the reset in `begin`.
        trace.slots[Mark::FirstText as usize].store(pack(old.0, 5), Ordering::Relaxed);
        assert_eq!(
            trace.report(new.0, Outcome::Pasted).at(Mark::FirstText),
            None
        );

        trace.record(new.0, Mark::FirstText, || 1_300);
        let report = trace.finish(new.0, Outcome::Pasted).unwrap();
        assert_eq!(report.at(Mark::FirstText), Some(300));
    }

    #[test]
    fn begin_reports_an_unfinished_session_as_abandoned() {
        let trace = Trace::new();
        let (old, _) = trace.begin(0);
        trace.record(old.0, Mark::FinalText, || 4_000);
        let (new, abandoned) = trace.begin(5_000);
        let abandoned = abandoned.unwrap();
        assert_eq!(abandoned.outcome, Outcome::Abandoned);
        assert_eq!(abandoned.at(Mark::FinalText), Some(4_000));
        assert!(trace.finish(old.0, Outcome::Pasted).is_none());
        assert!(trace.finish(new.0, Outcome::Pasted).is_some());
    }

    #[test]
    fn render_dashes_missing_marks_and_measures_final_and_paste_from_release() {
        let partial = render(&report(
            [Some(85), None, None, None, Some(3_000), None, None],
            Outcome::Cancelled,
        ));
        for expected in [
            "press→frame 85 ms",
            "press→speech —",
            "release→paste —",
            "pasted=—",
        ] {
            assert!(
                partial.contains(expected),
                "{expected:?} missing from {partial}"
            );
        }

        let full = render(&report(
            [
                Some(85),
                Some(410),
                Some(412),
                Some(2_310),
                Some(3_050),
                Some(3_690),
                Some(3_752),
            ],
            Outcome::Pasted,
        ));
        for expected in [
            "release→final 640 ms",
            "release→paste 702 ms",
            "outcome pasted",
        ] {
            assert!(full.contains(expected), "{expected:?} missing from {full}");
        }
    }

    #[test]
    fn release_relative_deltas_need_the_release() {
        let mut report = report(
            [None, None, None, None, None, Some(700), Some(800)],
            Outcome::Pasted,
        );
        assert_eq!(report.since_release(Mark::FinalText), None);
        report.offsets_us[Mark::Release as usize] = Some(500_000);
        assert_eq!(report.since_release(Mark::FinalText), Some(200_000));
        assert_eq!(report.since_release(Mark::Pasted), Some(300_000));
    }

    #[test]
    fn render_rounds_to_whole_milliseconds() {
        let mut report = report([None; MARK_COUNT], Outcome::Empty);
        report.offsets_us[Mark::FirstFrame as usize] = Some(84_500);
        report.offsets_us[Mark::FirstSpeech as usize] = Some(84_499);
        let line = render(&report);
        assert!(line.contains("press→frame 85 ms"), "{line}");
        assert!(line.contains("press→speech 84 ms"), "{line}");
    }
}
