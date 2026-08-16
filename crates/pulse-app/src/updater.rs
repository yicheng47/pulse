#[cfg(all(target_os = "macos", feature = "updater"))]
mod sparkle {
    use objc2::{
        MainThreadMarker, MainThreadOnly, extern_class, extern_methods,
        rc::{Allocated, Retained},
        runtime::{AnyObject, NSObject},
    };

    use crate::preferences;

    #[link(name = "Sparkle", kind = "framework")]
    unsafe extern "C" {}

    extern_class!(
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        struct SPUStandardUpdaterController;
    );

    extern_class!(
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        struct SPUUpdater;
    );

    impl SPUStandardUpdaterController {
        extern_methods!(
            #[unsafe(method(initWithStartingUpdater:updaterDelegate:userDriverDelegate:))]
            #[unsafe(method_family = init)]
            fn init_with_starting_updater(
                this: Allocated<Self>,
                start_updater: bool,
                updater_delegate: Option<&AnyObject>,
                user_driver_delegate: Option<&AnyObject>,
            ) -> Retained<Self>;

            #[unsafe(method(updater))]
            #[unsafe(method_family = none)]
            fn updater(&self) -> Retained<SPUUpdater>;

            #[unsafe(method(startUpdater))]
            #[unsafe(method_family = none)]
            fn start_updater(&self);

            #[unsafe(method(checkForUpdates:))]
            #[unsafe(method_family = none)]
            fn check_for_updates(&self, sender: Option<&AnyObject>);
        );
    }

    impl SPUUpdater {
        extern_methods!(
            #[unsafe(method(automaticallyChecksForUpdates))]
            #[unsafe(method_family = none)]
            fn automatically_checks_for_updates(&self) -> bool;

            #[unsafe(method(setAutomaticallyChecksForUpdates:))]
            #[unsafe(method_family = none)]
            fn set_automatically_checks_for_updates(&self, enabled: bool);
        );
    }

    pub(crate) struct Updater {
        controller: Retained<SPUStandardUpdaterController>,
    }

    impl Updater {
        pub(crate) fn new() -> Self {
            let main_thread = MainThreadMarker::new().expect("Pulse must start on the main thread");
            let controller = SPUStandardUpdaterController::init_with_starting_updater(
                SPUStandardUpdaterController::alloc(main_thread),
                false,
                None,
                None,
            );
            match preferences::take_legacy_update_check_preference() {
                Ok(Some(enabled)) => controller
                    .updater()
                    .set_automatically_checks_for_updates(enabled),
                Ok(None) => {}
                Err(error) => {
                    eprintln!("Could not migrate the launch update-check preference: {error}")
                }
            }
            controller.start_updater();
            Self { controller }
        }

        pub(crate) fn is_available(&self) -> bool {
            true
        }

        pub(crate) fn check_for_updates(&self) {
            self.controller.check_for_updates(None);
        }

        pub(crate) fn automatically_checks_for_updates(&self) -> bool {
            self.controller.updater().automatically_checks_for_updates()
        }

        pub(crate) fn set_automatically_checks_for_updates(&self, enabled: bool) {
            self.controller
                .updater()
                .set_automatically_checks_for_updates(enabled);
        }
    }
}

#[cfg(not(all(target_os = "macos", feature = "updater")))]
mod sparkle {
    pub(crate) struct Updater;

    impl Updater {
        pub(crate) fn new() -> Self {
            Self
        }

        pub(crate) fn is_available(&self) -> bool {
            false
        }

        pub(crate) fn check_for_updates(&self) {}

        pub(crate) fn automatically_checks_for_updates(&self) -> bool {
            false
        }

        pub(crate) fn set_automatically_checks_for_updates(&self, _enabled: bool) {}
    }
}

pub(crate) use sparkle::Updater;
