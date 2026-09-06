use super::*;

#[test]
fn dropping_controller_stops_active_playback_with_sender_clone_alive() {
    let (controller, log) = fake_controller();
    let events = controller.subscribe();
    let commands = controller.command_sender();
    commands
        .send(PlaybackCommand::PlayFile {
            path: PathBuf::from("track.flac"),
        })
        .unwrap();
    wait_for(&events, |event| {
        matches!(
            event,
            PlaybackEvent::Position {
                position_ms: 1_000,
                ..
            }
        )
    });

    drop(controller);
    assert!(commands.send(PlaybackCommand::Stop).is_err());
    loop {
        match events.recv_timeout(Duration::from_secs(1)) {
            Ok(_) => {}
            Err(RecvTimeoutError::Disconnected) => break,
            Err(RecvTimeoutError::Timeout) => {
                panic!("playback worker did not shut down after command disconnect")
            }
        }
    }

    assert_eq!(log.lock().unwrap().stops, 1);
    assert_eq!(log.lock().unwrap().releases, 1);
}

#[test]
fn dropping_controller_releases_a_backend_retained_by_pause() {
    let (controller, log) = fake_controller_with_kind(EngineKind::Integer);
    let events = controller.subscribe();
    let commands = controller.command_sender();
    commands
        .send(PlaybackCommand::PlayFile {
            path: PathBuf::from("track.flac"),
        })
        .unwrap();
    wait_for(&events, |event| {
        matches!(
            event,
            PlaybackEvent::Position {
                position_ms: 1_000,
                ..
            }
        )
    });
    commands.send(PlaybackCommand::Pause).unwrap();
    wait_for(&events, |event| {
        *event == PlaybackEvent::StateChanged(PlaybackState::Paused)
    });

    drop(controller);
    while let Ok(_event) = events.recv_timeout(Duration::from_secs(1)) {}

    let log = log.lock().unwrap();
    assert_eq!(log.stops, 1);
    assert_eq!(log.releases, 1);
}

#[test]
fn explicit_shutdown_releases_a_paused_integer_backend_once() {
    let (mut controller, log) = fake_controller_with_kind(EngineKind::Integer);
    let events = controller.subscribe();
    let commands = controller.command_sender();
    commands
        .send(PlaybackCommand::PlayFile {
            path: PathBuf::from("track.flac"),
        })
        .unwrap();
    wait_for(&events, |event| {
        matches!(
            event,
            PlaybackEvent::Position {
                position_ms: 1_000,
                ..
            }
        )
    });
    commands.send(PlaybackCommand::Pause).unwrap();
    wait_for(&events, |event| {
        *event == PlaybackEvent::StateChanged(PlaybackState::Paused)
    });

    controller.shutdown().unwrap();
    assert_eq!(log.lock().unwrap().releases, 1);

    controller.shutdown().unwrap();
    assert_eq!(log.lock().unwrap().releases, 1);
}

#[test]
fn rejected_backend_release_error_does_not_block_opening_the_selected_device() {
    let log = Arc::new(Mutex::new(FakeLog {
        release_error: true,
        ..FakeLog::default()
    }));
    let old_kind = EngineKind::Universal {
        exclusive_mode: true,
    };
    let new_kind = EngineKind::Universal {
        exclusive_mode: false,
    };
    let old_backend = Box::new(FakeBackend {
        log: Arc::clone(&log),
        device_id: 7,
        engine_kind: old_kind,
        exclusive_mode: true,
        retains_device: false,
        fed_frames: 0,
        format: None,
        volume: (1.0, false),
        hardware_volume: None,
        hardware_volume_active: false,
        hardware_volume_event_pending: false,
    });
    let factory_log = Arc::clone(&log);
    let backend_factory: BackendFactory = Arc::new(move |device_id, engine_kind| {
        factory_log.lock().unwrap().opened_devices.push(device_id);
        Ok(Box::new(FakeBackend {
            log: Arc::clone(&factory_log),
            device_id,
            engine_kind,
            exclusive_mode: false,
            retains_device: false,
            fed_frames: 0,
            format: None,
            volume: (1.0, false),
            hardware_volume: None,
            hardware_volume_active: false,
            hardware_volume_event_pending: false,
        }))
    });
    let decoder_factory: DecoderFactory = Arc::new(|_| unreachable!());
    let (_command_tx, command_rx) = mpsc::channel();
    let (event_tx, event_rx) = mpsc::channel();
    let subscribers = Arc::new(Mutex::new(vec![event_tx]));
    let mut worker = Worker::new(
        WorkerSettings {
            output_device: 8,
            engine_kind: new_kind,
            output_stall_timeout: Duration::MAX,
            now: Box::new(Instant::now),
        },
        command_rx,
        subscribers,
        backend_factory,
        decoder_factory,
        Arc::new(AtomicBool::new(false)),
        ActiveBackendRelease::default(),
    );
    worker.backend = Some((7, old_kind, old_backend));

    let replacement = worker.take_or_open_backend(new_kind).unwrap();

    assert_eq!(replacement.volume_domain(), VolumeDomain::Software);
    assert!(matches!(
        event_rx.recv_timeout(Duration::from_secs(1)).unwrap(),
        PlaybackEvent::Error { message, .. } if message.contains("backend release failed")
    ));
    let log = log.lock().unwrap();
    assert_eq!(log.releases, 1);
    assert_eq!(log.opened_devices, [8]);
}

struct RecordingRelease {
    calls: Arc<std::sync::atomic::AtomicUsize>,
    released: AtomicBool,
    observed: Sender<()>,
    delay: Duration,
}

impl BackendRelease for RecordingRelease {
    fn release_before(&self, _deadline: Instant) -> Result<(), EngineError> {
        if !self.released.swap(true, Ordering::AcqRel) {
            self.calls.fetch_add(1, Ordering::Relaxed);
            thread::sleep(self.delay);
            self.observed.send(()).unwrap();
        }
        Ok(())
    }
}

#[test]
fn shutdown_completes_release_before_giving_the_worker_a_fresh_join_window() {
    let (command_tx, _command_rx) = mpsc::channel();
    let subscribers = Arc::new(Mutex::new(Vec::new()));
    let shutdown = Arc::new(AtomicBool::new(false));
    let (unblock_tx, unblock_rx) = mpsc::channel();
    let (done_tx, done_rx) = mpsc::channel();
    let worker = thread::spawn(move || {
        unblock_rx.recv().unwrap();
        done_tx.send(()).unwrap();
    });
    let release_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let (release_observed_tx, release_observed_rx) = mpsc::channel();
    let backend_release = ActiveBackendRelease::default();
    backend_release.replace(Some(Arc::new(RecordingRelease {
        calls: Arc::clone(&release_calls),
        released: AtomicBool::new(false),
        observed: release_observed_tx,
        delay: Duration::from_millis(100),
    })));
    let mut controller = PlaybackController {
        command_tx,
        subscribers,
        shutdown,
        worker: Some(worker),
        backend_release,
    };
    let (shutdown_result_tx, shutdown_result_rx) = mpsc::channel();
    let shutdown_thread = thread::spawn(move || {
        let started = Instant::now();
        let result = controller
            .shutdown_with_timeouts(Duration::from_millis(20), Duration::from_millis(250));
        shutdown_result_tx
            .send((started.elapsed(), result))
            .unwrap();
    });

    release_observed_rx
        .recv_timeout(Duration::from_millis(250))
        .unwrap();
    assert!(matches!(
        shutdown_result_rx.try_recv(),
        Err(TryRecvError::Empty)
    ));
    let (elapsed, error) = shutdown_result_rx
        .recv_timeout(Duration::from_secs(1))
        .unwrap();
    let error = error.unwrap_err();

    assert!(elapsed >= Duration::from_millis(300));
    assert!(elapsed < Duration::from_millis(750));
    assert_eq!(release_calls.load(Ordering::Relaxed), 1);
    assert!(matches!(
        error,
        EngineError::BackendRelease(message)
            if message.contains("playback worker did not exit within 250 ms after the device release attempt")
                && message.contains("device release completed on the shutdown thread")
    ));

    unblock_tx.send(()).unwrap();
    done_rx.recv_timeout(Duration::from_secs(1)).unwrap();
    shutdown_thread.join().unwrap();
}

#[test]
fn shutdown_bounds_release_lock_contention_and_an_unresponsive_worker() {
    let (command_tx, _command_rx) = mpsc::channel();
    let subscribers = Arc::new(Mutex::new(Vec::new()));
    let shutdown = Arc::new(AtomicBool::new(false));
    let (unblock_tx, unblock_rx) = mpsc::channel();
    let (done_tx, done_rx) = mpsc::channel();
    let (locked_tx, locked_rx) = mpsc::channel();
    let release_handle = crate::integer_engine::IntegerReleaseHandle::empty_for_test();
    let worker_release_handle = release_handle.clone();
    let worker = thread::spawn(move || {
        worker_release_handle.hold_resources_for_test(locked_tx, unblock_rx);
        done_tx.send(()).unwrap();
    });
    locked_rx.recv_timeout(Duration::from_secs(1)).unwrap();
    let backend_release = ActiveBackendRelease::default();
    backend_release.replace(Some(Arc::new(release_handle)));
    let mut controller = PlaybackController {
        command_tx,
        subscribers,
        shutdown,
        worker: Some(worker),
        backend_release,
    };

    let started = Instant::now();
    let error = controller
        .shutdown_with_timeouts(Duration::from_millis(20), Duration::from_millis(20))
        .unwrap_err();

    assert!(started.elapsed() < Duration::from_millis(250));
    assert!(matches!(
        error,
        EngineError::BackendRelease(message)
            if message.contains("timed out waiting for the integer device release lock")
                && message.contains("playback worker did not exit within 20 ms after the device release attempt")
                && message.contains("device release did not complete on the shutdown thread")
    ));
    controller.shutdown().unwrap();

    unblock_tx.send(()).unwrap();
    done_rx.recv_timeout(Duration::from_secs(1)).unwrap();
}

#[test]
fn a_panicking_worker_disconnects_event_subscribers() {
    let log = Arc::new(Mutex::new(FakeLog::default()));
    let backend_log = Arc::clone(&log);
    let controller = PlaybackController::spawn_with_dependencies(
        7,
        EngineKind::Universal {
            exclusive_mode: true,
        },
        Arc::new(move |device_id, engine_kind| {
            let EngineKind::Universal { exclusive_mode } = engine_kind else {
                panic!("test factory expected universal engine")
            };
            Ok(Box::new(FakeBackend {
                log: Arc::clone(&backend_log),
                device_id,
                engine_kind,
                exclusive_mode,
                retains_device: false,
                fed_frames: 0,
                format: None,
                volume: (f32::NAN, false),
                hardware_volume: None,
                hardware_volume_active: false,
                hardware_volume_event_pending: false,
            }))
        }),
        Arc::new(|_| panic!("decoder factory exploded")),
        Duration::MAX,
        Box::new(Instant::now),
    );
    let events = controller.subscribe();
    let commands = controller.command_sender();
    commands
        .send(PlaybackCommand::PlayFile {
            path: PathBuf::from("track.flac"),
        })
        .unwrap();

    loop {
        match events.recv_timeout(Duration::from_secs(2)) {
            Ok(_) => {}
            Err(RecvTimeoutError::Disconnected) => break,
            Err(RecvTimeoutError::Timeout) => {
                panic!("subscribers were not disconnected after a worker panic")
            }
        }
    }
}
