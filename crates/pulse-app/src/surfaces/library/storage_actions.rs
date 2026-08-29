use super::*;

impl LibraryView {
    pub(super) fn begin_add_storage(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.scan.is_some() {
            return;
        }
        self.rename_draft = None;
        self.text_input.reset("");
        self.modal = Some(Modal::AddStorage(AddStorageDraft {
            path: None,
            scan_now: true,
        }));
        window.focus(&self.input_focus, cx);
        cx.notify();
    }

    pub(super) fn choose_storage_folder(&mut self, cx: &mut Context<Self>) {
        if self.scan.is_some() {
            return;
        }
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Choose".into()),
        });
        cx.spawn(async move |this, cx| {
            let response = receiver.await;
            let _ = this.update(cx, |this, cx| {
                match response {
                    Ok(Ok(Some(paths))) => {
                        if let Some(path) = paths.into_iter().next() {
                            let display_name = path
                                .file_name()
                                .map(|name| name.to_string_lossy().into_owned())
                                .unwrap_or_else(|| path.display().to_string());
                            let fill_display_name = this.text_input.text().trim().is_empty();
                            if let Some(Modal::AddStorage(draft)) = &mut this.modal {
                                draft.path = Some(path);
                                this.error = None;
                            }
                            if fill_display_name {
                                this.text_input.reset(display_name);
                            }
                        }
                    }
                    Ok(Ok(None)) => {}
                    Ok(Err(error)) => this.error = Some(error.to_string()),
                    Err(error) => this.error = Some(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn confirm_add_storage(&mut self, cx: &mut Context<Self>) {
        if self.store.is_none() {
            self.error = Some(self.store_busy_message());
            cx.notify();
            return;
        }
        let Some(Modal::AddStorage(draft)) = self.modal.take() else {
            return;
        };
        let Some(path) = draft.path.as_ref() else {
            self.modal = Some(Modal::AddStorage(draft));
            cx.notify();
            return;
        };
        let display_name = self.text_input.text().trim().to_string();
        if display_name.is_empty() {
            self.modal = Some(Modal::AddStorage(draft));
            cx.notify();
            return;
        }
        let store = self.store.as_mut().expect("store availability checked");
        match store.add_storage_root(path, &display_name) {
            Ok(root) => {
                self.selected_root_id = Some(root.id);
                self.reload_or_show_error();
                if draft.scan_now {
                    self.start_scan(root.id, cx);
                }
            }
            Err(error) => {
                self.error = Some(error.to_string());
                self.modal = Some(Modal::AddStorage(draft));
            }
        }
        cx.notify();
    }

    pub(super) fn request_remove_storage(
        &mut self,
        root_id: StorageRootId,
        cx: &mut Context<Self>,
    ) {
        if self.scan.is_some() {
            return;
        }
        let Some(root) = self.roots.iter().find(|root| root.root.id == root_id) else {
            return;
        };
        self.modal = Some(Modal::RemoveStorage {
            root_id,
            display_name: root.root.display_name.clone(),
        });
        cx.notify();
    }

    pub(super) fn confirm_remove_storage(&mut self, cx: &mut Context<Self>) {
        if self.store.is_none() {
            self.error = Some(self.store_busy_message());
            cx.notify();
            return;
        }
        let Some(Modal::RemoveStorage { root_id, .. }) = self.modal.take() else {
            return;
        };
        let store = self.store.as_mut().expect("store availability checked");
        match store.remove_storage_root(root_id) {
            Ok(cover_paths) => {
                for path in cover_paths {
                    if let Err(error) = fs::remove_file(&path)
                        && error.kind() != io::ErrorKind::NotFound
                    {
                        self.error = Some(format!(
                            "Removed the storage root, but could not delete {}: {error}",
                            path.display()
                        ));
                    }
                }
                self.app_store.update(cx, |store, store_cx| {
                    store.send_command(PlaybackAction::ClearMissingMarks, store_cx);
                });
                self.reload_or_show_error();
            }
            Err(error) => self.error = Some(error.to_string()),
        }
        cx.notify();
    }

    pub(super) fn begin_rename_storage(&mut self, root_id: StorageRootId, cx: &mut Context<Self>) {
        if self.scan.is_some() {
            return;
        }
        let Some(root) = self.roots.iter().find(|root| root.root.id == root_id) else {
            return;
        };
        let display_name = root.root.display_name.clone();
        self.text_input.reset(display_name);
        self.rename_draft = Some(RenameDraft { root_id });
        cx.notify();
    }

    pub(super) fn commit_rename_storage(&mut self, cx: &mut Context<Self>) {
        if self.store.is_none() {
            self.error = Some(self.store_busy_message());
            cx.notify();
            return;
        }
        let Some(draft) = self.rename_draft.take() else {
            return;
        };
        let display_name = self.text_input.text().trim().to_string();
        let store = self.store.as_mut().expect("store availability checked");
        if let Err(error) = store.rename_storage_root(draft.root_id, &display_name) {
            self.error = Some(error.to_string());
            self.rename_draft = Some(draft);
        } else {
            self.reload_or_show_error();
        }
        cx.notify();
    }

    pub(super) fn start_scan(&mut self, root_id: StorageRootId, cx: &mut Context<Self>) {
        if self.scan.is_some() {
            return;
        }
        // Defense in depth: a worker (album delete) may own the store even
        // with no scan active.
        let Some(mut store) = self.store.take() else {
            self.error = Some(self.store_busy_message());
            cx.notify();
            return;
        };
        let sender = self.worker_tx.clone();
        let cover_cache_directory = self.cover_cache_directory.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = Arc::clone(&cancel);
        self.scan = Some(ActiveScan {
            root_id,
            progress: None,
            cancel,
        });
        thread::Builder::new()
            .name("pulse-library-scan".to_string())
            .spawn(move || {
                let progress_sender = sender.clone();
                let outcome = catch_unwind(AssertUnwindSafe(|| {
                    let result = scan_storage_root_cancellable(
                        &mut store,
                        root_id,
                        cover_cache_directory,
                        move |progress| {
                            let _ = progress_sender
                                .send(WorkerEvent::ScanProgress { root_id, progress });
                        },
                        || worker_cancel.load(Ordering::Acquire),
                    )
                    .map(|report| match report {
                        Some(report) => ScanCompletion::Completed {
                            outcome: report.outcome,
                            removals_suppressed: report.removals_suppressed,
                        },
                        None => ScanCompletion::Cancelled,
                    })
                    .map_err(|error| error.to_string());
                    (store, result)
                }));
                let _ = match outcome {
                    Ok((store, result)) => sender.send(WorkerEvent::ScanFinished {
                        root_id,
                        store,
                        result,
                    }),
                    // The store was consumed by the unwind; the UI reopens it.
                    Err(_) => sender.send(WorkerEvent::ScanPanicked { root_id }),
                };
            })
            .expect("failed to spawn library scan worker");
        cx.notify();
    }

    pub(super) fn cancel_scan(&mut self, cx: &mut Context<Self>) {
        if let Some(scan) = &self.scan {
            scan.cancel.store(true, Ordering::Release);
        }
        cx.notify();
    }
}
