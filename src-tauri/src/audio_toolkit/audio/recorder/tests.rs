use super::{
    is_microphone_access_denied, is_no_input_device_error, run_consumer, AudioRecorder,
    CaptureProcessor, CaptureTransportState, ChunkDisposition, Cmd, SegmentSink, VadConfig,
    VadPolicy,
};
use crate::audio_toolkit::vad::{VadFrame, VoiceActivityDetector};
use rtrb::RingBuffer;
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

#[test]
fn unopened_recorder_does_not_need_reopen() {
    let recorder = AudioRecorder::new().expect("recorder");
    assert!(!recorder.needs_reopen());
}

#[test]
fn stream_error_requires_reopen() {
    let recorder = AudioRecorder::new().expect("recorder");
    recorder.stream_error.store(true, Ordering::Relaxed);
    assert!(recorder.needs_reopen());
}

/// Pass-through detector with a configurable frame size, standing in for a
/// backend such as Earshot whose frames are not 30 ms.
struct FixedFrameVad(usize);

impl VoiceActivityDetector for FixedFrameVad {
    fn push_frame<'a>(&'a mut self, frame: &'a [f32]) -> anyhow::Result<VadFrame<'a>> {
        Ok(VadFrame::Speech(frame))
    }

    fn frame_samples(&self) -> usize {
        self.0
    }
}

#[test]
fn resampler_frame_size_follows_the_vad_backend() {
    let frame_samples = 256;
    let vad = VadConfig {
        detector: Arc::new(Mutex::new(Box::new(FixedFrameVad(frame_samples)))),
        frame_samples,
        offline_hangover_frames: 0,
        streaming_hangover_frames: 0,
    };
    let frame_lengths = Arc::new(Mutex::new(Vec::new()));
    let observed = Arc::clone(&frame_lengths);
    let mut processor = CaptureProcessor::new(
        16_000,
        Some(vad),
        None,
        Some(Arc::new(move |frame: &[f32]| {
            observed.lock().unwrap().push(frame.len())
        })),
        Instant::now(),
    );

    let (ready_tx, _ready_rx) = mpsc::channel();
    processor.begin_recording(VadPolicy::Offline, ready_tx);
    processor.process_raw_chunk(&[0.0; 1024], ChunkDisposition::Capture);
    let samples = processor.finish_recording();

    assert_eq!(samples.len(), 1024);
    assert_eq!(*frame_lengths.lock().unwrap(), vec![frame_samples; 4]);
}

/// Detector replaying a scripted per-frame decision:
/// `S` = speech, `N` = noise, `E` = segment end.
struct ScriptedFrameVad {
    script: std::collections::VecDeque<char>,
    frame_samples: usize,
}

impl VoiceActivityDetector for ScriptedFrameVad {
    fn push_frame<'a>(&'a mut self, frame: &'a [f32]) -> anyhow::Result<VadFrame<'a>> {
        Ok(match self.script.pop_front().unwrap_or('N') {
            'S' => VadFrame::Speech(frame),
            'E' => VadFrame::SegmentEnd,
            _ => VadFrame::Noise,
        })
    }

    fn frame_samples(&self) -> usize {
        self.frame_samples
    }
}

const SEG_FRAME: usize = 480; // 30 ms at 16 kHz

fn scripted_vad(script: &str) -> VadConfig {
    VadConfig {
        detector: Arc::new(Mutex::new(Box::new(ScriptedFrameVad {
            script: script.chars().collect(),
            frame_samples: SEG_FRAME,
        }))),
        frame_samples: SEG_FRAME,
        offline_hangover_frames: 0,
        streaming_hangover_frames: 0,
    }
}

type Segments = Arc<Mutex<Vec<Vec<f32>>>>;

fn processor_with_segments(script: &str, gate: Option<bool>) -> (CaptureProcessor, Segments) {
    let segments: Segments = Arc::new(Mutex::new(Vec::new()));
    let observed = Arc::clone(&segments);
    let sink = SegmentSink {
        callback: Arc::new(move |segment: Vec<f32>| observed.lock().unwrap().push(segment)),
        gate: gate.map(|open| Arc::new(move || open) as super::SegmentGate),
    };
    let processor = CaptureProcessor::new(
        16_000,
        Some(scripted_vad(script)),
        None,
        None,
        Instant::now(),
    )
    .with_segment_sink(Some(sink));
    (processor, segments)
}

/// Feed `frames` distinct 30 ms frames (value = frame index) and stop.
fn record_frames(processor: &mut CaptureProcessor, frames: usize) -> Vec<f32> {
    let (ready_tx, _ready_rx) = mpsc::channel();
    processor.begin_recording(VadPolicy::Offline, ready_tx);
    for i in 0..frames {
        processor.process_raw_chunk(&[i as f32; SEG_FRAME], ChunkDisposition::Capture);
    }
    processor.finish_recording()
}

#[test]
fn segment_end_hands_the_accumulated_speech_to_the_segment_callback() {
    // Two utterances separated by a boundary; the trailing 1-frame (30 ms)
    // partial segment is below MIN_FINAL_SECS and is not flushed at stop.
    let script = "NSSENSSSENS";
    let (mut processor, segments) = processor_with_segments(script, None);
    let recording = record_frames(&mut processor, script.len());

    let segments = segments.lock().unwrap();
    assert_eq!(segments.len(), 2);
    assert_eq!(
        segments[0],
        [[1.0f32; SEG_FRAME], [2.0; SEG_FRAME]].concat()
    );
    assert_eq!(
        segments[1],
        [[5.0f32; SEG_FRAME], [6.0; SEG_FRAME], [7.0; SEG_FRAME]].concat()
    );

    // The recording buffer still holds every speech frame, boundaries or not.
    let speech_frames: Vec<f32> = script
        .chars()
        .enumerate()
        .filter(|(_, c)| *c == 'S')
        .flat_map(|(i, _)| [i as f32; SEG_FRAME])
        .collect();
    assert_eq!(recording, speech_frames);
}

#[test]
fn segment_accumulation_never_changes_the_recording_buffer() {
    let script = "SSSENNSSESSSSSSSSSSSSSS";
    let (mut with_segments, _segments) = processor_with_segments(script, None);
    let mut without_segments = CaptureProcessor::new(
        16_000,
        Some(scripted_vad(script)),
        None,
        None,
        Instant::now(),
    );
    assert_eq!(
        record_frames(&mut with_segments, script.len()),
        record_frames(&mut without_segments, script.len())
    );
}

#[test]
fn long_trailing_partial_segment_is_flushed_at_stop() {
    // 12 speech frames = 360 ms >= MIN_FINAL_SECS (350 ms), no boundary.
    let script = "NSSSSSSSSSSSS";
    let (mut processor, segments) = processor_with_segments(script, None);
    record_frames(&mut processor, script.len());
    let segments = segments.lock().unwrap();
    assert_eq!(segments.len(), 1);
    assert_eq!(segments[0].len(), 12 * SEG_FRAME);
}

#[test]
fn short_trailing_partial_segment_is_not_flushed_at_stop() {
    // 11 speech frames = 330 ms < MIN_FINAL_SECS.
    let script = "NSSSSSSSSSSS";
    let (mut processor, segments) = processor_with_segments(script, None);
    record_frames(&mut processor, script.len());
    assert!(segments.lock().unwrap().is_empty());
}

#[test]
fn closed_segment_gate_disables_segments_for_the_recording() {
    let script = "SSSESSSSSSSSSSSSS";
    let (mut processor, segments) = processor_with_segments(script, Some(false));
    let recording = record_frames(&mut processor, script.len());
    assert!(segments.lock().unwrap().is_empty());
    assert_eq!(
        recording.len(),
        script.chars().filter(|c| *c == 'S').count() * SEG_FRAME
    );
}

#[test]
fn segments_do_not_leak_across_recordings() {
    // First recording ends mid-segment with a short (un-flushed) tail; the
    // next recording's first segment must not include it.
    let (mut processor, segments) = processor_with_segments("SSSSSE", None);
    let (ready_tx, _ready_rx) = mpsc::channel();
    processor.begin_recording(VadPolicy::Offline, ready_tx);
    processor.process_raw_chunk(&[9.0; SEG_FRAME], ChunkDisposition::Capture);
    processor.finish_recording();

    let (ready_tx, _ready_rx) = mpsc::channel();
    processor.begin_recording(VadPolicy::Offline, ready_tx);
    for _ in 0..5 {
        processor.process_raw_chunk(&[1.0; SEG_FRAME], ChunkDisposition::Capture);
    }
    processor.finish_recording();

    let segments = segments.lock().unwrap();
    assert_eq!(segments.len(), 1);
    assert_eq!(segments[0], vec![1.0; 4 * SEG_FRAME]);
}

#[test]
fn idle_chunks_are_discarded_without_reaching_the_recording() {
    let mut processor = CaptureProcessor::new(16_000, None, None, None, Instant::now());
    processor.process_raw_chunk(&[1.0; 480], ChunkDisposition::Discard);
    assert!(processor.finish_recording().is_empty());
}

#[test]
fn shutdown_is_processed_without_audio_samples() {
    let (_producer, consumer) = RingBuffer::<f32>::new(48_000);
    let (cmd_tx, cmd_rx) = mpsc::channel();
    let (done_tx, done_rx) = mpsc::channel();
    let worker = thread::spawn(move || {
        run_consumer(
            CaptureProcessor::new(48_000, None, None, None, Instant::now()),
            consumer,
            cmd_rx,
            Arc::new(CaptureTransportState::default()),
            Arc::new(AtomicBool::new(false)),
        );
        let _ = done_tx.send(());
    });

    cmd_tx.send(Cmd::Shutdown).expect("send shutdown");
    assert!(done_rx.recv_timeout(Duration::from_secs(1)).is_ok());
    worker.join().expect("join consumer");
}

#[test]
fn callback_writes_mono_samples() {
    let (mut producer, mut consumer) = RingBuffer::<f32>::new(8);
    let transport = CaptureTransportState::default();

    AudioRecorder::write_input_to_ring(&[0.25f32, -0.5, 1.0], 1, None, &mut producer, &transport);

    let mut output = [0.0; 3];
    consumer.pop_entire_slice(&mut output).expect("samples");
    assert_eq!(output, [0.25, -0.5, 1.0]);
}

#[test]
fn callback_downmixes_or_selects_multichannel_input() {
    let transport = CaptureTransportState::default();
    let (mut average_tx, mut average_rx) = RingBuffer::<f32>::new(4);
    AudioRecorder::write_input_to_ring(
        &[1.0f32, 3.0, -1.0, 1.0],
        2,
        None,
        &mut average_tx,
        &transport,
    );
    let mut averaged = [0.0; 2];
    average_rx
        .pop_entire_slice(&mut averaged)
        .expect("averaged samples");
    assert_eq!(averaged, [2.0, 0.0]);

    let (mut selected_tx, mut selected_rx) = RingBuffer::<f32>::new(4);
    AudioRecorder::write_input_to_ring(
        &[1.0f32, 3.0, -1.0, 1.0],
        2,
        Some(1),
        &mut selected_tx,
        &transport,
    );
    let mut selected = [0.0; 2];
    selected_rx
        .pop_entire_slice(&mut selected)
        .expect("selected samples");
    assert_eq!(selected, [3.0, 1.0]);
}

#[test]
fn callback_forwards_boundary_block_then_stays_silent_until_resumed() {
    let (mut producer, mut consumer) = RingBuffer::<f32>::new(8);
    let transport = CaptureTransportState::default();

    // The block in hand when a pause is first observed was captured before
    // the stop, so it is forwarded and only then acknowledged.
    transport.pause_requested.store(true, Ordering::Release);
    AudioRecorder::write_input_to_ring(&[1.0f32, 2.0], 1, None, &mut producer, &transport);
    assert!(transport.pause_acknowledged.load(Ordering::Acquire));
    assert_eq!(consumer.slots(), 2);

    // Later blocks while paused are dropped and are not counted as overruns.
    AudioRecorder::write_input_to_ring(&[3.0f32], 1, None, &mut producer, &transport);
    assert_eq!(consumer.slots(), 2);
    assert_eq!(transport.overrun_samples.load(Ordering::Relaxed), 0);

    // Clearing the pause, as the consumer does before stop() returns, resumes capture.
    transport.pause_acknowledged.store(false, Ordering::Relaxed);
    transport.pause_requested.store(false, Ordering::Release);
    AudioRecorder::write_input_to_ring(&[4.0f32], 1, None, &mut producer, &transport);
    let mut output = [0.0; 3];
    consumer.pop_entire_slice(&mut output).expect("samples");
    assert_eq!(output, [1.0, 2.0, 4.0]);
    assert!(!transport.pause_acknowledged.load(Ordering::Acquire));
}

#[test]
fn callback_partially_fills_ring_and_counts_dropped_audio() {
    let (mut producer, mut consumer) = RingBuffer::<f32>::new(2);
    let transport = CaptureTransportState::default();

    AudioRecorder::write_input_to_ring(&[1.0f32, 2.0, 3.0], 1, None, &mut producer, &transport);

    let mut captured = [0.0; 2];
    consumer
        .pop_entire_slice(&mut captured)
        .expect("partial callback audio");
    assert_eq!(captured, [1.0, 2.0]);
    assert_eq!(transport.overrun_samples.load(Ordering::Relaxed), 1);
}

#[test]
fn bounded_drain_leaves_remaining_samples_for_the_next_command_cycle() {
    let (mut producer, mut consumer) = RingBuffer::<f32>::new(8);
    producer
        .push_entire_slice(&[1.0, 2.0, 3.0, 4.0, 5.0])
        .expect("samples");
    let mut drained = Vec::new();

    let count =
        super::drain_available_samples(&mut consumer, 3, |part| drained.extend_from_slice(part));

    assert_eq!(count, 3);
    assert_eq!(drained, [1.0, 2.0, 3.0]);
    assert_eq!(consumer.slots(), 2);
}

#[test]
fn ring_wraparound_preserves_both_read_slices_in_order() {
    let (mut producer, mut consumer) = RingBuffer::<f32>::new(5);
    let transport = CaptureTransportState::default();
    producer
        .push_entire_slice(&[1.0, 2.0, 3.0, 4.0])
        .expect("initial samples");
    let mut discarded = [0.0; 3];
    consumer
        .pop_entire_slice(&mut discarded)
        .expect("advance ring head");

    AudioRecorder::write_input_to_ring(
        &[5.0f32, 6.0, 7.0, 8.0],
        1,
        None,
        &mut producer,
        &transport,
    );

    let chunk = consumer.read_chunk(5).expect("wrapped samples");
    let (first, second) = chunk.as_slices();
    assert!(!first.is_empty());
    assert!(!second.is_empty());
    let ordered = first
        .iter()
        .chain(second.iter())
        .copied()
        .collect::<Vec<_>>();
    assert_eq!(ordered, [4.0, 5.0, 6.0, 7.0, 8.0]);
}

#[test]
fn repeated_start_stop_cycles_resume_capture_without_leaking_samples() {
    let (mut producer, consumer) = RingBuffer::<f32>::new(16_000);
    let transport = Arc::new(CaptureTransportState::default());
    let (cmd_tx, cmd_rx) = mpsc::channel();
    let streamed = Arc::new(Mutex::new(Vec::new()));
    let streamed_cb = Arc::clone(&streamed);
    let consumer_transport = Arc::clone(&transport);
    let worker = thread::spawn(move || {
        let processor = CaptureProcessor::new(
            16_000,
            None,
            None,
            Some(Arc::new(move |frame: &[f32]| {
                streamed_cb.lock().unwrap().extend_from_slice(frame)
            })),
            Instant::now(),
        );
        run_consumer(
            processor,
            consumer,
            cmd_rx,
            consumer_transport,
            Arc::new(AtomicBool::new(false)),
        );
    });

    let wait_for_pause_request = || {
        let deadline = Instant::now() + Duration::from_secs(1);
        while !transport.pause_requested.load(Ordering::Acquire) {
            assert!(Instant::now() < deadline, "pause was not requested");
            thread::sleep(Duration::from_millis(1));
        }
    };

    let first_input = [0.25f32, -0.5, 1.0];
    let (ready_tx, ready_rx) = mpsc::channel();
    cmd_tx
        .send(Cmd::Start(VadPolicy::Disabled, Instant::now(), ready_tx))
        .expect("first start");
    AudioRecorder::write_input_to_ring(&first_input, 1, None, &mut producer, &transport);
    ready_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("first capture ready");

    let (reply_tx, reply_rx) = mpsc::channel();
    cmd_tx.send(Cmd::Stop(reply_tx)).expect("first stop");
    wait_for_pause_request();
    // The first callback after Stop carries audio captured before the stop,
    // so it belongs to the recording.
    AudioRecorder::write_input_to_ring(&[99.0f32], 1, None, &mut producer, &transport);

    let first_samples = reply_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("first stop reply");
    let first_expected = [0.25f32, -0.5, 1.0, 99.0];
    assert_eq!(&first_samples[..first_expected.len()], &first_expected);
    assert!(first_samples[first_expected.len()..]
        .iter()
        .all(|&sample| sample == 0.0));
    assert!(!transport.pause_requested.load(Ordering::Acquire));

    let first_streamed_len = {
        let streamed = streamed.lock().unwrap();
        assert_eq!(&streamed[..first_expected.len()], &first_expected);
        streamed.len()
    };

    // Start again immediately after stop() would have returned. The producer
    // must already be re-enabled, and no first-cycle samples may leak through.
    let second_input = [0.75f32, -0.25, 0.5];
    let (ready_tx, ready_rx) = mpsc::channel();
    cmd_tx
        .send(Cmd::Start(VadPolicy::Disabled, Instant::now(), ready_tx))
        .expect("second start");
    AudioRecorder::write_input_to_ring(&second_input, 1, None, &mut producer, &transport);
    ready_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("second capture ready");

    let (reply_tx, reply_rx) = mpsc::channel();
    cmd_tx.send(Cmd::Stop(reply_tx)).expect("second stop");
    wait_for_pause_request();
    AudioRecorder::write_input_to_ring(&[199.0f32], 1, None, &mut producer, &transport);

    let second_samples = reply_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("second stop reply");
    let second_expected = [0.75f32, -0.25, 0.5, 199.0];
    assert_eq!(&second_samples[..second_expected.len()], &second_expected);
    assert!(second_samples[second_expected.len()..]
        .iter()
        .all(|&sample| sample == 0.0));
    assert!(!first_samples
        .iter()
        .any(|sample| second_expected.contains(sample)));
    assert!(!second_samples
        .iter()
        .any(|sample| first_expected.contains(sample)));
    assert!(!transport.pause_requested.load(Ordering::Acquire));

    {
        let streamed = streamed.lock().unwrap();
        assert_eq!(streamed.len(), first_streamed_len + second_samples.len());
        assert_eq!(
            &streamed[first_streamed_len..first_streamed_len + second_expected.len()],
            &second_expected
        );
    }

    cmd_tx.send(Cmd::Shutdown).expect("shutdown");
    worker.join().expect("consumer worker");
}

#[test]
fn missing_callback_at_stop_marks_stream_for_rebuild_and_returns_samples() {
    let (_producer, consumer) = RingBuffer::<f32>::new(16_000);
    let transport = Arc::new(CaptureTransportState::default());
    let stream_error = Arc::new(AtomicBool::new(false));
    let observed_error = Arc::clone(&stream_error);
    let (cmd_tx, cmd_rx) = mpsc::channel();
    let worker_transport = Arc::clone(&transport);
    let worker = thread::spawn(move || {
        run_consumer(
            CaptureProcessor::new(16_000, None, None, None, Instant::now()),
            consumer,
            cmd_rx,
            worker_transport,
            stream_error,
        );
    });

    let (ready_tx, _ready_rx) = mpsc::channel();
    cmd_tx
        .send(Cmd::Start(VadPolicy::Disabled, Instant::now(), ready_tx))
        .expect("start");
    let (reply_tx, reply_rx) = mpsc::channel();
    cmd_tx.send(Cmd::Stop(reply_tx)).expect("stop");

    let samples = reply_rx
        .recv_timeout(Duration::from_secs(3))
        .expect("pause timeout still returns captured samples");
    assert!(samples.is_empty());
    worker.join().expect("consumer exits after pause timeout");
    assert!(observed_error.load(Ordering::Acquire));
}

#[test]
fn detects_access_is_denied() {
    assert!(is_microphone_access_denied("Access is denied"));
}

#[test]
fn detects_permission_denied() {
    assert!(is_microphone_access_denied("permission denied"));
}

#[test]
fn detects_windows_error_code() {
    assert!(is_microphone_access_denied("WASAPI error: 0x80070005"));
}

#[test]
fn does_not_match_unrelated_errors() {
    assert!(!is_microphone_access_denied("device not found"));
}

#[test]
fn detects_no_input_device() {
    assert!(is_no_input_device_error("No input device found"));
}

#[test]
fn detects_coreaudio_config_error() {
    assert!(is_no_input_device_error(
        "Failed to fetch preferred config: A backend-specific error has occurred: An unknown error unknown to the coreaudio-rs API occurred"
    ));
}

#[test]
fn does_not_match_other_errors_for_no_device() {
    assert!(!is_no_input_device_error("permission denied"));
    assert!(!is_no_input_device_error("device not found"));
}
